//! Dynamic Wasm operations shared across runtime value kinds.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use exs_value::ValueRef;

use crate::gc;
use crate::runtime;
use crate::value::{RtValue, RuntimeList, list, numeric, object};

/// Adds two runtime values through List or numeric dispatch.
pub(crate) fn add(left: ValueRef, right: ValueRef) -> ValueRef {
    match runtime::value(left) {
        RtValue::List(_) => list::operations::add(left, right),
        _ => numeric::arithmetic(left, right, i64::checked_add, |left, right| left + right),
    }
}

/// Tests two runtime values for equality.
pub(crate) fn equal(left: ValueRef, right: ValueRef) -> ValueRef {
    let equal = match (runtime::value(left), runtime::value(right)) {
        (RtValue::Null, RtValue::Null) => true,
        (left, right) if numeric::is_numeric(left) && numeric::is_numeric(right) => {
            numeric::numbers_equal(numeric::number_of(left), numeric::number_of(right))
        }
        (RtValue::String(left), RtValue::String(right)) => left.as_str() == right.as_str(),
        (RtValue::List(_), RtValue::List(_)) | (RtValue::Object(_), RtValue::Object(_)) => {
            left == right
        }
        _ => false,
    };
    runtime::allocate(RtValue::Bool(equal))
}

/// Tests two runtime values for inequality.
pub(crate) fn not_equal(left: ValueRef, right: ValueRef) -> ValueRef {
    let equal = equal(left, right);
    runtime::allocate(RtValue::Bool(!numeric::boolean(equal)))
}

/// Appends a value through the receiver's runtime collection dispatch.
pub(crate) fn append(receiver: ValueRef, item: ValueRef) -> ValueRef {
    list::operations::append(receiver, item)
}

/// Reads one value through the receiver's runtime indexing dispatch.
pub(crate) fn index_get(receiver: ValueRef, index: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        RtValue::List(_) => list::operations::get(receiver, index),
        RtValue::Object(_) => object::operations::get(receiver, index),
        _ => runtime::trap(),
    }
}

/// Replaces one value through the receiver's runtime indexing dispatch.
pub(crate) fn index_set(receiver: ValueRef, index: ValueRef, replacement: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        RtValue::List(_) => list::operations::set(receiver, index, replacement),
        RtValue::Object(_) => object::operations::set(receiver, index, replacement),
        _ => runtime::trap(),
    }
}

/// Creates the shallow List or scalar-String snapshot consumed by a for loop.
pub(crate) fn iter_snapshot(iterable: ValueRef) -> ValueRef {
    match runtime::value(iterable) {
        RtValue::List(list) => {
            let elements = list.elements.clone();
            runtime::allocate(RtValue::List(Box::new(RuntimeList { elements })))
        }
        RtValue::String(string) => {
            let scalars = string
                .as_str()
                .chars()
                .map(|scalar| scalar.to_string())
                .collect::<Vec<_>>();
            let checkpoint = gc::temporary_root_checkpoint();
            let mut elements = Vec::with_capacity(scalars.len());
            for scalar in scalars {
                let value = runtime::allocate(RtValue::String(Box::new(
                    crate::value::RuntimeString::from_string(scalar),
                )));
                gc::push_temporary_root(value);
                elements.push(value);
            }
            let snapshot = runtime::allocate(RtValue::List(Box::new(RuntimeList { elements })));
            gc::restore_temporary_roots(checkpoint);
            snapshot
        }
        _ => runtime::trap(),
    }
}

/// Returns the scalar or entry count for runtime values with a visible length.
pub(crate) fn length(value: ValueRef) -> ValueRef {
    let length = match runtime::value(value) {
        RtValue::String(value) => value.as_str().chars().count(),
        RtValue::List(value) => value.elements.len(),
        RtValue::Object(value) => value.entries.len(),
        _ => runtime::trap(),
    };
    list::operations::length_value(length)
}

/// Dispatches a statically named runtime member method.
pub(crate) fn call_method(receiver: ValueRef, method: ValueRef, arguments: ValueRef) -> ValueRef {
    let method = string_value(method);
    match method.as_str() {
        "push" => list::operations::append(receiver, list::operations::single_argument(arguments)),
        "pop" => {
            list::operations::require_no_arguments(arguments);
            list::operations::pop(receiver)
        }
        "insert" => {
            let (index, value) = list::operations::two_arguments(arguments);
            list::operations::insert(receiver, index, value)
        }
        "remove" => {
            list::operations::remove(receiver, list::operations::single_argument(arguments))
        }
        "clear" => {
            list::operations::require_no_arguments(arguments);
            list::operations::clear(receiver)
        }
        "has" => object::operations::has(receiver, list::operations::single_argument(arguments)),
        "delete" => {
            object::operations::delete(receiver, list::operations::single_argument(arguments))
        }
        "keys" => {
            list::operations::require_no_arguments(arguments);
            object::operations::keys(receiver)
        }
        "values" => {
            list::operations::require_no_arguments(arguments);
            object::operations::values(receiver)
        }
        _ => runtime::trap(),
    }
}

/// Copies one runtime String value for use as a key or method name.
pub(crate) fn string_value(reference: ValueRef) -> String {
    match runtime::value(reference) {
        RtValue::String(value) => value.as_str().into(),
        _ => runtime::trap(),
    }
}
