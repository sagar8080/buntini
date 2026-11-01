//! Explicit opt-in JSONL bridge for namespaced real-app acceptance, debug only.
#[cfg(debug_assertions)]
fn main() {
    use serde_json::{json, Value};
    use std::io::{self, BufRead};
    let runtime = tokio::runtime::Runtime::new().expect("Acceptance runtime");
    for line in io::stdin().lock().lines() {
        let result = (|| -> Result<Value, String> {
            let raw = line.map_err(|error| error.to_string())?;
            if raw.len() > 131_072 {
                return Err("Acceptance request exceeds its limit.".into());
            }
            let request: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
            let namespace = request["namespace"].as_str().ok_or("Missing namespace")?;
            Ok(match request["operation"].as_str() {
                Some("execute") => runtime.block_on(buntini_lib::acceptance_apps::execute_exact(
                    namespace,
                    request["tool"].as_str().ok_or("Missing tool")?,
                    &request["proposed"],
                    &request["expected"],
                )),
                Some("cleanup") => runtime.block_on(buntini_lib::acceptance_apps::cleanup_receipt(
                    namespace,
                    &request["receipt"],
                    &request["expected"],
                )),
                Some("corpus") => buntini_lib::acceptance_apps::corpus(namespace),
                Some("read_notes") => {
                    runtime.block_on(buntini_lib::acceptance_apps::read_notes_exact(
                        namespace,
                        request["query"].as_str().ok_or("Missing query")?,
                    ))
                }
                _ => return Err("Expected execute, cleanup, read_notes or corpus.".into()),
            })
        })()
        .unwrap_or_else(|error| json!({"passed":false,"error":error}));
        println!("{}", result);
    }
}

#[cfg(not(debug_assertions))]
fn main() {
    eprintln!("Real-app acceptance is available only in debug builds.");
    std::process::exit(2);
}
