use base64::Engine as _;
use near_kit::*;
use near_kit::sandbox::{SandboxConfig, SANDBOX_ROOT_ACCOUNT, SANDBOX_ROOT_SECRET_KEY};

const DEPLOYER_WASM: &[u8] =
    include_bytes!("../../deployer/target/near/monty_deployer.wasm");
const VM_WASM: &[u8] =
    include_bytes!("../../target/monty_near_vm_optimized.wasm");

#[tokio::main]
async fn main() {
    let rpc_url = std::env::var("SANDBOX_RPC").expect("Set SANDBOX_RPC=http://localhost:3030");
    println!("Using sandbox at {rpc_url}");

    let near = Near::custom(&rpc_url)
        .credentials(SANDBOX_ROOT_SECRET_KEY, SANDBOX_ROOT_ACCOUNT)
        .unwrap()
        .build();

    let account = SANDBOX_ROOT_ACCOUNT;

    // Deploy deployer
    println!("Deploying deployer ({} bytes)...", DEPLOYER_WASM.len());
    near.deploy(DEPLOYER_WASM.to_vec())
        .wait_until(TxExecutionStatus::Final)
        .send().await.unwrap();

    // Upload chunks
    let chunk_size = 300_000;
    let total = VM_WASM.len();
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut offset = 0usize;

    while offset < total {
        let end = (offset + chunk_size).min(total);
        let chunk_b64 = b64.encode(&VM_WASM[offset..end]);
        let outcome = near.call(account, "store_chunk")
            .args(serde_json::json!({ "data_b64": chunk_b64 }))
            .gas(Gas::from_tgas(300))
            .wait_until(TxExecutionStatus::Final)
            .send().await.unwrap();
        assert!(!outcome.is_failure(), "store_chunk failed");
        offset = end;
    }
    println!("Uploaded {total} bytes in chunks");

    // Verify
    let stored: u64 = near.view(account, "code_size").await.unwrap();
    assert_eq!(stored as usize, total);
    println!("Verified: {stored} bytes stored");

    // Deploy directly (no assemble step)
    println!("Calling deploy_direct...");
    let outcome = near.call(account, "deploy_direct")
        .args_raw(format!("\"{account}\"").into_bytes())
        .gas(Gas::from_gas(1_000_000_000_000_000))
        .wait_until(TxExecutionStatus::Final)
        .send().await.unwrap();

    for (i, receipt) in outcome.receipts_outcome.iter().enumerate() {
        let gas = receipt.outcome.gas_burnt.as_gas();
        println!("  receipt {i}: {:.1} TGas status={:?}", gas as f64 / 1e12, receipt.outcome.status);
        for log_line in &receipt.outcome.logs {
            println!("    log: {log_line}");
        }
    }

    if outcome.is_failure() {
        panic!("deploy_direct failed!");
    }

    // Test execute
    println!("\nTesting execute...");
    let outcome = near.call(account, "execute")
        .args_raw(b"value_return(\"hello from deploy_direct!\")".to_vec())
        .gas(Gas::from_tgas(300))
        .wait_until(TxExecutionStatus::Final)
        .send().await.unwrap();

    let result = String::from_utf8(outcome.result().unwrap()).unwrap();
    println!("Result: {result}");
    assert_eq!(result, "hello from deploy_direct!");
    println!("\nSUCCESS!");
}
