//! Language Error tests: kinds and messages, origins, traces and their encoding.

use core::mem::offset_of;
use std::boxed::Box;
use std::vec;
use std::vec::Vec;

use exs_abi::{ErrorSeverity, ExsError, ExsStackFrame, ExsValue, SourcePositionId};
use exs_layout::ValueRef;
use exs_layout::VmState;

use super::execution::Execution;
use super::heap::VM;
use crate::{
    Arithmetic, Memory, arithmetic, concat, error_new, for_snapshot, missing_value, negate,
    type_mismatch,
};

/// Sets the VM's current source position.
fn set_position(execution: &mut Execution, position: u32) {
    execution.memory.store_u32(
        VM + offset_of!(VmState, errors.source_position) as u32,
        position,
    );
}

/// Pushes one trace entry for `function_id` called from `call_site`.
fn push_trace(execution: &mut Execution, function_id: u32, call_site: u32) {
    let top = VM + offset_of!(VmState, errors.trace_top) as u32;
    let entry = execution.memory.load_u32(top);
    execution
        .memory
        .store_u64(entry, u64::from(function_id) | (u64::from(call_site) << 32));
    execution.memory.store_u32(top, entry + 8);
}

/// Produces the numeric TypeError and IntOverflowError values with their data.
#[test]
fn reports_numeric_errors_like_v1() {
    let mut execution = Execution::new();
    let big = crate::int_value(&mut execution.memory, VM, i64::MAX).expect("room");
    execution.root(&[big]);
    let overflow = arithmetic(&mut execution.memory, VM, Arithmetic::Add, big, big).expect("room");
    assert_eq!(
        execution.error(overflow),
        (
            false,
            "IntOverflowError".into(),
            "integer arithmetic overflowed the ExS signed 64-bit range".into(),
            big
        )
    );
    let none = ValueRef::NONE.raw();
    let not_numeric = arithmetic(
        &mut execution.memory,
        VM,
        Arithmetic::Subtract,
        Execution::int(1),
        none,
    )
    .expect("room");
    assert_eq!(
        execution.error(not_numeric),
        (
            false,
            "TypeError".into(),
            "numeric operations require an Int or Float value".into(),
            none
        )
    );
    let min = crate::int_value(&mut execution.memory, VM, i64::MIN).expect("room");
    execution.root(&[min]);
    let negated = negate(&mut execution.memory, VM, min).expect("room");
    assert_eq!(
        execution.error(negated).2,
        "integer negation overflowed the ExS signed 64-bit range"
    );
}

/// Gives every failing `+` step its own origin and adds later operands to the Error, like the
/// one-`+`-at-a-time evaluation.
#[test]
fn fails_concatenation_steps_like_v1() {
    let mut execution = Execution::new();
    let prefix = execution.string("a");
    let positions = 3000;
    for (index, position) in (0..).zip([11_u32, 12, 13]) {
        execution.memory.store_u32(positions + index * 4, position);
    }
    let none = ValueRef::NONE.raw();
    let slots = execution.root(&[prefix, Execution::int(1), none, Execution::int(2)]);
    let result = concat(&mut execution.memory, VM, slots, 4, positions).expect("room");
    // "a" + 1 joins, + None fails at step 2, and + 2 adds a number to that Error at step 3.
    let (_, kind, message, inner) = execution.error(result);
    assert_eq!(kind, "TypeError");
    assert_eq!(message, "numeric operations require an Int or Float value");
    let (_, _, inner_message, inner_data) = execution.error(inner);
    assert_eq!(
        inner_message,
        "String addition requires a String, Bool, Int, or Float right operand"
    );
    assert_eq!(inner_data, none);
    assert_eq!(
        execution.encoded(inner),
        ExsValue::Error(ExsError {
            severity: ErrorSeverity::Recoverable,
            kind: "TypeError".into(),
            message: inner_message,
            data: Box::new(ExsValue::None),
            origin: Some(SourcePositionId(12)),
            trace: Vec::new(),
            cause: None,
        })
    );
    let ExsValue::Error(outer) = execution.encoded(result) else {
        panic!("an Error");
    };
    assert_eq!(outer.origin, Some(SourcePositionId(13)));
}

/// Records the current origin and the active trace innermost first, and encodes them like the
/// runner's codec.
#[test]
fn records_origin_and_trace() {
    let mut execution = Execution::new();
    push_trace(&mut execution, 4, 0);
    push_trace(&mut execution, 7, 21);
    set_position(&mut execution, 33);
    let kind = execution.string("Custom");
    let message = execution.string("went wrong");
    let data = execution.list(&[Execution::int(1)]);
    let error = error_new(&mut execution.memory, VM, kind, message, data).expect("room");
    assert_eq!(
        execution.encoded(error),
        ExsValue::Error(ExsError {
            severity: ErrorSeverity::Recoverable,
            kind: "Custom".into(),
            message: "went wrong".into(),
            data: Box::new(ExsValue::List(vec![ExsValue::Int(1)])),
            origin: Some(SourcePositionId(33)),
            trace: vec![
                ExsStackFrame {
                    function_id: 7,
                    call_site: SourcePositionId(21),
                },
                ExsStackFrame {
                    function_id: 4,
                    call_site: SourcePositionId(0),
                },
            ],
            cause: None,
        })
    );
    let invalid = error_new(&mut execution.memory, VM, data, message, data).expect("room");
    assert_eq!(
        execution.error(invalid).2,
        "error kind requires a String value"
    );
}

/// Creates the fatal or recoverable contract TypeError, MissingValue and NotIterable.
#[test]
fn creates_language_errors() {
    let mut execution = Execution::new();
    let one = Execution::int(1);
    let fatal = type_mismatch(&mut execution.memory, VM, one, false).expect("room");
    assert_eq!(
        execution.error(fatal),
        (
            true,
            "TypeError".into(),
            "value does not satisfy the declared function type".into(),
            one
        )
    );
    let recoverable = type_mismatch(&mut execution.memory, VM, one, true).expect("room");
    assert!(!execution.error(recoverable).0);
    let missing = missing_value(&mut execution.memory, VM).expect("room");
    assert_eq!(execution.error_kind(missing), "MissingValue");
    let not_iterable = for_snapshot(&mut execution.memory, VM, one).expect("room");
    assert_eq!(
        execution.error(not_iterable).2,
        "for requires an iterable or Iterator receiver"
    );
}
