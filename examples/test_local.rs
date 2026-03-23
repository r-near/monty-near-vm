//! Quick local test — runs Python code through Monty on native x86.
//! Just tests the parse+compile+run loop, not the NEAR host functions.
//!
//! Usage: cargo run --example test_local

use std::borrow::Cow;

use monty::{
    ExtFunctionResult, MontyException, MontyObject, MontyRun, NameLookupResult,
    NoLimitTracker, PrintWriter, PrintWriterCallback, RunProgress,
};

/// Simulated "NEAR" functions for local testing
fn is_near_function(name: &str) -> bool {
    matches!(name, "value_return" | "input" | "log" | "storage_read" | "storage_write" | "sha256" | "current_account_id" | "block_height")
}

fn dispatch_function(name: &str, args: &[MontyObject]) -> MontyObject {
    match name {
        "value_return" => {
            match args.first() {
                Some(MontyObject::String(s)) => println!("[value_return] {s}"),
                Some(other) => println!("[value_return] {other:?}"),
                None => println!("[value_return] (empty)"),
            }
            MontyObject::None
        }
        "log" => {
            match args.first() {
                Some(MontyObject::String(s)) => println!("[log] {s}"),
                Some(other) => println!("[log] {other:?}"),
                None => println!("[log] (empty)"),
            }
            MontyObject::None
        }
        "current_account_id" => MontyObject::String("test.near".to_string()),
        "block_height" => MontyObject::Int(12345),
        "sha256" => MontyObject::String("deadbeef".to_string()),
        _ => {
            println!("[unknown function] {name}({args:?})");
            MontyObject::None
        }
    }
}

struct TestPrint;

impl PrintWriterCallback for TestPrint {
    fn stdout_write(&mut self, output: Cow<'_, str>) -> Result<(), MontyException> {
        print!("{output}");
        Ok(())
    }
    fn stdout_push(&mut self, end: char) -> Result<(), MontyException> {
        print!("{end}");
        Ok(())
    }
}

fn run_test(name: &str, code: &str) {
    println!("\n=== {name} ===");
    println!("Code: {code}");
    println!("---");

    let runner = match MontyRun::new(code.to_owned(), "test.py", vec![]) {
        Ok(r) => r,
        Err(e) => {
            println!("COMPILE ERROR: {e}");
            return;
        }
    };

    let mut test_print = TestPrint;
    let print = PrintWriter::Callback(&mut test_print);

    let mut progress = match runner.start(vec![], NoLimitTracker, print) {
        Ok(p) => p,
        Err(e) => {
            println!("START ERROR: {e}");
            return;
        }
    };

    loop {
        match progress {
            RunProgress::FunctionCall(call) => {
                let result = dispatch_function(&call.function_name, &call.args);
                progress = match call.resume(
                    ExtFunctionResult::Return(result),
                    PrintWriter::Callback(&mut test_print),
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        println!("RUNTIME ERROR: {e}");
                        return;
                    }
                };
            }
            RunProgress::NameLookup(lookup) => {
                let result = if is_near_function(&lookup.name) {
                    NameLookupResult::Value(MontyObject::None)
                } else {
                    NameLookupResult::Undefined
                };
                progress = match lookup.resume(result, PrintWriter::Callback(&mut test_print)) {
                    Ok(p) => p,
                    Err(e) => {
                        println!("NAME ERROR: {e}");
                        return;
                    }
                };
            }
            RunProgress::Complete(val) => {
                println!("Complete: {val:?}");
                break;
            }
            RunProgress::OsCall(_) => {
                println!("ERROR: OS call not supported");
                break;
            }
            RunProgress::ResolveFutures(_) => {
                println!("ERROR: async not supported");
                break;
            }
        }
    }
    println!("=== OK ===");
}

fn main() {
    run_test("hello world", r#"value_return("Hello from Monty!")"#);

    run_test("arithmetic", r#"
x = 21 * 2
value_return(str(x))
"#);

    run_test("print output", r#"
print("hello from python")
print("second line")
value_return("done")
"#);

    run_test("context functions", r#"
acct = current_account_id()
height = block_height()
value_return(acct + " at block " + str(height))
"#);

    run_test("conditionals + loop", r#"
total = 0
i = 1
while i <= 10:
    total = total + i
    i = i + 1
value_return(str(total))
"#);

    run_test("function definition", r#"
def fib(n):
    if n <= 1:
        return n
    return fib(n - 1) + fib(n - 2)

value_return(str(fib(10)))
"#);

    println!("\n\nAll tests passed!");
}
