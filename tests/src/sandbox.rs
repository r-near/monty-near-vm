use base64::Engine as _;
use near_kit::*;
use near_kit::sandbox::{SandboxConfig, SANDBOX_ROOT_ACCOUNT, SANDBOX_ROOT_SECRET_KEY};
use tokio::sync::OnceCell;

const DEPLOYER_WASM: &[u8] =
    include_bytes!("../../deployer/target/near/monty_deployer.wasm");
const VM_WASM: &[u8] =
    include_bytes!("../../target/monty_near_vm_optimized.wasm");

/// Shared setup state — deploy once, test many times
static SETUP: OnceCell<(Near, String)> = OnceCell::const_new();

async fn get_setup() -> &'static (Near, String) {
    SETUP.get_or_init(|| async { setup().await }).await
}

/// Deploy the VM contract via the factory pattern:
/// 1. Deploy the tiny deployer contract to root account
/// 2. Send VM WASM in base64 chunks via store_chunk
/// 3. Call assemble() to combine chunks into single storage key
/// 4. Call deploy() to deploy VM using the register trick
async fn setup() -> (Near, String) {
    // Use SANDBOX_RPC env var if set, otherwise start a fresh sandbox
    let (_sandbox, near, root_id) = if let Ok(rpc_url) = std::env::var("SANDBOX_RPC") {
        println!("Using external sandbox at {rpc_url}");
        let near = Near::custom(&rpc_url)
            .credentials(SANDBOX_ROOT_SECRET_KEY, SANDBOX_ROOT_ACCOUNT)
            .unwrap()
            .build();
        (None, near, SANDBOX_ROOT_ACCOUNT.to_string())
    } else {
        let sandbox = SandboxConfig::fresh().await;
        let near = sandbox.client();
        let root_id = sandbox.root_account_id().to_string();
        (Some(sandbox), near, root_id)
    };

    // Deploy the deployer contract to root
    println!("Deploying deployer contract ({} bytes)...", DEPLOYER_WASM.len());
    near.deploy(DEPLOYER_WASM.to_vec()).wait_until(TxExecutionStatus::Final).send().await.unwrap();

    // Upload VM WASM in base64-encoded chunks.
    // 300KB raw = ~400KB base64 + JSON overhead, safely under RPC limits.
    let chunk_size = 300_000;
    let total = VM_WASM.len();
    println!("Uploading VM WASM ({total} bytes) in chunks...");

    let b64 = base64::engine::general_purpose::STANDARD;
    let mut offset = 0usize;
    while offset < total {
        let end = (offset + chunk_size).min(total);
        let chunk_b64 = b64.encode(&VM_WASM[offset..end]);
        println!("  store_chunk offset={offset} raw_size={} b64_size={}", end - offset, chunk_b64.len());

        let outcome = near.call(&root_id, "store_chunk")
            .args(serde_json::json!({
                "data_b64": chunk_b64,
            }))
            .gas(Gas::from_tgas(300))
            .wait_until(TxExecutionStatus::Final)
            .send()
            .await
            .unwrap();

        if outcome.is_failure() {
            panic!("store_chunk failed at offset {offset}: {:?}", outcome.status);
        }
        print_outcome("store_chunk", &outcome);

        offset = end;
    }

    // Verify stored size
    let stored: u64 = near.view(&root_id, "code_size").await.unwrap();
    println!("Stored code size: {stored} bytes");
    assert_eq!(stored as usize, total);

    // Assemble all chunks into a single "code" storage key
    println!("Assembling chunks into single storage key...");
    let assemble_outcome = near.call(&root_id, "assemble")
        .args(serde_json::json!({}))
        .gas(Gas::from_tgas(300))
        .wait_until(TxExecutionStatus::Final)
        .send()
        .await
        .unwrap();

    if assemble_outcome.is_failure() {
        print_outcome("assemble", &assemble_outcome);
        panic!("assemble failed: {:?}", assemble_outcome.status);
    }
    print_outcome("assemble", &assemble_outcome);

    // Deploy VM to the root account itself (replace deployer with VM contract)
    // This avoids sub-account creation and uses sir (same-id) action costs.
    println!("Deploying VM to {root_id} (self-deploy with register trick)...");
    let deploy_outcome = near.call(&root_id, "deploy")
        .args_raw(format!("\"{}\"", root_id).into_bytes())
        .gas(Gas::from_tgas(300))
        .wait_until(TxExecutionStatus::Final)
        .send()
        .await
        .unwrap();

    print_outcome("deploy", &deploy_outcome);
    if deploy_outcome.is_failure() {
        panic!("deploy failed: {:?}", deploy_outcome.status);
    }

    println!("VM contract deployed to {root_id}");
    (near, root_id)
}

/// Print gas usage and logs for a transaction outcome
fn print_outcome(label: &str, outcome: &FinalExecutionOutcome) {
    let tx_gas = outcome.transaction_outcome.outcome.gas_burnt.as_gas();
    println!("[{label}] tx gas_burnt: {tx_gas}");

    for (i, receipt) in outcome.receipts_outcome.iter().enumerate() {
        let gas = receipt.outcome.gas_burnt.as_gas();
        let tgas = gas as f64 / 1e12;
        println!("[{label}] receipt {i}: gas_burnt={gas} ({tgas:.1} TGas) status={:?}",
            receipt.outcome.status);
        for log_line in &receipt.outcome.logs {
            println!("[{label}]   log: {log_line}");
        }
    }
}

/// Helper: call execute with Python code and return the string result
async fn execute(near: &Near, contract_id: &str, code: &str) -> String {
    let outcome = near
        .call(contract_id, "execute")
        .args_raw(code.as_bytes().to_vec())
        .gas(Gas::from_tgas(300))
        .wait_until(TxExecutionStatus::Final)
        .send()
        .await
        .unwrap();

    let bytes = outcome.result().unwrap();
    String::from_utf8(bytes).unwrap()
}

#[tokio::test]
async fn test_hello_world() {
    let (near, contract_id) = get_setup().await;

    let result = execute(near, contract_id, r#"value_return("Hello from Monty VM on NEAR!")"#).await;
    println!("Result: {result}");
    assert_eq!(result, "Hello from Monty VM on NEAR!");
}

#[tokio::test]
async fn test_arithmetic() {
    let (near, contract_id) = get_setup().await;

    let result = execute(near, contract_id, "x = 21 * 2\nvalue_return(str(x))").await;
    println!("Result: {result}");
    assert_eq!(result, "42");
}

#[tokio::test]
async fn test_print_and_return() {
    let (near, contract_id) = get_setup().await;

    let code = r#"
print("hello from python on NEAR")
value_return("done")
"#;
    let result = execute(near, contract_id, code).await;
    assert_eq!(result, "done");
}

#[tokio::test]
async fn test_storage() {
    let (near, contract_id) = get_setup().await;

    let code = r#"
storage_write("mykey", "myvalue")
val = storage_read("mykey")
value_return(val)
"#;
    let result = execute(near, contract_id, code).await;
    println!("Result: {result}");
    assert_eq!(result, "myvalue");
}

#[tokio::test]
async fn test_sha256() {
    let (near, contract_id) = get_setup().await;

    let result = execute(near, contract_id, r#"value_return(sha256("hello"))"#).await;
    println!("Result: {result}");
    assert_eq!(result, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
}

#[tokio::test]
async fn test_context() {
    let (near, contract_id) = get_setup().await;

    let code = r#"
acct = current_account_id()
height = block_height()
value_return(acct + " at block " + str(height))
"#;
    let result = execute(near, contract_id, code).await;
    println!("Result: {result}");
    assert!(result.contains(contract_id));
    assert!(result.contains("at block"));
}

#[tokio::test]
async fn test_counter() {
    let (near, contract_id) = get_setup().await;

    let code = r#"
val = storage_read("counter")
if val is None:
    count = 0
else:
    count = int(val)
count = count + 1
storage_write("counter", str(count))
value_return(str(count))
"#;

    let r1 = execute(near, contract_id, code).await;
    println!("Counter call 1: {r1}");
    // Counter starts at whatever value it's at (shared state)
    let c1: i64 = r1.parse().unwrap();
    assert!(c1 >= 1);

    let r2 = execute(near, contract_id, code).await;
    println!("Counter call 2: {r2}");
    let c2: i64 = r2.parse().unwrap();
    assert_eq!(c2, c1 + 1);
}

#[tokio::test]
async fn test_fibonacci() {
    let (near, contract_id) = get_setup().await;

    let code = r#"
def fib(n):
    if n <= 1:
        return n
    return fib(n - 1) + fib(n - 2)

value_return(str(fib(10)))
"#;
    let result = execute(near, contract_id, code).await;
    println!("Result: {result}");
    assert_eq!(result, "55");
}
