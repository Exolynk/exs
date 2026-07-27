//! Rust implementations exported by the Wasm-target runtime.

use core::panic::PanicInfo;

use exs_value::Value;

/// Adds two runtime values through the runtime's numeric dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_add(left: Value, right: Value) -> Value {
    integer_binary(left, right, i64::checked_add)
}

/// Subtracts two runtime values through the runtime's numeric dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_sub(left: Value, right: Value) -> Value {
    integer_binary(left, right, i64::checked_sub)
}

/// Multiplies two runtime values through the runtime's numeric dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_mul(left: Value, right: Value) -> Value {
    integer_binary(left, right, i64::checked_mul)
}

/// Negates a runtime numeric value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_neg(value: Value) -> Value {
    encode(integer(value).checked_neg().and_then(Value::int))
}

/// Tests two runtime values for equality.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_eq(left: Value, right: Value) -> Value {
    Value::bool(left == right)
}

/// Tests two runtime values for inequality.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_ne(left: Value, right: Value) -> Value {
    Value::bool(left != right)
}

/// Compares two runtime values using the runtime's ordering dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_lt(left: Value, right: Value) -> Value {
    compare(left, right, Ordering::Less)
}

/// Compares two runtime values using the runtime's ordering dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_le(left: Value, right: Value) -> Value {
    compare(left, right, Ordering::LessOrEqual)
}

/// Compares two runtime values using the runtime's ordering dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_gt(left: Value, right: Value) -> Value {
    compare(left, right, Ordering::Greater)
}

/// Compares two runtime values using the runtime's ordering dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_ge(left: Value, right: Value) -> Value {
    compare(left, right, Ordering::GreaterOrEqual)
}

/// Negates a runtime boolean value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_not(value: Value) -> Value {
    Value::bool(!boolean(value))
}

/// Converts a runtime boolean value to a Wasm condition.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_condition(value: Value) -> i32 {
    i32::from(boolean(value))
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    trap()
}

enum Ordering {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

fn integer_binary(left: Value, right: Value, operation: fn(i64, i64) -> Option<i64>) -> Value {
    encode(operation(integer(left), integer(right)).and_then(Value::int))
}

fn compare(left: Value, right: Value, ordering: Ordering) -> Value {
    let left = integer(left);
    let right = integer(right);
    let result = match ordering {
        Ordering::Less => left < right,
        Ordering::LessOrEqual => left <= right,
        Ordering::Greater => left > right,
        Ordering::GreaterOrEqual => left >= right,
    };
    Value::bool(result)
}

fn integer(value: Value) -> i64 {
    match value.as_int() {
        Some(value) => value,
        None => trap(),
    }
}

fn boolean(value: Value) -> bool {
    match value.as_bool() {
        Some(value) => value,
        None => trap(),
    }
}

fn encode(value: Option<Value>) -> Value {
    match value {
        Some(value) => value,
        None => trap(),
    }
}

fn trap() -> ! {
    core::arch::wasm32::unreachable()
}
