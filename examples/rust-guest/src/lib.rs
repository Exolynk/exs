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

/// Opens the example counter stream and sums every integer it yields.
async fn sum_stream(_inputs: Vec<ExsValue>) -> ExsValue {
    let mut stream = match host::stream("counter", [ExsValue::Int(3)]).await {
        Ok(stream) => stream,
        Err(error) => return error,
    };
    let mut total = 0_i64;
    loop {
        match stream.next().await {
            Ok(host::IteratorStep::Item(ExsValue::Int(value))) => total += value,
            Ok(host::IteratorStep::Item(_)) => {}
            Ok(host::IteratorStep::Done) => return ExsValue::Int(total),
            Err(error) => return error,
        }
    }
}

exs_guest::export!(main, add, sum_stream);
