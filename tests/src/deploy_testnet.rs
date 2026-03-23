use base64::Engine as _;
use near_kit::*;

const DEPLOYER_WASM: &[u8] =
    include_bytes!("../../deployer/target/near/monty_deployer.wasm");
const VM_WASM: &[u8] =
    include_bytes!("../../target/monty_near_vm_optimized.wasm");

const ACCOUNT: &str = "monty-vm.testnet";

#[tokio::main]
async fn main() {
    let signer = KeyringSigner::new(
        "testnet",
        ACCOUNT,
        "ed25519:2NKDS8EphBUHzkigcNRLNsyXLH5y6HUJnaJf6c694R7X",
    ).unwrap();
    let near = Near::testnet().signer(signer).build();

    println!("=== Deploying Monty VM to {ACCOUNT} ===");
    println!("Deployer WASM: {} bytes", DEPLOYER_WASM.len());
    println!("VM WASM: {} bytes", VM_WASM.len());

    // Step 1: Deploy deployer contract
    println!("\n--- Step 1: Deploy deployer contract ---");
    let outcome = near.deploy(DEPLOYER_WASM.to_vec())
        .wait_until(TxExecutionStatus::Final)
        .send().await.unwrap();
    print_outcome("deploy-deployer", &outcome);

    // Step 1b: Reset state (in case of re-run)
    println!("\n--- Step 1b: Reset deployer state ---");
    let outcome = near.call(ACCOUNT, "reset")
        .args(serde_json::json!({}))
        .gas(Gas::from_tgas(300))
        .wait_until(TxExecutionStatus::Final)
        .send().await.unwrap();
    print_outcome("reset", &outcome);

    // Step 2: Upload VM WASM in chunks
    println!("\n--- Step 2: Upload VM WASM chunks ---");
    let chunk_size = 300_000;
    let total = VM_WASM.len();
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut offset = 0usize;

    while offset < total {
        let end = (offset + chunk_size).min(total);
        let chunk_b64 = b64.encode(&VM_WASM[offset..end]);
        println!("  Chunk: offset={offset} raw={} b64={}", end - offset, chunk_b64.len());

        let outcome = near.call(ACCOUNT, "store_chunk")
            .args(serde_json::json!({ "data_b64": chunk_b64 }))
            .gas(Gas::from_tgas(300))
            .wait_until(TxExecutionStatus::Final)
            .send().await.unwrap();

        if outcome.is_failure() {
            print_outcome("store_chunk", &outcome);
            panic!("store_chunk failed");
        }

        let gas = outcome.receipts_outcome[0].outcome.gas_burnt.as_gas();
        println!("    gas: {:.1} TGas", gas as f64 / 1e12);

        offset = end;
    }

    // Step 3: Verify
    println!("\n--- Step 3: Verify stored size ---");
    let stored: u64 = near.view(ACCOUNT, "code_size").await.unwrap();
    println!("Stored: {stored} bytes (expected {total})");
    assert_eq!(stored as usize, total);

    // Step 4: Deploy VM directly (skip assemble — reads chunks in WASM memory)
    // Uses 1 PGas (protocol 83 raised limit from 300 TGas)
    println!("\n--- Step 4: Deploy VM directly (chunks → WASM memory → deploy) ---");
    let outcome = near.call(ACCOUNT, "deploy_direct")
        .args_raw(format!("\"{ACCOUNT}\"").into_bytes())
        .gas(Gas::from_gas(1_000_000_000_000_000)) // 1 PGas
        .wait_until(TxExecutionStatus::Final)
        .send().await.unwrap();
    print_outcome("deploy-vm", &outcome);
    if outcome.is_failure() {
        panic!("deploy failed");
    }

    // Step 5: Test
    println!("\n--- Step 5: Test execute ---");
    let outcome = near.call(ACCOUNT, "execute")
        .args_raw(b"value_return(\"Hello from Monty Python VM on NEAR testnet!\")".to_vec())
        .gas(Gas::from_tgas(300))
        .wait_until(TxExecutionStatus::Final)
        .send().await.unwrap();
    let result = String::from_utf8(outcome.result().unwrap()).unwrap();
    println!("Result: {result}");

    println!("\n=== Done! VM live at {ACCOUNT} ===");
}

fn print_outcome(label: &str, outcome: &FinalExecutionOutcome) {
    for (i, receipt) in outcome.receipts_outcome.iter().enumerate() {
        let gas = receipt.outcome.gas_burnt.as_gas();
        let tgas = gas as f64 / 1e12;
        println!("[{label}] receipt {i}: {tgas:.1} TGas status={:?}", receipt.outcome.status);
        for log_line in &receipt.outcome.logs {
            println!("[{label}]   log: {log_line}");
        }
    }
}
