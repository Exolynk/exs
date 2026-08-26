//! A Rust guest that exposes multiple asynchronous functions through the ExS runner ABI.

use std::{format, vec::Vec};

use exs_guest::{ExsValue, host};

/// Reports the supplied input count through the CLI output host.
async fn main(inputs: Vec<ExsValue>) -> ExsValue {
    let message = format!("Rust allocated {} host arguments", inputs.len());
    host::call("println", [ExsValue::String(message)]).await;

    add(inputs).await
}

/// Adds eight to the first integer input and is callable independently through the runner.
async fn add(inputs: Vec<ExsValue>) -> ExsValue {
    let value = match inputs.first() {
        Some(ExsValue::Int(v)) => *v,
        _ => 0,
    };

    ExsValue::Int(value + 8)
}

exs_guest::export!(main, add);
