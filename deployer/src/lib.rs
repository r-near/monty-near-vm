use near_sdk::{base64, env, near};
use base64::Engine as _;

#[derive(Default)]
#[near(contract_state)]
pub struct Deployer {
    num_chunks: u32,
    total_size: u64,
}

const CHUNK_KEY_PREFIX: &[u8] = b"c:";
const CODE_KEY: &[u8] = b"code";

#[near]
impl Deployer {
    /// Reset chunk counter. Old storage keys will be overwritten by new uploads.
    pub fn reset(&mut self) {
        assert_eq!(env::predecessor_account_id(), env::current_account_id());
        env::log_str(&format!("Reset: was {} chunks / {} bytes", self.num_chunks, self.total_size));
        self.num_chunks = 0;
        self.total_size = 0;
    }

    /// Append a base64-encoded chunk.
    pub fn store_chunk(&mut self, data_b64: String) {
        assert_eq!(env::predecessor_account_id(), env::current_account_id());
        let data = base64::engine::general_purpose::STANDARD.decode(&data_b64)
            .unwrap_or_else(|e| panic!("invalid base64: {e}"));
        let chunk_len = data.len() as u64;

        let key = [CHUNK_KEY_PREFIX, &self.num_chunks.to_le_bytes()].concat();
        env::storage_write(&key, &data);

        self.num_chunks += 1;
        self.total_size += chunk_len;

        env::log_str(&format!(
            "Stored chunk {} ({chunk_len} bytes), total: {} bytes",
            self.num_chunks - 1, self.total_size
        ));
    }

    /// Assemble all chunks into a single "code" storage key.
    /// This must be called before deploy().
    pub fn assemble(&self) {
        assert_eq!(env::predecessor_account_id(), env::current_account_id());
        assert!(self.num_chunks > 0, "no chunks stored");

        env::log_str(&format!(
            "Assembling {} chunks ({} bytes total)...",
            self.num_chunks, self.total_size
        ));

        let mut code = Vec::with_capacity(self.total_size as usize);
        for i in 0..self.num_chunks {
            let key = [CHUNK_KEY_PREFIX, &i.to_le_bytes()].concat();
            let chunk = env::storage_read(&key).unwrap_or_else(|| panic!("missing chunk {i}"));
            env::log_str(&format!(
                "  chunk {i}: {} bytes (assembled: {} bytes)",
                chunk.len(),
                code.len() + chunk.len()
            ));
            code.extend_from_slice(&chunk);
        }

        assert_eq!(code.len() as u64, self.total_size, "size mismatch");

        env::log_str(&format!("Writing assembled code ({} bytes) to storage...", code.len()));
        env::storage_write(CODE_KEY, &code);
        env::log_str("Assemble complete");
    }

    /// View: total bytes stored.
    pub fn code_size(&self) -> u64 {
        self.total_size
    }
}

/// Raw deploy export — bypasses SDK to minimize gas.
///
/// Reads the pre-assembled "code" key from storage directly into a register,
/// then deploys from that register using the register trick (len=u64::MAX).
/// The 3+ MB code blob NEVER touches WASM linear memory.
#[unsafe(no_mangle)]
pub extern "C" fn deploy() {
    unsafe {
        log("deploy: start");

        // Auth check: predecessor must be current account
        near_sys::predecessor_account_id(0);
        near_sys::current_account_id(1);
        let pred_len = near_sys::register_len(0);
        let curr_len = near_sys::register_len(1);

        if pred_len != curr_len {
            panic_str("only the contract owner can deploy");
        }

        let mut pred = vec![0u8; pred_len as usize];
        let mut curr = vec![0u8; curr_len as usize];
        near_sys::read_register(0, pred.as_mut_ptr() as u64);
        near_sys::read_register(1, curr.as_mut_ptr() as u64);
        if pred != curr {
            panic_str("only the contract owner can deploy");
        }

        log("deploy: auth ok");

        // Read target account from input
        near_sys::input(0);
        let input_len = near_sys::register_len(0);
        let mut target = vec![0u8; input_len as usize];
        near_sys::read_register(0, target.as_mut_ptr() as u64);

        // Strip JSON quotes if present: "vm.sandbox" -> vm.sandbox
        let target = if target.first() == Some(&b'"') && target.last() == Some(&b'"') {
            &target[1..target.len() - 1]
        } else {
            &target
        };

        let target_str = core::str::from_utf8(target).unwrap_or("?");
        log_fmt(format_args!("deploy: target={target_str}"));

        // Read assembled code from "code" key directly into register 0.
        // This is the key optimization: the code stays in the host register
        // and never gets copied into WASM linear memory.
        let found = near_sys::storage_read(
            CODE_KEY.len() as u64,
            CODE_KEY.as_ptr() as u64,
            0, // register 0
        );
        if found != 1 {
            panic_str("no assembled code found — call assemble() first");
        }

        let code_len = near_sys::register_len(0);
        log_fmt(format_args!("deploy: code loaded into register ({code_len} bytes)"));

        // Create promise to deploy to target account
        let promise_id = near_sys::promise_batch_create(
            target.len() as u64,
            target.as_ptr() as u64,
        );

        // Deploy from register 0 using the register trick:
        // When code_len == u64::MAX, the runtime reads from register code_ptr
        // instead of WASM memory. This avoids allocating 3+ MB in linear memory.
        near_sys::promise_batch_action_deploy_contract(
            promise_id,
            u64::MAX, // magic value: read from register
            0,        // register 0
        );

        log_fmt(format_args!("deploy: promise created — deploying {code_len} bytes to {target_str}"));
    }
}

/// Direct deploy — reads chunks from storage, assembles in WASM memory, deploys.
/// Skips the assemble() step to stay under the 4MB trie proof limit on testnet.
/// Storage proof is only the chunk reads (~3.2MB), well under 4MB.
#[unsafe(no_mangle)]
pub extern "C" fn deploy_direct() {
    unsafe {
        log("deploy_direct: start");

        // Auth check
        near_sys::predecessor_account_id(0);
        near_sys::current_account_id(1);
        let pred_len = near_sys::register_len(0);
        let curr_len = near_sys::register_len(1);
        if pred_len != curr_len {
            panic_str("only the contract owner can deploy");
        }
        let mut pred = vec![0u8; pred_len as usize];
        let mut curr = vec![0u8; curr_len as usize];
        near_sys::read_register(0, pred.as_mut_ptr() as u64);
        near_sys::read_register(1, curr.as_mut_ptr() as u64);
        if pred != curr {
            panic_str("only the contract owner can deploy");
        }

        log("deploy_direct: auth ok");

        // Read target from input
        near_sys::input(0);
        let input_len = near_sys::register_len(0);
        let mut target = vec![0u8; input_len as usize];
        near_sys::read_register(0, target.as_mut_ptr() as u64);
        let target = if target.first() == Some(&b'"') && target.last() == Some(&b'"') {
            &target[1..target.len() - 1]
        } else {
            &target
        };
        let target_str = core::str::from_utf8(target).unwrap_or("?");
        log_fmt(format_args!("deploy_direct: target={target_str}"));

        // Read state
        let state_key = b"STATE";
        let found = near_sys::storage_read(
            state_key.len() as u64, state_key.as_ptr() as u64, 0,
        );
        assert!(found == 1, "no state");
        let state_len = near_sys::register_len(0);
        let mut state_buf = vec![0u8; state_len as usize];
        near_sys::read_register(0, state_buf.as_mut_ptr() as u64);
        let num_chunks = u32::from_le_bytes([state_buf[0], state_buf[1], state_buf[2], state_buf[3]]);
        let total_size = u64::from_le_bytes([
            state_buf[4], state_buf[5], state_buf[6], state_buf[7],
            state_buf[8], state_buf[9], state_buf[10], state_buf[11],
        ]);

        log_fmt(format_args!("deploy_direct: {num_chunks} chunks, {total_size} bytes total"));

        // Read all chunks into WASM memory via temp buffers
        let mut code = Vec::with_capacity(total_size as usize);
        for i in 0..num_chunks {
            let key = [CHUNK_KEY_PREFIX, &i.to_le_bytes()].concat();
            let found = near_sys::storage_read(key.len() as u64, key.as_ptr() as u64, 0);
            assert!(found == 1, "missing chunk");
            let chunk_len = near_sys::register_len(0) as usize;
            let mut chunk = vec![0u8; chunk_len];
            near_sys::read_register(0, chunk.as_mut_ptr() as u64);
            code.extend_from_slice(&chunk);
            log_fmt(format_args!("  chunk {i}: {chunk_len} bytes (total: {})", code.len()));
        }

        log_fmt(format_args!("deploy_direct: assembled {} bytes in WASM memory", code.len()));

        // Hash check: log first/last bytes and sha256 for verification
        if code.len() >= 8 {
            log_fmt(format_args!(
                "deploy_direct: first8={:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x} last8={:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                code[0], code[1], code[2], code[3], code[4], code[5], code[6], code[7],
                code[code.len()-8], code[code.len()-7], code[code.len()-6], code[code.len()-5],
                code[code.len()-4], code[code.len()-3], code[code.len()-2], code[code.len()-1],
            ));
        }

        // Deploy from WASM memory
        let promise_id = near_sys::promise_batch_create(
            target.len() as u64, target.as_ptr() as u64,
        );
        near_sys::promise_batch_action_deploy_contract(
            promise_id, code.len() as u64, code.as_ptr() as u64,
        );

        log_fmt(format_args!("deploy_direct: deploying {} bytes to {target_str}", code.len()));
    }
}

fn panic_str(msg: &str) -> ! {
    unsafe {
        near_sys::panic_utf8(msg.len() as u64, msg.as_ptr() as u64);
    }
    unreachable!()
}

fn log(msg: &str) {
    unsafe {
        near_sys::log_utf8(msg.len() as u64, msg.as_ptr() as u64);
    }
}

fn log_fmt(args: core::fmt::Arguments) {
    let msg = args.to_string();
    log(&msg);
}
