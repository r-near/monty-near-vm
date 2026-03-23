use near_kit::*;

const ACCOUNT: &str = "monty-vm.testnet";

#[tokio::main]
async fn main() {
    let signer = KeyringSigner::new(
        "testnet", ACCOUNT,
        "ed25519:2NKDS8EphBUHzkigcNRLNsyXLH5y6HUJnaJf6c694R7X",
    ).unwrap();
    let near = Near::testnet().signer(signer).build();

    let code = std::env::args().nth(1).expect("Usage: run-testnet '<python code>'");

    println!("Executing on {ACCOUNT}...\n");
    let outcome = near.call(ACCOUNT, "execute")
        .args_raw(code.as_bytes().to_vec())
        .gas(Gas::from_tgas(300))
        .wait_until(TxExecutionStatus::Final)
        .send().await.unwrap();

    let tx_hash = &outcome.transaction_outcome.id;
    println!("TX: https://testnet.nearblocks.io/txns/{tx_hash}\n");

    for (i, receipt) in outcome.receipts_outcome.iter().enumerate() {
        let gas = receipt.outcome.gas_burnt.as_gas();
        println!("Receipt {i}: {:.1} TGas", gas as f64 / 1e12);
        for log_line in &receipt.outcome.logs {
            println!("  log: {log_line}");
        }
    }

    let result = String::from_utf8(outcome.result().unwrap()).unwrap();
    println!("\nReturn value: {result}");
}
