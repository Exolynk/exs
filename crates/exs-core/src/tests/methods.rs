//! Built-in method and static function tests.

use exs_abi::BuiltinMethod;
use exs_layout::StaticFunction;
use exs_value::ValueRef;

use super::execution::Execution;
use super::heap::VM;
use crate::{Number, call_builtin, call_static, float_value, int_value, number};

/// Calls one built-in method on `receiver`.
fn call(execution: &mut Execution, method: BuiltinMethod, receiver: u64, first: u64) -> u64 {
    let none = ValueRef::NONE.raw();
    call_builtin(&mut execution.memory, VM, method, receiver, first, none).expect("room")
}

/// Computes numeric methods like v1 and reports its Errors.
#[test]
fn evaluates_numeric_methods() {
    let mut execution = Execution::new();
    let (two, ten) = (Execution::int(2), Execution::int(10));
    let power = call(&mut execution, BuiltinMethod::Pow, two, ten);
    assert_eq!(number(&execution.memory, power), Some(Number::Int(1024)));
    let gcd = call(
        &mut execution,
        BuiltinMethod::Gcd,
        Execution::int(12),
        Execution::int(-18),
    );
    assert_eq!(gcd, Execution::int(6));
    let min = int_value(&mut execution.memory, VM, i64::MIN).expect("room");
    execution.root(&[min]);
    let overflow = call(
        &mut execution,
        BuiltinMethod::Abs,
        min,
        ValueRef::NONE.raw(),
    );
    assert_eq!(execution.error_kind(overflow), "IntOverflowError");
    let not_int = call(
        &mut execution,
        BuiltinMethod::IsEven,
        ValueRef::TRUE.raw(),
        0,
    );
    assert_eq!(
        execution.error(not_int).2,
        "is_even requires an Int receiver"
    );
    let half = float_value(&mut execution.memory, VM, 2.5).expect("room");
    execution.root(&[half]);
    let rounded = call(&mut execution, BuiltinMethod::Round, half, 0);
    assert_eq!(number(&execution.memory, rounded), Some(Number::Float(3.0)));
    let truncated = call(&mut execution, BuiltinMethod::ToInt, half, 0);
    assert_eq!(truncated, two);
    let negative = call(
        &mut execution,
        BuiltinMethod::RemEuclid,
        Execution::int(-7),
        Execution::int(3),
    );
    assert_eq!(negative, two);
    let zero = call(
        &mut execution,
        BuiltinMethod::DivEuclid,
        ten,
        Execution::int(0),
    );
    assert_eq!(execution.error_kind(zero), "DivisionByZeroError");
}

/// Renders, measures and inspects values like v1.
#[test]
fn evaluates_value_methods() {
    let mut execution = Execution::new();
    let list = execution.list(&[Execution::int(1), Execution::int(2)]);
    let rendered = call(&mut execution, BuiltinMethod::ToString, list, 0);
    assert_eq!(execution.text(rendered), "[]");
    let text = execution.string("äb");
    assert_eq!(
        call(&mut execution, BuiltinMethod::Length, text, 0),
        Execution::int(2)
    );
    assert_eq!(
        call(&mut execution, BuiltinMethod::IsEmpty, list, 0),
        ValueRef::FALSE.raw()
    );
    let error = call(&mut execution, BuiltinMethod::Length, Execution::int(1), 0);
    assert_eq!(execution.error_kind(error), "TypeError");
    let kind = call(&mut execution, BuiltinMethod::Kind, error, 0);
    assert_eq!(execution.text(kind), "TypeError");
    let data = call(&mut execution, BuiltinMethod::Data, error, 0);
    assert_eq!(data, Execution::int(1));
}

/// Parses numbers and evaluates Math functions like v1.
#[test]
fn evaluates_static_functions() {
    let mut execution = Execution::new();
    let none = ValueRef::NONE.raw();
    let digits = execution.string("-42");
    let parsed = call_static(
        &mut execution.memory,
        VM,
        StaticFunction::IntParse,
        digits,
        none,
    );
    assert_eq!(parsed, Some(Execution::int(-42)));
    let invalid = execution.string("4.2.1");
    let error = call_static(
        &mut execution.memory,
        VM,
        StaticFunction::FloatParse,
        invalid,
        none,
    )
    .expect("room");
    assert_eq!(execution.error_kind(error), "ParseError");
    let one = float_value(&mut execution.memory, VM, 1.0).expect("room");
    execution.root(&[one]);
    let angle = call_static(
        &mut execution.memory,
        VM,
        StaticFunction::MathAtan2,
        one,
        one,
    )
    .expect("room");
    assert_eq!(
        number(&execution.memory, angle),
        Some(Number::Float(core::f64::consts::FRAC_PI_4))
    );
    let error = call_static(
        &mut execution.memory,
        VM,
        StaticFunction::MathHypot,
        one,
        none,
    )
    .expect("room");
    assert_eq!(
        execution.error(error).2,
        "Math::hypot requires Float arguments requires a Float argument"
    );
}
