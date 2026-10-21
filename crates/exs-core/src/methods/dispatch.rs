//! Dispatch of built-in receiver methods by their `exs_abi::BuiltinMethod` index.

use exs_abi::BuiltinMethod;

use crate::memory::Memory;
use crate::methods::numbers::{
    abs, boolean, clamp, euclidean, float_predicate, float_unary, gcd_or_lcm, int_unary,
    min_or_max, pow, signum, to_float, to_int,
};
use crate::methods::values::{ErrorPart, error_part, is_empty, length, operator, render};
use crate::numbers::Arithmetic;

/// Calls the built-in method `method` on `receiver` with up to two arguments, like v1's
/// `call_builtin`; all three values must be rooted.
///
/// Returns `None` when memory is exhausted or `method` is one the v2 backend does not
/// implement yet, which the compiler never emits.
pub fn call_builtin(
    memory: &mut impl Memory,
    vm: u32,
    method: BuiltinMethod,
    receiver: u64,
    first: u64,
    second: u64,
) -> Option<u64> {
    let float = |memory: &mut _, operation: fn(f64) -> f64, message| {
        float_unary(memory, vm, receiver, operation, message)
    };
    let predicate = |memory: &mut _, test: fn(f64) -> bool, message| {
        float_predicate(memory, vm, receiver, test, message)
    };
    match method {
        BuiltinMethod::Add => operator(memory, vm, Some(Arithmetic::Add), receiver, first),
        BuiltinMethod::Sub => operator(memory, vm, Some(Arithmetic::Subtract), receiver, first),
        BuiltinMethod::Mul => operator(memory, vm, Some(Arithmetic::Multiply), receiver, first),
        BuiltinMethod::Div => operator(memory, vm, None, receiver, first),
        BuiltinMethod::DivEuclid => euclidean(memory, vm, receiver, first, false),
        BuiltinMethod::RemEuclid => euclidean(memory, vm, receiver, first, true),
        BuiltinMethod::ToString | BuiltinMethod::Debug => render(memory, vm, receiver),
        BuiltinMethod::Abs => abs(memory, vm, receiver),
        BuiltinMethod::Floor => float(memory, libm::floor, "floor requires a Float receiver"),
        BuiltinMethod::Ceil => float(memory, libm::ceil, "ceil requires a Float receiver"),
        BuiltinMethod::Round => float(memory, libm::round, "round requires a Float receiver"),
        BuiltinMethod::Trunc => float(memory, libm::trunc, "trunc requires a Float receiver"),
        BuiltinMethod::Fract => float(
            memory,
            |value| value - libm::trunc(value),
            "fract requires a Float receiver",
        ),
        BuiltinMethod::Sqrt => float(memory, libm::sqrt, "sqrt requires a Float receiver"),
        BuiltinMethod::Signum => int_unary(
            memory,
            vm,
            receiver,
            signum,
            "signum requires an Int receiver",
        ),
        BuiltinMethod::IsEven => int_unary(
            memory,
            vm,
            receiver,
            |value| boolean(value % 2 == 0),
            "is_even requires an Int receiver",
        ),
        BuiltinMethod::IsOdd => int_unary(
            memory,
            vm,
            receiver,
            |value| boolean(value % 2 != 0),
            "is_odd requires an Int receiver",
        ),
        BuiltinMethod::Gcd => gcd_or_lcm(memory, vm, receiver, first, false),
        BuiltinMethod::Lcm => gcd_or_lcm(memory, vm, receiver, first, true),
        BuiltinMethod::Pow => pow(memory, vm, receiver, first),
        BuiltinMethod::Min => min_or_max(memory, vm, receiver, first, false),
        BuiltinMethod::Max => min_or_max(memory, vm, receiver, first, true),
        BuiltinMethod::Clamp => clamp(memory, vm, receiver, first, second),
        BuiltinMethod::ToFloat => to_float(memory, vm, receiver),
        BuiltinMethod::ToInt => to_int(memory, vm, receiver),
        BuiltinMethod::IsNan => predicate(memory, f64::is_nan, "is_nan requires a Float receiver"),
        BuiltinMethod::IsFinite => predicate(
            memory,
            f64::is_finite,
            "is_finite requires a Float receiver",
        ),
        BuiltinMethod::IsInfinite => predicate(
            memory,
            f64::is_infinite,
            "is_infinite requires a Float receiver",
        ),
        BuiltinMethod::Length => length(memory, vm, receiver),
        BuiltinMethod::IsEmpty => is_empty(memory, vm, receiver),
        BuiltinMethod::Kind => error_part(memory, vm, receiver, ErrorPart::Kind),
        BuiltinMethod::Message => error_part(memory, vm, receiver, ErrorPart::Message),
        BuiltinMethod::Data => error_part(memory, vm, receiver, ErrorPart::Data),
        BuiltinMethod::Cause => error_part(memory, vm, receiver, ErrorPart::Cause),
        _ => None,
    }
}
