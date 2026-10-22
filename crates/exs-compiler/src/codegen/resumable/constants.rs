//! Durable slots that only hold one constant.
//!
//! the graph stores every literal in a temporary slot before an operation reads it. A slot
//! whose only writer is one constant operation always holds that constant, so its readers use
//! the constant directly and the writer emits nothing: in a loop, its literals no longer go
//! through the frame on every iteration.

use std::collections::HashMap;

use exs_layout::ValueRef;

use crate::ast::Expression;
use crate::codegen::continuation::Operation;

/// Returns the raw value of every slot of `operations` that only one constant operation
/// writes: an immediate None, Bool or small Int, or a static String from `strings`.
pub(super) fn constant_slots(
    operations: &[Operation<'_, '_>],
    strings: &HashMap<String, u64>,
) -> HashMap<u32, u64> {
    let mut writers: HashMap<u32, usize> = HashMap::new();
    for operation in operations {
        for slot in written_slots(operation) {
            *writers.entry(slot).or_default() += 1;
        }
    }
    operations
        .iter()
        .filter_map(|operation| constant(operation, strings))
        .filter(|(slot, _)| writers.get(slot) == Some(&1))
        .collect()
}

/// Returns the slot and raw value of a constant operation.
pub(super) fn constant(
    operation: &Operation<'_, '_>,
    strings: &HashMap<String, u64>,
) -> Option<(u32, u64)> {
    let small = |value: i128| {
        i64::try_from(value)
            .ok()
            .and_then(ValueRef::small_int)
            .map(ValueRef::raw)
    };
    match operation {
        Operation::Literal {
            expression,
            destination,
        } => {
            let value = match expression {
                Expression::Integer(value, _) => small(*value)?,
                Expression::Bool(value, _) => ValueRef::boolean(*value).raw(),
                Expression::None(_) => ValueRef::NONE.raw(),
                Expression::String(text, _) => *strings.get(text)?,
                _ => return None,
            };
            Some((*destination, value))
        }
        Operation::String {
            value, destination, ..
        } => Some((*destination, *strings.get(*value)?)),
        Operation::Integer {
            value, destination, ..
        } => Some((*destination, small(i128::from(*value))?)),
        Operation::None { destination, .. } => Some((*destination, ValueRef::NONE.raw())),
        Operation::Boolean {
            value, destination, ..
        } => Some((*destination, ValueRef::boolean(*value).raw())),
        _ => None,
    }
}

/// Returns the slots of the operation's frame that `operation` writes, as the step emitter
/// implements it.
fn written_slots(operation: &Operation<'_, '_>) -> Vec<u32> {
    match operation {
        Operation::Literal { destination, .. }
        | Operation::String { destination, .. }
        | Operation::RuntimeStatic { destination, .. }
        | Operation::Integer { destination, .. }
        | Operation::None { destination, .. }
        | Operation::Boolean { destination, .. }
        | Operation::Copy { destination, .. }
        | Operation::WireDecode { destination, .. }
        | Operation::CellNew { destination, .. }
        | Operation::CellGet { destination, .. }
        | Operation::Closure { destination, .. }
        | Operation::Unary { destination, .. }
        | Operation::Binary { destination, .. }
        | Operation::FormatNew { destination, .. }
        | Operation::FormatFinish { destination, .. }
        | Operation::OrderingTest { destination, .. }
        | Operation::List { destination, .. }
        | Operation::Object { destination, .. }
        | Operation::Error { destination, .. }
        | Operation::Assert { destination, .. }
        | Operation::TypedObject { destination, .. }
        | Operation::Enum { destination, .. }
        | Operation::EnumMatches { destination, .. }
        | Operation::EnumField { destination, .. }
        | Operation::MatchError { destination, .. }
        | Operation::IsType { destination, .. }
        | Operation::Propagate { destination, .. }
        | Operation::Index { destination, .. }
        | Operation::Property { destination, .. }
        | Operation::HostResume { destination, .. }
        | Operation::HostStream { destination, .. }
        | Operation::DirectCall { destination, .. }
        | Operation::ChildCall { destination, .. }
        | Operation::ClosureCall { destination, .. }
        | Operation::ParallelStart { destination, .. }
        | Operation::ParallelDynamicStart { destination, .. }
        | Operation::ParallelTake { destination, .. }
        | Operation::InstanceCall { destination, .. }
        | Operation::IterSnapshot { destination, .. } => std::vec![*destination],
        Operation::Operator {
            destination,
            comparison,
            ..
        } => std::iter::once(*destination)
            .chain(comparison.map(|comparison| comparison.destination))
            .collect(),
        Operation::HostCall {
            argument_list,
            destination,
            ..
        } => std::vec![*argument_list, *destination],
        Operation::HostTime {
            destination,
            fields,
            ..
        } => std::iter::once(*destination)
            .chain(fields.iter().map(|field| field.slot))
            .collect(),
        Operation::NativeIterNext { item, .. } | Operation::IteratorBranch { item, .. } => {
            std::vec![*item]
        }
        Operation::CellSet { .. }
        | Operation::FormatPush { .. }
        | Operation::FormatValue { .. }
        | Operation::IndexSet { .. }
        | Operation::PropertySet { .. }
        | Operation::ValidateParameters { .. }
        | Operation::ValidateVariadicParameter { .. }
        | Operation::ValidateSlot { .. }
        | Operation::Branch { .. }
        | Operation::Goto { .. }
        | Operation::Return { .. } => Vec::new(),
    }
}
