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
        (RtValue::None, RtValue::None) => true,
        (left, right) if numeric::is_numeric(left) && numeric::is_numeric(right) => {
            match (numeric::number_of(left), numeric::number_of(right)) {
                (Some(left), Some(right)) => numeric::numbers_equal(left, right),
                _ => false,
            }
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
        _ => runtime::recoverable_error(
            "TypeError",
            "index access requires a List or Object receiver",
            receiver,
        ),
    }
}

/// Replaces one value through the receiver's runtime indexing dispatch.
pub(crate) fn index_set(receiver: ValueRef, index: ValueRef, replacement: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        RtValue::List(_) => list::operations::set(receiver, index, replacement),
        RtValue::Object(_) => object::operations::set(receiver, index, replacement),
        _ => runtime::recoverable_error(
            "TypeError",
            "index assignment requires a List or Object receiver",
            receiver,
        ),
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
        _ => runtime::recoverable_error(
            "NotIterable",
            "for requires a List or String iterable",
            iterable,
        ),
    }
}

/// Returns the scalar or entry count for runtime values with a visible length.
pub(crate) fn length(value: ValueRef) -> ValueRef {
    let length = match runtime::value(value) {
        RtValue::String(value) => value.as_str().chars().count(),
        RtValue::List(value) => value.elements.len(),
        RtValue::Object(value) => value.entries.len(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "len requires a String, List, or Object value",
                value,
            );
        }
    };
    list::operations::length_value(length)
}

/// Dispatches a statically named runtime member method.
pub(crate) fn call_method(receiver: ValueRef, method: ValueRef, arguments: ValueRef) -> ValueRef {
    let method = match string_value(method) {
        Ok(method) => method,
        Err(error) => return error,
    };
    match method.as_str() {
        "push" => match list::operations::single_argument(arguments) {
            Ok(item) => list::operations::append(receiver, item),
            Err(error) => error,
        },
        "pop" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => list::operations::pop(receiver),
            Err(error) => error,
        },
        "insert" => match list::operations::two_arguments(arguments) {
            Ok((index, value)) => list::operations::insert(receiver, index, value),
            Err(error) => error,
        },
        "remove" => match list::operations::single_argument(arguments) {
            Ok(index) => list::operations::remove(receiver, index),
            Err(error) => error,
        },
        "clear" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => list::operations::clear(receiver),
            Err(error) => error,
        },
        "has" => match list::operations::single_argument(arguments) {
            Ok(key) => object::operations::has(receiver, key),
            Err(error) => error,
        },
        "delete" => match list::operations::single_argument(arguments) {
            Ok(key) => object::operations::delete(receiver, key),
            Err(error) => error,
        },
        "keys" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => object::operations::keys(receiver),
            Err(error) => error,
        },
        "values" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => object::operations::values(receiver),
            Err(error) => error,
        },
        _ => runtime::recoverable_error(
            "MethodNotFound",
            "receiver does not support this method",
            receiver,
        ),
    }
}

/// Copies one runtime String value for use as a key or method name.
pub(crate) fn string_value(reference: ValueRef) -> Result<String, ValueRef> {
    match runtime::value(reference) {
        RtValue::String(value) => Ok(value.as_str().into()),
        _ => Err(runtime::recoverable_error(
            "TypeError",
            "Object keys and method names require a String value",
            reference,
        )),
    }
}
