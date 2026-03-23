# monty-near-vm

A Python virtual machine deployed as a NEAR smart contract. Anyone can call it with arbitrary Python code and it executes on-chain.

This embeds the full [Monty](https://github.com/pydantic/monty) Python compiler (including the [ruff](https://github.com/astral-sh/ruff) parser) inside a 3.2 MB WASM contract. Python source code is parsed, compiled to bytecode, and executed within a single NEAR function call. Python's `print()` is wired to NEAR's `log_utf8`, and the full NEAR host API is exposed as Python builtin functions.

**Live on testnet:** [`monty-vm.testnet`](https://testnet.nearblocks.io/address/monty-vm.testnet)

## Quick example

```python
def fib(n):
    if n <= 1:
        return n
    return fib(n - 1) + fib(n - 2)

for i in range(11):
    print("fib(" + str(i) + ") = " + str(fib(i)))

value_return(str(fib(10)))
```

On-chain output (7.6 TGas):
```
log: fib(0) = 0
log: fib(1) = 1
log: fib(2) = 1
log: fib(3) = 2
log: fib(4) = 3
log: fib(5) = 5
log: fib(6) = 8
log: fib(7) = 13
log: fib(8) = 21
log: fib(9) = 34
log: fib(10) = 55

Return value: 55
```

## How it works

The contract exposes a single method:

```
execute(code: raw UTF-8 bytes) -> raw bytes
```

The input is raw Python source code (not JSON-wrapped). The contract:
1. Parses the source using the embedded ruff parser
2. Compiles it to Monty bytecode
3. Executes the bytecode on the Monty VM
4. Routes external function calls (like `storage_write`, `sha256`, `promise_create`) to the corresponding NEAR host functions
5. Returns whatever was passed to `value_return()`

`print()` statements produce NEAR logs visible in the transaction receipt.

## Python builtins mapped to NEAR

All of these are callable directly from Python code passed to `execute`:

### Core
| Function | Description |
|----------|-------------|
| `value_return(data)` | Set the return value of the function call |
| `input()` | Read the raw input bytes as a string |
| `log(msg)` | Write a log line (also available via `print()`) |

### Storage
| Function | Description |
|----------|-------------|
| `storage_write(key, value)` | Write a key-value pair to contract storage |
| `storage_read(key)` | Read a value from storage (returns `None` if missing) |
| `storage_remove(key)` | Remove a key from storage |
| `storage_has_key(key)` | Check if a key exists |
| `storage_usage()` | Get total storage bytes used |

### Context
| Function | Description |
|----------|-------------|
| `current_account_id()` | The account this contract is deployed to |
| `predecessor_account_id()` | The account that called this function |
| `signer_account_id()` | The account that signed the transaction |
| `signer_account_pk()` | Public key of the signer (hex) |
| `block_height()` | Current block height |
| `block_timestamp()` | Current block timestamp in nanoseconds |
| `epoch_height()` | Current epoch height |

### Economics
| Function | Description |
|----------|-------------|
| `account_balance()` | Account balance in yoctoNEAR (string) |
| `account_locked_balance()` | Locked balance in yoctoNEAR (string) |
| `attached_deposit()` | Attached deposit in yoctoNEAR (string) |
| `prepaid_gas()` | Prepaid gas in gas units |
| `used_gas()` | Gas used so far |

### Cryptography
| Function | Description |
|----------|-------------|
| `sha256(data)` | SHA-256 hash (hex string) |
| `keccak256(data)` | Keccak-256 hash (hex string) |
| `keccak512(data)` | Keccak-512 hash (hex string) |
| `ripemd160(data)` | RIPEMD-160 hash (hex string) |
| `random_seed()` | VRF-based random seed (hex string) |
| `ecrecover(hash, sig, v, malleability_flag)` | Recover secp256k1 public key |
| `ed25519_verify(sig, msg, pub_key)` | Verify Ed25519 signature |

### Promises (cross-contract calls)
| Function | Description |
|----------|-------------|
| `promise_create(account, method, args, amount, gas)` | Create a cross-contract call |
| `promise_then(promise_id, account, method, args, amount, gas)` | Chain a callback |
| `promise_and(promise_ids...)` | Join multiple promises |
| `promise_batch_create(account)` | Create a batch promise |
| `promise_batch_then(promise_id, account)` | Chain a batch callback |
| `promise_batch_action_transfer(promise_id, amount)` | Add a transfer action |
| `promise_batch_action_function_call(promise_id, method, args, amount, gas)` | Add a function call |
| `promise_batch_action_create_account(promise_id)` | Add a create-account action |
| `promise_batch_action_deploy_contract(promise_id, code)` | Add a deploy action |
| `promise_batch_action_stake(promise_id, amount, public_key)` | Add a stake action |
| `promise_batch_action_delete_account(promise_id, beneficiary)` | Add a delete-account action |
| `promise_results_count()` | Number of promise results available |
| `promise_result(index)` | Get a promise result |
| `promise_return(promise_id)` | Return a promise as the function result |

### Validators
| Function | Description |
|----------|-------------|
| `validator_stake(account_id)` | Get a validator's stake |
| `validator_total_stake()` | Get total validator stake |

### Advanced cryptography
| Function | Description |
|----------|-------------|
| `alt_bn128_g1_multiexp(data)` | BN128 G1 multi-exponentiation |
| `alt_bn128_g1_sum(data)` | BN128 G1 point addition |
| `alt_bn128_pairing_check(data)` | BN128 pairing check |
| `bls12381_p1_sum(data)` | BLS12-381 P1 point addition |
| `bls12381_p2_sum(data)` | BLS12-381 P2 point addition |
| `bls12381_g1_multiexp(data)` | BLS12-381 G1 multi-exponentiation |
| `bls12381_g2_multiexp(data)` | BLS12-381 G2 multi-exponentiation |
| `bls12381_pairing_check(data)` | BLS12-381 pairing check |
| `bls12381_map_fp_to_g1(data)` | Map field element to G1 |
| `bls12381_map_fp2_to_g2(data)` | Map field element to G2 |
| `bls12381_p1_decompress(data)` | Decompress P1 point |
| `bls12381_p2_decompress(data)` | Decompress P2 point |

## Project structure

```
monty-near-vm/
  src/lib.rs              # The VM contract (~820 lines)
  Cargo.toml              # Dependencies: monty, near-sys, getrandom
  rust-toolchain.toml     # Nightly Rust (for -Zbuild-std)
  .cargo/config.toml      # wasm32 target, MVP CPU, no bulk-memory
  deployer/
    src/lib.rs            # Bootstrap deployer contract (~270 lines)
    Cargo.toml            # Dependencies: near-sdk, near-sys
  tests/
    src/sandbox.rs        # Sandbox integration tests (8 tests)
    src/deploy_testnet.rs # Testnet deployment script
    src/run_testnet.rs    # Execute Python on testnet
  examples/
    test_local.rs         # Local x86 test (no blockchain)
```

## Building

### Prerequisites

- Rust nightly (managed via `rust-toolchain.toml`)
- [`cargo-near`](https://github.com/near/cargo-near) for building the deployer
- [`wasm-opt`](https://github.com/WebAssembly/binaryen) for WASM optimization
- Docker for sandbox tests

### Build the VM contract

```bash
# Build with nightly + build-std for MVP WASM (no bulk-memory instructions)
cargo build --release -Zbuild-std=std,panic_abort

# Optimize for size
wasm-opt -Oz target/wasm32-unknown-unknown/release/monty_near_vm.wasm \
  -o target/monty_near_vm_optimized.wasm

# Verify no bulk-memory instructions (needed for NearVM compatibility)
wasm-tools validate --features=-bulk-memory target/monty_near_vm_optimized.wasm
```

The optimized WASM is ~3.1 MB.

### Build the deployer

```bash
cd deployer
cargo near build non-reproducible-wasm --no-abi
```

The deployer is ~80 KB.

## Deploying

NEAR has a 1.5 MB transaction size limit, but the VM contract is 3.2 MB. The deployer contract solves this with a chunked upload + self-deploy pattern.

### The deployer contract

The deployer is a temporary bootstrap contract that:

1. **Accepts chunks** via `store_chunk(data_b64)` — each call uploads ~300 KB of base64-encoded WASM, stored in individual storage keys
2. **Deploys the VM** via `deploy_direct(target_account)` — reads all chunks from storage, assembles them in WASM memory, and creates a `promise_batch_action_deploy_contract` to deploy the assembled code to the target account

When the target is the deployer's own account (self-deploy), the VM contract replaces the deployer. The deployer is a self-destructing bootloader.

The deployer has two deploy strategies:

- **`deploy_direct()`** — Assembles chunks in WASM linear memory and deploys from there. Uses ~94 TGas. Works on both sandbox and testnet. This is the primary deploy method.
- **`deploy()`** — Reads a pre-assembled `"code"` key from storage directly into a host register, then deploys using the **register trick** (`code_len = u64::MAX` tells the NEAR runtime to read from a register instead of WASM memory). Uses only ~54 TGas but requires a separate `assemble()` step that exceeds testnet's 4 MB trie proof limit. Only works on sandbox where that limit is disabled.

Both strategies bypass the near-sdk (raw `#[no_mangle] extern "C"` exports) to minimize gas overhead.

### Deploy to testnet

```bash
cd tests

# Build the deploy tool
cargo build --bin deploy-testnet

# Run (uses near-cli-rs keychain for signing)
cargo run --bin deploy-testnet
```

The deploy script:
1. Deploys the deployer contract to the target account
2. Uploads the VM WASM in 11 base64-encoded chunks (~50 TGas each)
3. Calls `deploy_direct()` to assemble and deploy (~94 TGas + 208 TGas for the deploy action)
4. Runs a test `execute` call to verify

Total cost: ~14 transactions, ~150 NEAR for storage staking (3.2 MB contract code + 3.2 MB chunk data).

### Deploy to sandbox

```bash
# Start a sandbox
docker run -d --name sandbox -p 3030:3030 nearprotocol/sandbox:2.11.0-rc.3 \
  --rpc-addr 0.0.0.0:3030

# Run tests (deploy + execute)
SANDBOX_RPC=http://localhost:3030 cargo test -- --nocapture --test-threads=1
```

## Gas costs

Measured on testnet (protocol version 83, 1 PGas limit):

| Operation | Gas |
|-----------|-----|
| `store_chunk` (300 KB) | ~50 TGas |
| `deploy_direct` (assemble 3.2 MB + create deploy promise) | 94 TGas |
| Deploy action receipt (3.2 MB contract) | 208 TGas |
| `execute` — hello world | ~2 TGas |
| `execute` — fibonacci(10) with logging | ~8 TGas |
| `execute` — storage write + read | ~5 TGas |
| `execute` — SHA-256 hash | ~3 TGas |

## WASM build details

The VM contract is built with specific flags to ensure compatibility with NEAR's NearVM (Wasmer singlepass) runtime:

- **Nightly Rust** with `-Zbuild-std=std,panic_abort` — rebuilds the standard library from source to avoid bulk-memory instructions that Rust stable (1.87+) emits by default
- **`target-cpu=mvp`** — targets the minimal viable product WASM spec, excluding post-MVP features
- **No bulk-memory** — NearVM on protocol <=82 rejects `memory.fill` and `memory.copy` instructions
- **`opt-level = "s"`** with LTO — optimizes for size while keeping the contract under the 4 MB limit
- **`wasm-opt -Oz`** — further reduces the binary from ~3.4 MB to ~3.1 MB
- **Custom `getrandom` backend** — routes randomness to NEAR's VRF-based `random_seed()` host function

## Testing

### Local (x86, no blockchain)

```bash
cargo run --example test_local
```

Runs the Monty VM natively with sample Python code. Useful for quick iteration.

### Sandbox (Docker-based NEAR sandbox)

```bash
# With external sandbox (recommended — avoids testcontainers OOM issues):
docker run -d --name sandbox -p 3030:3030 nearprotocol/sandbox:2.11.0-rc.3 \
  --rpc-addr 0.0.0.0:3030
SANDBOX_RPC=http://localhost:3030 cargo test -- --nocapture --test-threads=1

# Or with testcontainers (auto-managed, but may crash with large contracts):
cargo test -- --nocapture --test-threads=1
```

Tests: hello world, arithmetic, print+return, storage read/write, SHA-256, block context, counter (multi-call state), recursive fibonacci.

### Testnet

```bash
# Execute arbitrary Python on testnet
cargo run --bin run-testnet -- 'value_return(str(21 * 2))'
```

## Architecture

### VM contract (`src/lib.rs`)

The contract has a single entry point `execute()` that:

1. Reads raw UTF-8 input via `near_sys::input()`
2. Creates a `MontyRun` with a `PrintWriter::Callback` that buffers lines and flushes them to `log_utf8` on newline
3. Starts the Monty execution loop, handling:
   - **`NameLookup`** — resolves function names against the ~55 known NEAR host functions
   - **`FunctionCall`** — dispatches to the appropriate NEAR host function wrapper
   - **`Complete`** — execution finished
4. Returns the result via `near_sys::value_return()`

The contract uses `near_sys` directly (no near-sdk) to minimize binary size and gas overhead.

### Deployer contract (`deployer/src/lib.rs`)

A hybrid contract mixing near-sdk methods (for `store_chunk`, `assemble`, `reset`, `code_size`) with raw `#[no_mangle]` exports (for `deploy` and `deploy_direct`) to minimize gas in the deploy step.

## Technical constraints

| Constraint | Value | Impact |
|------------|-------|--------|
| NEAR transaction size limit | 1.5 MB | Can't deploy 3.2 MB contract directly; requires chunked upload via deployer |
| NEAR max contract size | 4 MB | VM contract at 3.1 MB fits with margin |
| Trie proof size limit (testnet) | 4 MB | Can't read+write 3.2 MB in one receipt; `deploy_direct` reads ~3.2 MB (under limit), deploy action runs in separate receipt |
| Trie proof size limit (sandbox) | Disabled | `assemble` + register-trick `deploy` works on sandbox but not testnet |
| Max gas per transaction | 1 PGas (protocol 83+) | Deploy fits comfortably; was 300 TGas before protocol 83 |
| Storage staking cost | ~10 NEAR per 100 KB | 3.2 MB contract ≈ 32 NEAR locked for storage |

## Deep dive: deploying a 3.2 MB contract to NEAR

Deploying this contract required solving several interacting constraints in the NEAR protocol. This section documents the problems encountered, the solutions explored, and the gas cost math behind it all. It's based on analysis of the [nearcore 2.10.7](https://github.com/near/nearcore) source code.

### The core problem

The VM contract (with the full Monty compiler + ruff parser embedded) compiles to a 3.2 MB WASM binary. NEAR's protocol has several limits that make deploying this nontrivial:

- **1.5 MB transaction size limit** — A `DeployContractAction` includes the full WASM inline. At 3.2 MB, the contract simply can't fit in a transaction.
- **4 MB max contract size** — Our 3.2 MB binary fits, but barely.
- **300 TGas max gas per transaction** (protocol <=82) — Reassembling 3.2 MB of data from storage costs significant gas.
- **4 MB per-receipt trie proof limit** (protocol >=69) — Reading and writing large values in a single receipt is limited by how much storage proof data can be included in a state witness.

The solution is a **deployer contract** — a temporary bootstrap contract that accepts the WASM in chunks, then deploys it via a promise.

### How `DeployContractAction` works internally

From [`nearcore/runtime/runtime/src/actions.rs`](https://github.com/near/nearcore/blob/2.10.7/runtime/runtime/src/actions.rs): the `DeployContract` action requires the **full WASM binary inline** in the action receipt. There is no mechanism to reference a storage key, stage code incrementally, or deploy in parts. The action completely overwrites any existing contract code on the account.

The gas cost formula (from `nearcore/core/parameters/res/runtime_configs/parameters.yaml`):

```
send_fee:  184,765,750,000 (base) + 6,812,999 × code_bytes
exec_fee:  184,765,750,000 (base) + 6,812,999 × code_bytes
```

For 3.2 MB: ~22 TGas send + ~22 TGas exec = ~44 TGas total for the deploy action itself. This is manageable — the hard part is getting the bytes assembled.

### Strategy 1: assemble + register trick (sandbox only)

The most gas-efficient approach we found uses NEAR's **register trick**. When calling `promise_batch_action_deploy_contract(promise_id, code_len, code_ptr)`, if `code_len == u64::MAX`, the runtime reads from a **host register** instead of WASM linear memory ([`nearcore/runtime/near-vm-runner/src/logic/vmstate.rs:280-292`](https://github.com/near/nearcore/blob/2.10.7/runtime/near-vm-runner/src/logic/vmstate.rs#L280-L292)):

```rust
pub fn get_memory_or_register(gas_counter, memory, registers, ptr, len) -> Cow<[u8]> {
    if len == u64::MAX {
        registers.get(gas_counter, ptr).map(Cow::Borrowed)  // read from register
    } else {
        memory.view(gas_counter, MemSlice { ptr, len })      // read from WASM memory
    }
}
```

This matters because of the **39x cost difference** between register and memory reads:

| Operation | Cost per byte |
|-----------|--------------|
| `read_register_byte` | 98,562 gas |
| `read_memory_byte` | 3,801,333 gas |

For 3.2 MB, that's 0.3 TGas (register) vs 12.2 TGas (memory).

The flow is:
1. **`assemble()`** — Read all chunks from storage, concatenate in WASM memory, write to a single `"code"` storage key (~164 TGas)
2. **`deploy()`** — `storage_read("code")` loads 3.2 MB directly into register 0. Then `promise_batch_action_deploy_contract(promise_id, u64::MAX, 0)` deploys from the register. The 3.2 MB **never touches WASM linear memory**. (~54 TGas)

Total: 164 + 54 = ~218 TGas for the function calls, plus 208 TGas for the deploy action receipt.

**Why this only works on sandbox:** The `assemble()` step reads 3.2 MB of chunks and writes 3.2 MB to the `"code"` key — that's ~6.4 MB of storage data touched in one receipt, exceeding the 4 MB trie proof limit. The NEAR sandbox [explicitly disables this limit](https://github.com/near/nearcore/blob/2.10.7/core/parameters/src/config_store.rs#L194) (`per_receipt_storage_proof_size_limit = usize::max_value()`) for the benchmarknet config which the sandbox uses. Testnet and mainnet enforce the 4 MB limit starting at protocol version 69.

### Strategy 2: deploy_direct (testnet-compatible)

Since we can't write the assembled code to a single storage key on testnet (trie proof limit), we skip the assembly step entirely:

1. **`deploy_direct()`** — Read all 11 chunks from individual storage keys into a `Vec` in WASM linear memory, then deploy from that memory.

The storage proof for this approach is only the **reads** (~3.2 MB), since the deploy action happens in a separate receipt with its own proof budget. 3.2 MB < 4 MB limit.

Gas: ~94 TGas for the function + 208 TGas for the deploy action.

### Why the naive approach failed initially

The very first deployer attempt (before the assemble/register trick) used this same pattern but failed at 300 TGas. The gas breakdown reveals why:

**Hidden cost: `Vec::resize` zero-filling on MVP WASM.** When building for `target-cpu=mvp` (no bulk-memory), the Rust standard library can't use `memory.fill`. Instead, `Vec::resize(new_len, 0)` compiles to a **loop of `i32.store` instructions**, each costing `regular_op_cost` (3,856,371 gas). For 3.2 MB:

```
3,200,000 bytes / 4 bytes per store × ~5 instructions per iteration × 3,856,371 gas
≈ 15.4 TGas just for zero-filling
```

The fix in `deploy_direct` uses `vec![0u8; chunk_len]` per chunk + `extend_from_slice` instead of resizing a single large Vec. Each chunk's temp buffer is small (~300 KB) and gets dropped immediately.

**Full gas accounting for `deploy_direct` (94 TGas):**

| Component | Gas | Notes |
|-----------|-----|-------|
| 11 × `storage_read` base | 0.6 TGas | 56.4 Ggas × 11 |
| `storage_read` value bytes | 18.0 TGas | 5.6 Mgas × 3.2M bytes |
| 11 × `write_register` | 12.2 TGas | Writing chunk data to registers |
| 11 × `read_register` + `write_memory` | 9.5 TGas | Copying register data to WASM memory |
| `read_memory` for deploy | 12.2 TGas | Runtime reads 3.2 MB from WASM memory |
| Deploy action send fee | 22.0 TGas | 6.8 Mgas × 3.2M bytes |
| WASM instruction execution | ~15 TGas | Vec operations, logging, auth checks |
| State read, auth, misc | ~5 TGas | |
| **Total burnt** | **~94 TGas** | |
| Deploy action exec fee (reserved) | 22.0 TGas | Charged to used_gas, not burnt |

The deploy action execution (208 TGas) runs in a separate receipt and includes WASM precompilation.

### Gas parameters reference

Key `ExtCosts` values from nearcore 2.10.7 (`core/parameters/res/runtime_configs/parameters.yaml`):

| Parameter | Gas | Per |
|-----------|-----|-----|
| `storage_read_base` | 56,356,845,750 | per call |
| `storage_read_value_byte` | 5,611,005 | per byte |
| `storage_write_base` | 64,196,736,000 | per call |
| `storage_write_value_byte` | 31,018,539 | per byte |
| `read_memory_byte` | 3,801,333 | per byte |
| `read_register_byte` | 98,562 | per byte |
| `write_memory_byte` | 2,723,772 | per byte |
| `write_register_byte` | 3,801,564 | per byte |
| `regular_op_cost` | 3,856,371 | per WASM instruction |
| `grow_mem_cost` | 1 | per WASM page (64 KB) — essentially free |

Register limits: `max_register_size` = 100 MB, `registers_memory_limit` = 1 GB, `max_number_registers` = 100. Plenty of room for 3.2 MB.

### The trie proof limit: sandbox vs testnet

The `per_receipt_storage_proof_size_limit` was introduced at **protocol version 69** as part of NEAR's [stateless validation](https://near.org/blog/stateless-validation) changes. Every storage read or write in a receipt contributes to a merkle proof that validators use to verify execution without having the full state. The proof includes the actual values (not just hashes), so reading 3.2 MB of data means 3.2 MB of proof.

```yaml
# nearcore/core/parameters/res/runtime_configs/69.yaml
per_receipt_storage_proof_size_limit: {old: 4_294_967_295, new: 4_000_000}
```

The sandbox [overrides this to `usize::max_value()`](https://github.com/near/nearcore/blob/2.10.7/core/parameters/src/config_store.rs#L194) in its benchmarknet configuration, which is what sandbox uses. This means contracts that work on sandbox may fail on testnet/mainnet if they touch >4 MB of storage data in a single receipt. This is a significant divergence that isn't obvious.

### Protocol 83: gas limit increase

Protocol version 83 (nearcore 2.11.0-rc) raised the gas limits:

```yaml
# nearcore/core/parameters/res/runtime_configs/83.yaml
max_gas_burnt:         {old: 300_000_000_000_000, new: 1_000_000_000_000_000}
max_total_prepaid_gas: {old: 300_000_000_000_000, new: 1_000_000_000_000_000}
```

From 300 TGas to 1 PGas. This gave us much more breathing room for the deploy step, though `deploy_direct` at 94 TGas would have fit under the old limit too. The deploy action receipt at 208 TGas also fits under 300 TGas individually.

### Bulk-memory and WASM compatibility

Rust 1.87+ emits [bulk-memory WASM instructions](https://github.com/WebAssembly/bulk-memory-operations) (`memory.fill`, `memory.copy`) by default. NEAR's NearVM runtime (Wasmer singlepass on protocol <=82) **rejects these instructions**. The workaround:

1. Use **nightly Rust** with `-Zbuild-std=std,panic_abort` to rebuild the standard library
2. Set `-C target-cpu=mvp` to disable all post-MVP WASM features
3. Configure `getrandom_backend="custom"` since the standard `getrandom` crate doesn't support `wasm32-unknown-unknown`

The custom `getrandom` backend routes to NEAR's VRF-based `random_seed()` host function.

Protocol 83 introduces Wasmtime as an alternative VM which does support bulk-memory, but we target MVP for maximum compatibility.

## Monty

[Monty](https://github.com/pydantic/monty) is a Python implementation by [Pydantic](https://pydantic.dev) that compiles a subset of Python to bytecode and runs it on a Rust VM. It uses the [ruff](https://github.com/astral-sh/ruff) parser for Python syntax. Monty supports functions, classes, closures, recursion, basic data types (int, float, str, bytes, list, dict, set, tuple, bool, None), control flow, exceptions, and f-strings.

## License

MIT
