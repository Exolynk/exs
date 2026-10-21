//! Static functions of built-in types: `Int::parse`, `Float::parse` and `Math`.

use core::str::FromStr;

use exs_layout::StaticFunction;

use crate::errors::recoverable;
use crate::memory::Memory;
use crate::methods::float_parse::parse_f64;
use crate::methods::numbers::{float_pair, float_unary, type_error};
use crate::numbers::{float_value, int_value};
use crate::strings::{string_object, text};

/// Calls one static function with its arguments `first` and `second`, which must be rooted.
///
/// Returns `None` when memory is exhausted.
pub fn call_static(
    memory: &mut impl Memory,
    vm: u32,
    function: StaticFunction,
    first: u64,
    second: u64,
) -> Option<u64> {
    let unary = |memory: &mut _, operation: fn(f64) -> f64, message| {
        float_unary(memory, vm, first, operation, message)
    };
    match function {
        StaticFunction::IntParse => parse_int(memory, vm, first),
        StaticFunction::FloatParse => parse_float(memory, vm, first),
        StaticFunction::MathPi => float_value(memory, vm, core::f64::consts::PI),
        StaticFunction::MathTau => float_value(memory, vm, core::f64::consts::TAU),
        StaticFunction::MathE => float_value(memory, vm, core::f64::consts::E),
        StaticFunction::MathSin => unary(memory, libm::sin, "Math::sin requires a Float argument"),
        StaticFunction::MathCos => unary(memory, libm::cos, "Math::cos requires a Float argument"),
        StaticFunction::MathTan => unary(memory, libm::tan, "Math::tan requires a Float argument"),
        StaticFunction::MathAsin => {
            unary(memory, libm::asin, "Math::asin requires a Float argument")
        }
        StaticFunction::MathAcos => {
            unary(memory, libm::acos, "Math::acos requires a Float argument")
        }
        StaticFunction::MathAtan => {
            unary(memory, libm::atan, "Math::atan requires a Float argument")
        }
        StaticFunction::MathExp => unary(memory, libm::exp, "Math::exp requires a Float argument"),
        StaticFunction::MathLn => unary(memory, libm::log, "Math::ln requires a Float argument"),
        StaticFunction::MathLog2 => {
            unary(memory, libm::log2, "Math::log2 requires a Float argument")
        }
        StaticFunction::MathLog10 => {
            unary(memory, libm::log10, "Math::log10 requires a Float argument")
        }
        // v1 passes the whole sentence as the name of its pair helper, which then appends its
        // receiver and argument wording; the messages keep that text.
        StaticFunction::MathAtan2 => float_pair(
            memory,
            vm,
            first,
            second,
            libm::atan2,
            [
                "Math::atan2 requires Float arguments requires a Float receiver",
                "Math::atan2 requires Float arguments requires a Float argument",
            ],
        ),
        StaticFunction::MathHypot => float_pair(
            memory,
            vm,
            first,
            second,
            libm::hypot,
            [
                "Math::hypot requires Float arguments requires a Float receiver",
                "Math::hypot requires Float arguments requires a Float argument",
            ],
        ),
    }
}

/// Evaluates `Int::parse(text)` with Rust's base-10 `i64` syntax, like v1.
fn parse_int(memory: &mut impl Memory, vm: u32, value: u64) -> Option<u64> {
    let Some(object) = string_object(value) else {
        return type_error(memory, vm, "Int::parse requires a String value", value);
    };
    match i64::from_str(text(memory, object)) {
        Ok(parsed) => int_value(memory, vm, parsed),
        Err(_) => recoverable(
            memory,
            vm,
            "ParseError",
            "Int::parse requires a base-10 signed 64-bit integer",
            value,
        ),
    }
}

/// Evaluates `Float::parse(text)` with Rust's `f64` syntax and results, like v1.
fn parse_float(memory: &mut impl Memory, vm: u32, value: u64) -> Option<u64> {
    let Some(object) = string_object(value) else {
        return type_error(memory, vm, "Float::parse requires a String value", value);
    };
    match parse_f64(text(memory, object).as_bytes()).ok_or(()) {
        Ok(parsed) => float_value(memory, vm, parsed),
        Err(_) => recoverable(
            memory,
            vm,
            "ParseError",
            "Float::parse requires a decimal floating-point value",
            value,
        ),
    }
}
