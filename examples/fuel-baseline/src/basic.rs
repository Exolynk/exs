//! Rust counterparts of the small ExS fuel scenarios.

use std::hint::black_box;

use exs_guest::{ExsValue, Vec};

/// Returns the first input as an Int, or 0.
fn limit(inputs: &[ExsValue]) -> i64 {
    match inputs.first() {
        Some(ExsValue::Int(value)) => *value,
        _ => 0,
    }
}

/// Returns one field of a host Object.
pub(crate) fn field<'a>(value: &'a ExsValue, key: &str) -> Option<&'a ExsValue> {
    match value {
        ExsValue::Object(entries) => entries
            .iter()
            .find_map(|(entry_key, value)| (entry_key == key).then_some(value)),
        _ => None,
    }
}

/// Returns immediately, measuring the fixed ABI and runtime overhead.
pub(crate) async fn empty_main(_inputs: Vec<ExsValue>) -> ExsValue {
    ExsValue::None
}

/// Returns `true` like a trivial show function.
pub(crate) async fn show_true(_inputs: Vec<ExsValue>) -> ExsValue {
    ExsValue::Bool(true)
}

/// Compares a String and an Int field of a host-supplied record.
pub(crate) async fn show_compare_record(inputs: Vec<ExsValue>) -> ExsValue {
    let record = inputs.first().unwrap_or(&ExsValue::None);
    let active = matches!(field(record, "name"), Some(ExsValue::String(name)) if name == "active");
    let large = matches!(field(record, "count"), Some(ExsValue::Int(count)) if *count > 10);
    ExsValue::Bool(active && large)
}

/// Concatenates the `name` field of every supplied object.
fn concat(inputs: &[ExsValue]) -> ExsValue {
    let mut output = String::new();
    if let Some(ExsValue::List(items)) = inputs.first() {
        for item in items {
            if let Some(ExsValue::String(name)) = field(item, "name") {
                output.push_str(name);
            }
            output.push(';');
        }
    }
    ExsValue::String(output)
}

/// Concatenates the names of 10 objects.
pub(crate) async fn concat_10_objects(inputs: Vec<ExsValue>) -> ExsValue {
    concat(&inputs)
}

/// Concatenates the names of 100 objects.
pub(crate) async fn concat_100_objects(inputs: Vec<ExsValue>) -> ExsValue {
    concat(&inputs)
}

/// Sums `index * 3 + 1` over a counted loop.
fn int_loop(limit: i64) -> i64 {
    let mut total = 0_i64;
    let mut index = 0_i64;
    while index < black_box(limit) {
        total = black_box(total + index * 3 + 1);
        index += 1;
    }
    total
}

/// Runs the integer arithmetic loop 1000 times.
pub(crate) async fn int_loop_1000(inputs: Vec<ExsValue>) -> ExsValue {
    ExsValue::Int(int_loop(limit(&inputs)))
}

/// Runs the guarded arithmetic `for` loop.
pub(crate) async fn async_int_loop_100(inputs: Vec<ExsValue>) -> ExsValue {
    let limit = limit(&inputs);
    let mut total = 0_i64;
    for index in 0..black_box(limit) {
        if index < limit {
            total = black_box(total + index * 3 + 1);
        }
    }
    ExsValue::Int(total)
}

/// Counts an empty `while` loop up to the limit.
///
/// The ExS scenario starts with a host call only to select resumable lowering; Rust has no
/// such distinction, so the baseline skips it.
pub(crate) async fn async_empty_while_1000(inputs: Vec<ExsValue>) -> ExsValue {
    let limit = limit(&inputs);
    let mut index = 0_i64;
    while index < limit {
        index = black_box(index + 1);
    }
    ExsValue::Int(index)
}

/// Runs the integer arithmetic loop, the ExS counterpart being resumable.
pub(crate) async fn async_int_while_1000(inputs: Vec<ExsValue>) -> ExsValue {
    ExsValue::Int(int_loop(limit(&inputs)))
}

/// Iterates an empty `for` loop over a range.
pub(crate) async fn for_range_1000(inputs: Vec<ExsValue>) -> ExsValue {
    for index in 0..black_box(limit(&inputs)) {
        black_box(index);
    }
    ExsValue::Int(0)
}

/// Iterates an empty `for` loop over a range held in a variable and returns its final cursor.
pub(crate) async fn for_range_variable_1000(inputs: Vec<ExsValue>) -> ExsValue {
    let mut range = 0..black_box(limit(&inputs));
    for index in range.by_ref() {
        black_box(index);
    }
    ExsValue::Int(range.start)
}

/// Adds one without being inlined, like a separate ExS function call.
#[inline(never)]
fn bump(value: i64) -> i64 {
    black_box(value) + 1
}

/// Calls a small function once per iteration.
fn call_loop(limit: i64) -> i64 {
    let mut total = 0_i64;
    let mut index = 0_i64;
    while index < black_box(limit) {
        total = bump(total);
        index += 1;
    }
    total
}

/// Calls a small function 100 times, the ExS counterpart containing a `match`.
pub(crate) async fn async_call_loop_100(inputs: Vec<ExsValue>) -> ExsValue {
    ExsValue::Int(call_loop(limit(&inputs)))
}

/// Calls a small function 100 times, the ExS counterpart being resumable.
pub(crate) async fn resumable_call_loop_100(inputs: Vec<ExsValue>) -> ExsValue {
    ExsValue::Int(call_loop(limit(&inputs)))
}

/// Calls a small function 1000 times.
pub(crate) async fn sync_call_loop_1000(inputs: Vec<ExsValue>) -> ExsValue {
    ExsValue::Int(call_loop(limit(&inputs)))
}

/// Formats one line with two Ints per iteration and sums the line lengths.
pub(crate) async fn format_loop_1000(inputs: Vec<ExsValue>) -> ExsValue {
    let limit = limit(&inputs);
    let mut length = 0_i64;
    let mut index = 0_i64;
    while index < limit {
        let line = black_box(format!("row {index}: {};", index * 2));
        length += line.len() as i64;
        index += 1;
    }
    ExsValue::Int(length)
}

/// Allocates one short-lived two-element vector per iteration.
pub(crate) async fn list_churn_6000(inputs: Vec<ExsValue>) -> ExsValue {
    let limit = limit(&inputs);
    let mut total = 0_i64;
    let mut index = 0_i64;
    while index < limit {
        let pair = black_box(vec![index, index]);
        total += pair[0];
        index += 1;
    }
    ExsValue::Int(total)
}
