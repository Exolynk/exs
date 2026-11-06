//! A Rust guest that exposes multiple asynchronous functions through the ExS runner ABI.

use std::time::Duration;
use std::{format, vec, vec::Vec};

use exs_guest::{ExsError, ExsValue, decode, encode, host};
use serde::{Deserialize, Serialize};

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

/// Sends its inputs to the `echo` host function and returns them with the host's response,
/// so every value kind crosses the boundary in both directions.
async fn round_trip(inputs: Vec<ExsValue>) -> ExsValue {
    let response = host::call("echo", [ExsValue::List(inputs.clone())]).await;
    ExsValue::List(vec![ExsValue::List(inputs), response])
}

/// Opens the example counter stream and sums every integer it yields.
async fn sum_stream(_inputs: Vec<ExsValue>) -> Result<ExsValue, ExsError> {
    let mut stream = host::stream("counter", [ExsValue::Int(3)]).await?;
    let mut total = 0_i64;
    loop {
        match stream.next().await? {
            host::IteratorStep::Item(ExsValue::Int(value)) => total += value,
            host::IteratorStep::Item(_) => {}
            host::IteratorStep::Done => return Ok(ExsValue::Int(total)),
        }
    }
}

/// Sleeps for the given number of milliseconds and returns how long the execution has run, in
/// milliseconds.
async fn pause(inputs: Vec<ExsValue>) -> Result<ExsValue, ExsError> {
    let milliseconds = match inputs.first() {
        Some(ExsValue::Int(value)) => u64::try_from(*value).unwrap_or(0),
        _ => 0,
    };
    host::sleep(Duration::from_millis(milliseconds)).await?;
    let elapsed = host::elapsed().await?;
    Ok(ExsValue::Int(
        i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX),
    ))
}

/// One line of an order, as a script or host passes it.
#[derive(Deserialize)]
struct Line {
    /// Price of one unit.
    price: i64,
    /// Number of units.
    quantity: i64,
}

/// The total of an order.
#[derive(Serialize)]
struct Total {
    /// Sum of all lines.
    amount: i64,
}

/// Decodes a list of order lines and returns their typed total.
async fn total(inputs: Vec<ExsValue>) -> Result<ExsValue, ExsError> {
    let lines: Vec<Line> = decode(inputs.into_iter().next().unwrap_or(ExsValue::None))?;
    let amount = lines.iter().map(|line| line.price * line.quantity).sum();
    encode(&Total { amount })
}

/// Calls the time operations of the runner and returns two UTC offsets and whether the clock
/// reads a time after the epoch.
async fn clock(_inputs: Vec<ExsValue>) -> Result<ExsValue, ExsError> {
    let components = host::DateTimeComponents {
        year: 2026,
        month: 7,
        day: 1,
        hour: 12,
        ..host::DateTimeComponents::default()
    };
    let zurich = host::from_components_in_timezone(components, "Europe/Zurich").await?;
    let tokyo = host::in_timezone(&zurich, "Asia/Tokyo").await?;
    let now = host::now().await?;
    Ok(ExsValue::List(vec![
        ExsValue::Int(i64::from(zurich.utc_offset_seconds)),
        ExsValue::Int(i64::from(tokyo.utc_offset_seconds)),
        ExsValue::Bool(now.unix_seconds > 0),
    ]))
}

exs_guest::export!(main, add, round_trip, sum_stream, pause, total, clock);
