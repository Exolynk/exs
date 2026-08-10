//! A minimal asynchronous Rust guest executed through the ExS runner ABI.

use std::{format, vec::Vec};

use exs_guest::{ExsValue, host};

/// Allocates host-call arguments before sending them to the CLI output host.
async fn main(inputs: Vec<ExsValue>) -> ExsValue {
    let message = format!("Rust allocated {} host arguments", inputs.len());
    host::call("println", [ExsValue::String(message)]).await;

    let mut value = match inputs.first() {
        Some(ExsValue::Int(v)) => *v,
        _ => 0,
    };

    value += 8;
    ExsValue::Int(value)
}

exs_guest::export!(main);
