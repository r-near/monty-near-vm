// monty-near-vm: The Monty Python VM deployed as a NEAR smart contract.
//
// Anyone can call `execute` with arbitrary Python code (the subset Monty
// supports) and it will be parsed, compiled, and executed on-chain. The Python
// code has access to the full NEAR host API as builtin functions. `print()`
// output is wired to NEAR's log_utf8.
//
// This embeds the ruff parser and Monty compiler in the WASM binary.

use std::borrow::Cow;

use monty::{
    ExtFunctionResult, MontyException, MontyObject, MontyRun, NameLookupResult,
    NoLimitTracker, PrintWriter, PrintWriterCallback, RunProgress,
};
use near_sys::*;

// ---------------------------------------------------------------------------
// Custom getrandom backend — uses NEAR's VRF randomness
// ---------------------------------------------------------------------------

#[no_mangle]
unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error> {
    unsafe {
        random_seed(0);
        let seed_len = register_len(0) as usize;
        let mut seed = [0u8; 32];
        read_register(0, seed.as_mut_ptr() as u64);

        let dest_slice = core::slice::from_raw_parts_mut(dest, len);
        for (i, byte) in dest_slice.iter_mut().enumerate() {
            *byte = seed[i % seed_len];
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// PrintWriter callback — wires Python print() to NEAR log_utf8
// ---------------------------------------------------------------------------

struct NearPrint {
    buffer: String,
}

impl NearPrint {
    fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }
}

impl PrintWriterCallback for NearPrint {
    fn stdout_write(&mut self, output: Cow<'_, str>) -> Result<(), MontyException> {
        self.buffer.push_str(&output);
        Ok(())
    }

    fn stdout_push(&mut self, end: char) -> Result<(), MontyException> {
        if end == '\n' {
            // Flush the line to NEAR log
            near_log(&self.buffer);
            self.buffer.clear();
        } else {
            self.buffer.push(end);
        }
        Ok(())
    }
}

impl Drop for NearPrint {
    fn drop(&mut self) {
        // Flush any remaining buffered output
        if !self.buffer.is_empty() {
            near_log(&self.buffer);
        }
    }
}

// ---------------------------------------------------------------------------
// NEAR host function wrappers
// ---------------------------------------------------------------------------

fn near_input() -> Vec<u8> {
    unsafe {
        input(0);
        let len = register_len(0);
        if len == u64::MAX {
            return Vec::new();
        }
        let mut buf = vec![0u8; len as usize];
        read_register(0, buf.as_mut_ptr() as u64);
        buf
    }
}

fn near_value_return(data: &[u8]) {
    unsafe {
        value_return(data.len() as u64, data.as_ptr() as u64);
    }
}

fn near_log(msg: &str) {
    unsafe {
        log_utf8(msg.len() as u64, msg.as_ptr() as u64);
    }
}

fn near_read_register_bytes(register_id: u64) -> Vec<u8> {
    unsafe {
        let len = register_len(register_id);
        let mut buf = vec![0u8; len as usize];
        read_register(register_id, buf.as_mut_ptr() as u64);
        buf
    }
}

fn near_read_register_string(register_id: u64) -> String {
    String::from_utf8(near_read_register_bytes(register_id)).unwrap_or_default()
}

fn near_storage_write(key: &[u8], value: &[u8]) -> bool {
    unsafe {
        storage_write(
            key.len() as u64,
            key.as_ptr() as u64,
            value.len() as u64,
            value.as_ptr() as u64,
            0,
        ) == 1
    }
}

fn near_storage_read(key: &[u8]) -> Option<Vec<u8>> {
    unsafe {
        if storage_read(key.len() as u64, key.as_ptr() as u64, 0) == 1 {
            Some(near_read_register_bytes(0))
        } else {
            None
        }
    }
}

fn near_storage_remove(key: &[u8]) -> bool {
    unsafe { storage_remove(key.len() as u64, key.as_ptr() as u64, 0) == 1 }
}

fn near_storage_has_key(key: &[u8]) -> bool {
    unsafe { storage_has_key(key.len() as u64, key.as_ptr() as u64) == 1 }
}

fn near_current_account_id() -> String {
    unsafe { current_account_id(0); }
    near_read_register_string(0)
}

fn near_predecessor_account_id() -> String {
    unsafe { predecessor_account_id(0); }
    near_read_register_string(0)
}

fn near_signer_account_id() -> String {
    unsafe { signer_account_id(0); }
    near_read_register_string(0)
}

fn near_block_height() -> u64 {
    unsafe { block_index() }
}

fn near_block_timestamp() -> u64 {
    unsafe { block_timestamp() }
}

fn near_sha256(data: &[u8]) -> Vec<u8> {
    unsafe { sha256(data.len() as u64, data.as_ptr() as u64, 0); }
    near_read_register_bytes(0)
}

fn near_keccak256(data: &[u8]) -> Vec<u8> {
    unsafe { keccak256(data.len() as u64, data.as_ptr() as u64, 0); }
    near_read_register_bytes(0)
}

fn to_hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use core::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

fn from_hex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .filter_map(|i| {
            hex.get(i..i + 2)
                .and_then(|s| u8::from_str_radix(s, 16).ok())
        })
        .collect()
}

fn near_signer_account_pk() -> Vec<u8> {
    unsafe { signer_account_pk(0); }
    near_read_register_bytes(0)
}

fn near_epoch_height() -> u64 {
    unsafe { epoch_height() }
}

fn near_storage_usage() -> u64 {
    unsafe { storage_usage() }
}

fn near_account_balance() -> u128 {
    let mut buf = [0u8; 16];
    unsafe { account_balance(buf.as_mut_ptr() as u64) };
    u128::from_le_bytes(buf)
}

fn near_account_locked_balance() -> u128 {
    let mut buf = [0u8; 16];
    unsafe { account_locked_balance(buf.as_mut_ptr() as u64) };
    u128::from_le_bytes(buf)
}

fn near_attached_deposit() -> u128 {
    let mut buf = [0u8; 16];
    unsafe { attached_deposit(buf.as_mut_ptr() as u64) };
    u128::from_le_bytes(buf)
}

fn near_prepaid_gas() -> u64 {
    unsafe { prepaid_gas() }
}

fn near_used_gas() -> u64 {
    unsafe { used_gas() }
}

fn near_random_seed() -> Vec<u8> {
    unsafe { random_seed(0); }
    near_read_register_bytes(0)
}

fn near_keccak512(data: &[u8]) -> Vec<u8> {
    unsafe { keccak512(data.len() as u64, data.as_ptr() as u64, 0); }
    near_read_register_bytes(0)
}

fn near_ripemd160(data: &[u8]) -> Vec<u8> {
    unsafe { ripemd160(data.len() as u64, data.as_ptr() as u64, 0); }
    near_read_register_bytes(0)
}

fn near_ecrecover(hash: &[u8], sig: &[u8], v: u64, malleability_flag: u64) -> Option<Vec<u8>> {
    unsafe {
        let result = ecrecover(
            hash.len() as u64, hash.as_ptr() as u64,
            sig.len() as u64, sig.as_ptr() as u64,
            v, malleability_flag, 0,
        );
        if result == 0 { None } else { Some(near_read_register_bytes(0)) }
    }
}

fn near_ed25519_verify(sig: &[u8], msg: &[u8], pub_key: &[u8]) -> bool {
    unsafe {
        ed25519_verify(
            sig.len() as u64, sig.as_ptr() as u64,
            msg.len() as u64, msg.as_ptr() as u64,
            pub_key.len() as u64, pub_key.as_ptr() as u64,
        ) == 1
    }
}

fn near_promise_create(account_id: &str, function_name: &str, arguments: &[u8], amount: u128, gas: u64) -> u64 {
    let amount_bytes = amount.to_le_bytes();
    unsafe {
        promise_create(
            account_id.len() as u64, account_id.as_ptr() as u64,
            function_name.len() as u64, function_name.as_ptr() as u64,
            arguments.len() as u64, arguments.as_ptr() as u64,
            amount_bytes.as_ptr() as u64, gas,
        )
    }
}

fn near_promise_then(promise_index: u64, account_id: &str, function_name: &str, arguments: &[u8], amount: u128, gas: u64) -> u64 {
    let amount_bytes = amount.to_le_bytes();
    unsafe {
        promise_then(
            promise_index,
            account_id.len() as u64, account_id.as_ptr() as u64,
            function_name.len() as u64, function_name.as_ptr() as u64,
            arguments.len() as u64, arguments.as_ptr() as u64,
            amount_bytes.as_ptr() as u64, gas,
        )
    }
}

fn near_promise_and(promise_indices: &[u64]) -> u64 {
    unsafe { promise_and(promise_indices.as_ptr() as u64, promise_indices.len() as u64) }
}

fn near_promise_batch_create(account_id: &str) -> u64 {
    unsafe { promise_batch_create(account_id.len() as u64, account_id.as_ptr() as u64) }
}

fn near_promise_batch_then(promise_index: u64, account_id: &str) -> u64 {
    unsafe { promise_batch_then(promise_index, account_id.len() as u64, account_id.as_ptr() as u64) }
}

fn near_promise_results_count() -> u64 {
    unsafe { promise_results_count() }
}

fn near_promise_result(result_idx: u64) -> (u64, Vec<u8>) {
    unsafe {
        let status = promise_result(result_idx, 0);
        if status == 1 { (status, near_read_register_bytes(0)) } else { (status, Vec::new()) }
    }
}

fn near_promise_return(promise_id: u64) {
    unsafe { promise_return(promise_id) }
}

fn near_promise_batch_action_create_account(promise_index: u64) {
    unsafe { promise_batch_action_create_account(promise_index) }
}

fn near_promise_batch_action_deploy_contract(promise_index: u64, code: &[u8]) {
    unsafe { promise_batch_action_deploy_contract(promise_index, code.len() as u64, code.as_ptr() as u64) }
}

fn near_promise_batch_action_function_call(promise_index: u64, function_name: &str, arguments: &[u8], amount: u128, gas: u64) {
    let amount_bytes = amount.to_le_bytes();
    unsafe {
        promise_batch_action_function_call(
            promise_index,
            function_name.len() as u64, function_name.as_ptr() as u64,
            arguments.len() as u64, arguments.as_ptr() as u64,
            amount_bytes.as_ptr() as u64, gas,
        )
    }
}

fn near_promise_batch_action_function_call_weight(promise_index: u64, function_name: &str, arguments: &[u8], amount: u128, gas: u64, weight: u64) {
    let amount_bytes = amount.to_le_bytes();
    unsafe {
        promise_batch_action_function_call_weight(
            promise_index,
            function_name.len() as u64, function_name.as_ptr() as u64,
            arguments.len() as u64, arguments.as_ptr() as u64,
            amount_bytes.as_ptr() as u64, gas, weight,
        )
    }
}

fn near_promise_batch_action_transfer(promise_index: u64, amount: u128) {
    let amount_bytes = amount.to_le_bytes();
    unsafe { promise_batch_action_transfer(promise_index, amount_bytes.as_ptr() as u64) }
}

fn near_promise_batch_action_stake(promise_index: u64, amount: u128, public_key: &[u8]) {
    let amount_bytes = amount.to_le_bytes();
    unsafe {
        promise_batch_action_stake(promise_index, amount_bytes.as_ptr() as u64, public_key.len() as u64, public_key.as_ptr() as u64)
    }
}

fn near_promise_batch_action_add_key_with_full_access(promise_index: u64, public_key: &[u8], nonce: u64) {
    unsafe { promise_batch_action_add_key_with_full_access(promise_index, public_key.len() as u64, public_key.as_ptr() as u64, nonce) }
}

fn near_promise_batch_action_add_key_with_function_call(promise_index: u64, public_key: &[u8], nonce: u64, allowance: u128, receiver_id: &str, function_names: &str) {
    let allowance_bytes = allowance.to_le_bytes();
    unsafe {
        promise_batch_action_add_key_with_function_call(
            promise_index,
            public_key.len() as u64, public_key.as_ptr() as u64,
            nonce, allowance_bytes.as_ptr() as u64,
            receiver_id.len() as u64, receiver_id.as_ptr() as u64,
            function_names.len() as u64, function_names.as_ptr() as u64,
        )
    }
}

fn near_promise_batch_action_delete_key(promise_index: u64, public_key: &[u8]) {
    unsafe { promise_batch_action_delete_key(promise_index, public_key.len() as u64, public_key.as_ptr() as u64) }
}

fn near_promise_batch_action_delete_account(promise_index: u64, beneficiary_id: &str) {
    unsafe { promise_batch_action_delete_account(promise_index, beneficiary_id.len() as u64, beneficiary_id.as_ptr() as u64) }
}

fn near_validator_stake(account_id: &str) -> u128 {
    let mut buf = [0u8; 16];
    unsafe { validator_stake(account_id.len() as u64, account_id.as_ptr() as u64, buf.as_mut_ptr() as u64) };
    u128::from_le_bytes(buf)
}

fn near_validator_total_stake() -> u128 {
    let mut buf = [0u8; 16];
    unsafe { validator_total_stake(buf.as_mut_ptr() as u64) };
    u128::from_le_bytes(buf)
}

fn near_alt_bn128_g1_multiexp(data: &[u8]) -> Vec<u8> {
    unsafe { alt_bn128_g1_multiexp(data.len() as u64, data.as_ptr() as u64, 0) };
    near_read_register_bytes(0)
}

fn near_alt_bn128_g1_sum(data: &[u8]) -> Vec<u8> {
    unsafe { alt_bn128_g1_sum(data.len() as u64, data.as_ptr() as u64, 0) };
    near_read_register_bytes(0)
}

fn near_alt_bn128_pairing_check(data: &[u8]) -> bool {
    unsafe { alt_bn128_pairing_check(data.len() as u64, data.as_ptr() as u64) == 1 }
}

fn near_bls12381_p1_sum(data: &[u8]) -> Option<Vec<u8>> {
    unsafe { if bls12381_p1_sum(data.len() as u64, data.as_ptr() as u64, 0) == 0 { None } else { Some(near_read_register_bytes(0)) } }
}

fn near_bls12381_p2_sum(data: &[u8]) -> Option<Vec<u8>> {
    unsafe { if bls12381_p2_sum(data.len() as u64, data.as_ptr() as u64, 0) == 0 { None } else { Some(near_read_register_bytes(0)) } }
}

fn near_bls12381_g1_multiexp(data: &[u8]) -> Option<Vec<u8>> {
    unsafe { if bls12381_g1_multiexp(data.len() as u64, data.as_ptr() as u64, 0) == 0 { None } else { Some(near_read_register_bytes(0)) } }
}

fn near_bls12381_g2_multiexp(data: &[u8]) -> Option<Vec<u8>> {
    unsafe { if bls12381_g2_multiexp(data.len() as u64, data.as_ptr() as u64, 0) == 0 { None } else { Some(near_read_register_bytes(0)) } }
}

fn near_bls12381_map_fp_to_g1(data: &[u8]) -> Option<Vec<u8>> {
    unsafe { if bls12381_map_fp_to_g1(data.len() as u64, data.as_ptr() as u64, 0) == 0 { None } else { Some(near_read_register_bytes(0)) } }
}

fn near_bls12381_map_fp2_to_g2(data: &[u8]) -> Option<Vec<u8>> {
    unsafe { if bls12381_map_fp2_to_g2(data.len() as u64, data.as_ptr() as u64, 0) == 0 { None } else { Some(near_read_register_bytes(0)) } }
}

fn near_bls12381_pairing_check(data: &[u8]) -> bool {
    unsafe { bls12381_pairing_check(data.len() as u64, data.as_ptr() as u64) == 1 }
}

fn near_bls12381_p1_decompress(data: &[u8]) -> Option<Vec<u8>> {
    unsafe { if bls12381_p1_decompress(data.len() as u64, data.as_ptr() as u64, 0) == 0 { None } else { Some(near_read_register_bytes(0)) } }
}

fn near_bls12381_p2_decompress(data: &[u8]) -> Option<Vec<u8>> {
    unsafe { if bls12381_p2_decompress(data.len() as u64, data.as_ptr() as u64, 0) == 0 { None } else { Some(near_read_register_bytes(0)) } }
}

// ---------------------------------------------------------------------------
// Known NEAR function names — resolved via NameLookup
// ---------------------------------------------------------------------------

/// Check if a name is a known NEAR host function and return a sentinel.
/// The actual dispatch happens on FunctionCall. We just need to resolve
/// the name so Monty doesn't raise NameError.
fn is_near_function(name: &str) -> bool {
    matches!(name,
        "value_return" | "input" | "log"
        | "storage_write" | "storage_read" | "storage_remove" | "storage_has_key"
        | "current_account_id" | "predecessor_account_id" | "signer_account_id"
        | "block_height" | "block_timestamp"
        | "sha256" | "keccak256"
        | "signer_account_pk" | "epoch_height" | "storage_usage"
        | "account_balance" | "account_locked_balance" | "attached_deposit"
        | "prepaid_gas" | "used_gas"
        | "random_seed" | "keccak512" | "ripemd160" | "ecrecover" | "ed25519_verify"
        | "promise_create" | "promise_then" | "promise_and"
        | "promise_batch_create" | "promise_batch_then"
        | "promise_results_count" | "promise_result" | "promise_return"
        | "promise_batch_action_create_account" | "promise_batch_action_deploy_contract"
        | "promise_batch_action_function_call" | "promise_batch_action_function_call_weight"
        | "promise_batch_action_transfer" | "promise_batch_action_stake"
        | "promise_batch_action_add_key_with_full_access"
        | "promise_batch_action_add_key_with_function_call"
        | "promise_batch_action_delete_key" | "promise_batch_action_delete_account"
        | "validator_stake" | "validator_total_stake"
        | "alt_bn128_g1_multiexp" | "alt_bn128_g1_sum" | "alt_bn128_pairing_check"
        | "bls12381_p1_sum" | "bls12381_p2_sum"
        | "bls12381_g1_multiexp" | "bls12381_g2_multiexp"
        | "bls12381_map_fp_to_g1" | "bls12381_map_fp2_to_g2"
        | "bls12381_pairing_check" | "bls12381_p1_decompress" | "bls12381_p2_decompress"
    )
}

// ---------------------------------------------------------------------------
// Dispatch table — routes Python external function calls to NEAR host functions
// ---------------------------------------------------------------------------

fn dispatch_function(name: &str, args: &[MontyObject]) -> MontyObject {
    let arg_str = |idx: usize| -> Option<&str> {
        match args.get(idx) {
            Some(MontyObject::String(s)) => Some(s.as_str()),
            _ => None,
        }
    };
    let arg_int = |idx: usize| -> Option<i64> {
        match args.get(idx) {
            Some(MontyObject::Int(n)) => Some(*n),
            _ => None,
        }
    };
    let arg_bytes = |idx: usize| -> Option<&[u8]> {
        match args.get(idx) {
            Some(MontyObject::String(s)) => Some(s.as_bytes()),
            Some(MontyObject::Bytes(b)) => Some(b.as_slice()),
            _ => None,
        }
    };
    let arg_u128 = |idx: usize| -> Option<u128> {
        match args.get(idx) {
            Some(MontyObject::String(s)) => s.parse::<u128>().ok(),
            Some(MontyObject::Int(n)) => Some(*n as u128),
            _ => None,
        }
    };

    match name {
        "value_return" => {
            let s = match args.first() {
                Some(MontyObject::String(s)) => s.as_bytes(),
                Some(MontyObject::Bytes(b)) => b.as_slice(),
                Some(other) => {
                    let repr = format!("{other:?}");
                    near_value_return(repr.as_bytes());
                    return MontyObject::None;
                }
                None => b"",
            };
            near_value_return(s);
            MontyObject::None
        }
        "input" => {
            let data = near_input();
            match String::from_utf8(data) {
                Ok(s) => MontyObject::String(s),
                Err(e) => MontyObject::Bytes(e.into_bytes()),
            }
        }
        "log" => {
            let msg = match args.first() {
                Some(MontyObject::String(s)) => s.clone(),
                Some(other) => format!("{other:?}"),
                None => String::new(),
            };
            near_log(&msg);
            MontyObject::None
        }

        "storage_write" => {
            let key = match args.first() { Some(MontyObject::String(s)) => s.as_bytes(), _ => return MontyObject::None };
            let value = match args.get(1) { Some(MontyObject::String(s)) => s.as_bytes(), _ => return MontyObject::None };
            MontyObject::Bool(near_storage_write(key, value))
        }
        "storage_read" => {
            let key = match args.first() { Some(MontyObject::String(s)) => s.as_bytes(), _ => return MontyObject::None };
            match near_storage_read(key) {
                Some(bytes) => MontyObject::String(String::from_utf8(bytes).unwrap_or_default()),
                None => MontyObject::None,
            }
        }
        "storage_remove" => {
            let key = match args.first() { Some(MontyObject::String(s)) => s.as_bytes(), _ => return MontyObject::None };
            MontyObject::Bool(near_storage_remove(key))
        }
        "storage_has_key" => {
            let key = match args.first() { Some(MontyObject::String(s)) => s.as_bytes(), _ => return MontyObject::None };
            MontyObject::Bool(near_storage_has_key(key))
        }

        "current_account_id" => MontyObject::String(near_current_account_id()),
        "predecessor_account_id" => MontyObject::String(near_predecessor_account_id()),
        "signer_account_id" => MontyObject::String(near_signer_account_id()),
        "block_height" => MontyObject::Int(near_block_height() as i64),
        "block_timestamp" => MontyObject::Int(near_block_timestamp() as i64),

        "sha256" => {
            let data = arg_bytes(0).unwrap_or(b"");
            MontyObject::String(to_hex(&near_sha256(data)))
        }
        "keccak256" => {
            let data = arg_bytes(0).unwrap_or(b"");
            MontyObject::String(to_hex(&near_keccak256(data)))
        }

        "signer_account_pk" => MontyObject::String(to_hex(&near_signer_account_pk())),
        "epoch_height" => MontyObject::Int(near_epoch_height() as i64),
        "storage_usage" => MontyObject::Int(near_storage_usage() as i64),

        "account_balance" => MontyObject::String(near_account_balance().to_string()),
        "account_locked_balance" => MontyObject::String(near_account_locked_balance().to_string()),
        "attached_deposit" => MontyObject::String(near_attached_deposit().to_string()),
        "prepaid_gas" => MontyObject::Int(near_prepaid_gas() as i64),
        "used_gas" => MontyObject::Int(near_used_gas() as i64),

        "random_seed" => MontyObject::String(to_hex(&near_random_seed())),
        "keccak512" => { let data = arg_bytes(0).unwrap_or(b""); MontyObject::String(to_hex(&near_keccak512(data))) }
        "ripemd160" => { let data = arg_bytes(0).unwrap_or(b""); MontyObject::String(to_hex(&near_ripemd160(data))) }
        "ecrecover" => {
            let hash = match args.first() { Some(MontyObject::String(s)) => from_hex(s), Some(MontyObject::Bytes(b)) => b.clone(), _ => return MontyObject::None };
            let sig = match args.get(1) { Some(MontyObject::String(s)) => from_hex(s), Some(MontyObject::Bytes(b)) => b.clone(), _ => return MontyObject::None };
            let v = arg_int(2).unwrap_or(0) as u64;
            let malleability_flag = arg_int(3).unwrap_or(0) as u64;
            match near_ecrecover(&hash, &sig, v, malleability_flag) { Some(pk) => MontyObject::String(to_hex(&pk)), None => MontyObject::None }
        }
        "ed25519_verify" => {
            let sig = match args.first() { Some(MontyObject::String(s)) => from_hex(s), Some(MontyObject::Bytes(b)) => b.clone(), _ => return MontyObject::None };
            let msg = arg_bytes(1).unwrap_or(b"");
            let pub_key = match args.get(2) { Some(MontyObject::String(s)) => from_hex(s), Some(MontyObject::Bytes(b)) => b.clone(), _ => return MontyObject::None };
            MontyObject::Bool(near_ed25519_verify(&sig, msg, &pub_key))
        }

        "promise_create" => {
            let account_id = arg_str(0).unwrap_or("");
            let function_name = arg_str(1).unwrap_or("");
            let arguments = arg_bytes(2).unwrap_or(b"");
            let amount = arg_u128(3).unwrap_or(0);
            let gas = arg_int(4).unwrap_or(0) as u64;
            MontyObject::Int(near_promise_create(account_id, function_name, arguments, amount, gas) as i64)
        }
        "promise_then" => {
            let promise_index = arg_int(0).unwrap_or(0) as u64;
            let account_id = arg_str(1).unwrap_or("");
            let function_name = arg_str(2).unwrap_or("");
            let arguments = arg_bytes(3).unwrap_or(b"");
            let amount = arg_u128(4).unwrap_or(0);
            let gas = arg_int(5).unwrap_or(0) as u64;
            MontyObject::Int(near_promise_then(promise_index, account_id, function_name, arguments, amount, gas) as i64)
        }
        "promise_and" => {
            let indices: Vec<u64> = args.iter().filter_map(|a| match a { MontyObject::Int(n) => Some(*n as u64), _ => None }).collect();
            MontyObject::Int(near_promise_and(&indices) as i64)
        }
        "promise_batch_create" => { let account_id = arg_str(0).unwrap_or(""); MontyObject::Int(near_promise_batch_create(account_id) as i64) }
        "promise_batch_then" => { let pi = arg_int(0).unwrap_or(0) as u64; let aid = arg_str(1).unwrap_or(""); MontyObject::Int(near_promise_batch_then(pi, aid) as i64) }
        "promise_results_count" => MontyObject::Int(near_promise_results_count() as i64),
        "promise_result" => {
            let result_idx = arg_int(0).unwrap_or(0) as u64;
            let (status, data) = near_promise_result(result_idx);
            if status == 1 { match String::from_utf8(data) { Ok(s) => MontyObject::String(s), Err(e) => MontyObject::Bytes(e.into_bytes()) } } else { MontyObject::None }
        }
        "promise_return" => { let pid = arg_int(0).unwrap_or(0) as u64; near_promise_return(pid); MontyObject::None }

        "promise_batch_action_create_account" => { let pi = arg_int(0).unwrap_or(0) as u64; near_promise_batch_action_create_account(pi); MontyObject::None }
        "promise_batch_action_deploy_contract" => { let pi = arg_int(0).unwrap_or(0) as u64; let code = arg_bytes(1).unwrap_or(b""); near_promise_batch_action_deploy_contract(pi, code); MontyObject::None }
        "promise_batch_action_function_call" => {
            let pi = arg_int(0).unwrap_or(0) as u64;
            let fname = arg_str(1).unwrap_or("");
            let arguments = arg_bytes(2).unwrap_or(b"");
            let amount = arg_u128(3).unwrap_or(0);
            let gas = arg_int(4).unwrap_or(0) as u64;
            near_promise_batch_action_function_call(pi, fname, arguments, amount, gas);
            MontyObject::None
        }
        "promise_batch_action_function_call_weight" => {
            let pi = arg_int(0).unwrap_or(0) as u64;
            let fname = arg_str(1).unwrap_or("");
            let arguments = arg_bytes(2).unwrap_or(b"");
            let amount = arg_u128(3).unwrap_or(0);
            let gas = arg_int(4).unwrap_or(0) as u64;
            let weight = arg_int(5).unwrap_or(1) as u64;
            near_promise_batch_action_function_call_weight(pi, fname, arguments, amount, gas, weight);
            MontyObject::None
        }
        "promise_batch_action_transfer" => { let pi = arg_int(0).unwrap_or(0) as u64; let amount = arg_u128(1).unwrap_or(0); near_promise_batch_action_transfer(pi, amount); MontyObject::None }
        "promise_batch_action_stake" => {
            let pi = arg_int(0).unwrap_or(0) as u64;
            let amount = arg_u128(1).unwrap_or(0);
            let pk = match args.get(2) { Some(MontyObject::String(s)) => from_hex(s), Some(MontyObject::Bytes(b)) => b.clone(), _ => return MontyObject::None };
            near_promise_batch_action_stake(pi, amount, &pk);
            MontyObject::None
        }
        "promise_batch_action_add_key_with_full_access" => {
            let pi = arg_int(0).unwrap_or(0) as u64;
            let pk = match args.get(1) { Some(MontyObject::String(s)) => from_hex(s), Some(MontyObject::Bytes(b)) => b.clone(), _ => return MontyObject::None };
            let nonce = arg_int(2).unwrap_or(0) as u64;
            near_promise_batch_action_add_key_with_full_access(pi, &pk, nonce);
            MontyObject::None
        }
        "promise_batch_action_add_key_with_function_call" => {
            let pi = arg_int(0).unwrap_or(0) as u64;
            let pk = match args.get(1) { Some(MontyObject::String(s)) => from_hex(s), Some(MontyObject::Bytes(b)) => b.clone(), _ => return MontyObject::None };
            let nonce = arg_int(2).unwrap_or(0) as u64;
            let allowance = arg_u128(3).unwrap_or(0);
            let receiver_id = arg_str(4).unwrap_or("");
            let function_names = arg_str(5).unwrap_or("");
            near_promise_batch_action_add_key_with_function_call(pi, &pk, nonce, allowance, receiver_id, function_names);
            MontyObject::None
        }
        "promise_batch_action_delete_key" => {
            let pi = arg_int(0).unwrap_or(0) as u64;
            let pk = match args.get(1) { Some(MontyObject::String(s)) => from_hex(s), Some(MontyObject::Bytes(b)) => b.clone(), _ => return MontyObject::None };
            near_promise_batch_action_delete_key(pi, &pk);
            MontyObject::None
        }
        "promise_batch_action_delete_account" => {
            let pi = arg_int(0).unwrap_or(0) as u64;
            let bid = arg_str(1).unwrap_or("");
            near_promise_batch_action_delete_account(pi, bid);
            MontyObject::None
        }

        "validator_stake" => { let aid = arg_str(0).unwrap_or(""); MontyObject::String(near_validator_stake(aid).to_string()) }
        "validator_total_stake" => MontyObject::String(near_validator_total_stake().to_string()),

        "alt_bn128_g1_multiexp" => { let data = arg_bytes(0).unwrap_or(b""); MontyObject::String(to_hex(&near_alt_bn128_g1_multiexp(data))) }
        "alt_bn128_g1_sum" => { let data = arg_bytes(0).unwrap_or(b""); MontyObject::String(to_hex(&near_alt_bn128_g1_sum(data))) }
        "alt_bn128_pairing_check" => { let data = arg_bytes(0).unwrap_or(b""); MontyObject::Bool(near_alt_bn128_pairing_check(data)) }

        "bls12381_p1_sum" => { let data = arg_bytes(0).unwrap_or(b""); match near_bls12381_p1_sum(data) { Some(r) => MontyObject::String(to_hex(&r)), None => MontyObject::None } }
        "bls12381_p2_sum" => { let data = arg_bytes(0).unwrap_or(b""); match near_bls12381_p2_sum(data) { Some(r) => MontyObject::String(to_hex(&r)), None => MontyObject::None } }
        "bls12381_g1_multiexp" => { let data = arg_bytes(0).unwrap_or(b""); match near_bls12381_g1_multiexp(data) { Some(r) => MontyObject::String(to_hex(&r)), None => MontyObject::None } }
        "bls12381_g2_multiexp" => { let data = arg_bytes(0).unwrap_or(b""); match near_bls12381_g2_multiexp(data) { Some(r) => MontyObject::String(to_hex(&r)), None => MontyObject::None } }
        "bls12381_map_fp_to_g1" => { let data = arg_bytes(0).unwrap_or(b""); match near_bls12381_map_fp_to_g1(data) { Some(r) => MontyObject::String(to_hex(&r)), None => MontyObject::None } }
        "bls12381_map_fp2_to_g2" => { let data = arg_bytes(0).unwrap_or(b""); match near_bls12381_map_fp2_to_g2(data) { Some(r) => MontyObject::String(to_hex(&r)), None => MontyObject::None } }
        "bls12381_pairing_check" => { let data = arg_bytes(0).unwrap_or(b""); MontyObject::Bool(near_bls12381_pairing_check(data)) }
        "bls12381_p1_decompress" => { let data = arg_bytes(0).unwrap_or(b""); match near_bls12381_p1_decompress(data) { Some(r) => MontyObject::String(to_hex(&r)), None => MontyObject::None } }
        "bls12381_p2_decompress" => { let data = arg_bytes(0).unwrap_or(b""); match near_bls12381_p2_decompress(data) { Some(r) => MontyObject::String(to_hex(&r)), None => MontyObject::None } }

        _ => {
            near_log(&format!("unknown external function: {name}"));
            MontyObject::None
        }
    }
}

// ---------------------------------------------------------------------------
// Python execution engine — parse + compile + run on-chain
// ---------------------------------------------------------------------------

fn run_python(code: &str) {
    let runner = MontyRun::new(code.to_owned(), "contract.py", vec![])
        .unwrap_or_else(|e| {
            near_log(&format!("monty compile error: {e}"));
            panic!("monty compile error");
        });

    let mut near_print = NearPrint::new();
    let print = PrintWriter::Callback(&mut near_print);

    let mut progress = runner
        .start(vec![], NoLimitTracker, print)
        .unwrap_or_else(|e| {
            near_log(&format!("monty start error: {e}"));
            panic!("monty start error");
        });

    loop {
        match progress {
            RunProgress::FunctionCall(call) => {
                let result = dispatch_function(&call.function_name, &call.args);
                progress = call
                    .resume(ExtFunctionResult::Return(result), PrintWriter::Callback(&mut near_print))
                    .unwrap_or_else(|e| {
                        near_log(&format!("monty runtime error: {e}"));
                        panic!("monty runtime error");
                    });
            }
            RunProgress::NameLookup(lookup) => {
                let result = if is_near_function(&lookup.name) {
                    // Return a sentinel — the actual work happens on FunctionCall
                    NameLookupResult::Value(MontyObject::None)
                } else {
                    NameLookupResult::Undefined
                };
                progress = lookup
                    .resume(result, PrintWriter::Callback(&mut near_print))
                    .unwrap_or_else(|e| {
                        near_log(&format!("monty name error: {e}"));
                        panic!("monty name error");
                    });
            }
            RunProgress::Complete(_) => break,
            RunProgress::OsCall(_) => {
                near_log("OS calls are not permitted in NEAR contracts");
                panic!("OS call attempted");
            }
            RunProgress::ResolveFutures(_) => {
                near_log("Async futures are not supported in NEAR contracts");
                panic!("async futures not supported");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// NEAR contract export
// ---------------------------------------------------------------------------

/// Execute arbitrary Python code on-chain.
///
/// Input: raw Python source code as the call arguments.
/// All NEAR host functions are available as Python builtins (no imports needed).
/// Python's print() output goes to NEAR logs.
///
/// Example:
///   near call <contract> execute '"value_return(\"hello world\")"' --accountId <caller>
#[no_mangle]
pub extern "C" fn execute() {
    let input_bytes = near_input();
    let code = String::from_utf8(input_bytes).unwrap_or_else(|_| {
        near_log("invalid UTF-8 input");
        panic!("invalid UTF-8 input");
    });

    run_python(&code);
}
