//! Language Errors: creation with origin and trace, and the Errors the language defines.
//!
//! Every Error records the VM's current source position as its origin and a copy of the
//! active trace, innermost function first. Kinds and messages match runtime v1 exactly.

use core::mem::{offset_of, size_of};

use exs_abi::{TYPE_BYTES, TYPE_ERROR};
use exs_layout::{ERROR_FATAL, ERROR_RECOVERABLE, ErrorObject, ObjectKind, StringObject};
use exs_value::ValueRef;
use exs_value::layout::{KIND_SHIFT, type_kind};

use crate::allocator::allocate;
use crate::memory::Memory;
use crate::strings::{string_object, text_string};
use crate::vm::Vm;

/// Byte offset of the severity in the header word.
const SEVERITY: u32 = 4;
/// Byte offset of [`ErrorObject::origin`].
const ORIGIN: u32 = offset_of!(ErrorObject, origin) as u32;
/// Byte offset of [`ErrorObject::trace`].
const TRACE: u32 = offset_of!(ErrorObject, trace) as u32;
/// Byte offset of [`ErrorObject::kind`].
const KIND: u32 = offset_of!(ErrorObject, kind) as u32;
/// Byte offset of [`ErrorObject::message`].
const MESSAGE: u32 = offset_of!(ErrorObject, message) as u32;
/// Byte offset of [`ErrorObject::data`].
const DATA: u32 = offset_of!(ErrorObject, data) as u32;
/// Byte offset of [`ErrorObject::cause`].
const CAUSE: u32 = offset_of!(ErrorObject, cause) as u32;
/// Byte offset of the byte length of a Bytes object.
const BYTES_LEN: u32 = offset_of!(StringObject, len) as u32;
/// Byte offset of the first byte of a Bytes object.
const BYTES_DATA: u32 = size_of::<StringObject>() as u32;

/// Returns the address of the Error behind one raw reference, or `None` for other values.
#[must_use]
pub fn error_object(raw: u64) -> Option<u32> {
    (raw >> KIND_SHIFT == u64::from(type_kind(TYPE_ERROR))).then_some(raw as u32)
}

/// Returns whether `raw` is a fatal Error.
pub(crate) fn is_fatal(memory: &impl Memory, raw: u64) -> bool {
    error_object(raw).is_some_and(|object| memory.load_u32(object + SEVERITY) == ERROR_FATAL)
}

/// The parts of one Error, as the boundary walk reads them.
pub(crate) struct ErrorParts {
    /// Whether the Error is fatal.
    pub(crate) fatal: bool,
    /// Kind String.
    pub(crate) kind: u64,
    /// Message String.
    pub(crate) message: u64,
    /// Attached data.
    pub(crate) data: u64,
    /// Origin source position, 0 for none.
    pub(crate) origin: u32,
    /// Address of the first trace entry and the number of entries.
    pub(crate) trace: (u32, u32),
    /// Related prior Error or value.
    pub(crate) cause: u64,
}

/// Reads the parts of the Error at `object`.
pub(crate) fn error_parts(memory: &impl Memory, object: u32) -> ErrorParts {
    let trace = memory.load_u32(object + TRACE);
    ErrorParts {
        fatal: memory.load_u32(object + SEVERITY) == ERROR_FATAL,
        kind: memory.load_u64(object + KIND),
        message: memory.load_u64(object + MESSAGE),
        data: memory.load_u64(object + DATA),
        origin: memory.load_u32(object + ORIGIN),
        trace: if trace == 0 {
            (0, 0)
        } else {
            (trace + BYTES_DATA, memory.load_u32(trace + BYTES_LEN) / 8)
        },
        cause: memory.load_u64(object + CAUSE),
    }
}

/// Creates one Error from the Strings `kind` and `message` with the current origin and trace.
///
/// All three values must be rooted or static; the Error keeps them alive from then on.
pub(crate) fn error_from_strings(
    memory: &mut impl Memory,
    vm: u32,
    fatal: bool,
    kind: u64,
    message: u64,
    data: u64,
) -> Option<u64> {
    let state = Vm(vm);
    let base = state.trace_base(memory);
    let top = state.trace_top(memory).min(state.trace_limit(memory));
    let entries = top.saturating_sub(base) / 8;
    let mut trace = 0;
    let root = state.root_stack_end(memory);
    if entries > 0 {
        let bytes = entries * 8;
        trace = allocate(memory, vm, BYTES_DATA + bytes, ObjectKind::Bytes)?;
        memory.store_u32(trace + BYTES_LEN, bytes);
        // Innermost function first, like runtime v1.
        for index in 0..entries {
            let entry = memory.load_u64(top - (index + 1) * 8);
            memory.store_u64(trace + BYTES_DATA + index * 8, entry);
        }
        let reference = ValueRef::from_parts(type_kind(TYPE_BYTES), 0, trace)?.raw();
        state.push_root(memory, reference)?;
    }
    let object = allocate(
        memory,
        vm,
        size_of::<ErrorObject>() as u32,
        ObjectKind::Error,
    );
    state.pop_roots(memory, root);
    let object = object?;
    let severity = if fatal {
        ERROR_FATAL
    } else {
        ERROR_RECOVERABLE
    };
    memory.store_u32(object + SEVERITY, severity);
    memory.store_u32(object + ORIGIN, state.source_position(memory));
    memory.store_u32(object + TRACE, trace);
    memory.store_u64(object + KIND, kind);
    memory.store_u64(object + MESSAGE, message);
    memory.store_u64(object + DATA, data);
    memory.store_u64(object + CAUSE, ValueRef::NONE.raw());
    ValueRef::from_parts(type_kind(TYPE_ERROR), 0, object).map(ValueRef::raw)
}

/// Creates one Error from all its parts, as the runner's codec carries them: `trace` is the
/// address of a Bytes object of trace entries innermost first, or 0.
///
/// Every value and the trace must be rooted by the caller.
pub(crate) fn error_from_parts(
    memory: &mut impl Memory,
    vm: u32,
    parts: &ErrorParts,
    trace: u32,
) -> Option<u64> {
    let object = allocate(
        memory,
        vm,
        size_of::<ErrorObject>() as u32,
        ObjectKind::Error,
    )?;
    let severity = if parts.fatal {
        ERROR_FATAL
    } else {
        ERROR_RECOVERABLE
    };
    memory.store_u32(object + SEVERITY, severity);
    memory.store_u32(object + ORIGIN, parts.origin);
    memory.store_u32(object + TRACE, trace);
    memory.store_u64(object + KIND, parts.kind);
    memory.store_u64(object + MESSAGE, parts.message);
    memory.store_u64(object + DATA, parts.data);
    memory.store_u64(object + CAUSE, parts.cause);
    ValueRef::from_parts(type_kind(TYPE_ERROR), 0, object).map(ValueRef::raw)
}

/// Creates one Error with the texts `kind` and `message`, attaching `data`.
///
/// `data` need not be rooted by the caller.
pub(crate) fn new_error(
    memory: &mut impl Memory,
    vm: u32,
    fatal: bool,
    kind: &str,
    message: &str,
    data: u64,
) -> Option<u64> {
    let state = Vm(vm);
    let root = state.push_root(memory, data)?;
    let created = (|| {
        let kind = text_string(memory, vm, kind)?;
        state.push_root(memory, kind)?;
        let message = text_string(memory, vm, message)?;
        state.push_root(memory, message)?;
        error_from_strings(memory, vm, fatal, kind, message, data)
    })();
    state.pop_roots(memory, root);
    created
}

/// Clears the origin and trace of the new Error at `object`, for Errors of the runner
/// boundary, which v1 reports without either.
pub(crate) fn clear_origin_and_trace(memory: &mut impl Memory, object: u32) {
    memory.store_u32(object + ORIGIN, 0);
    memory.store_u32(object + TRACE, 0);
}

/// Creates one recoverable Error with the texts `kind` and `message`, attaching `data`.
pub(crate) fn recoverable(
    memory: &mut impl Memory,
    vm: u32,
    kind: &str,
    message: &str,
    data: u64,
) -> Option<u64> {
    new_error(memory, vm, false, kind, message, data)
}

/// Evaluates the language's `Error(kind, message, data)` constructor.
///
/// Returns a TypeError when `kind` or `message` is no String. All values must be rooted.
pub fn error_new(
    memory: &mut impl Memory,
    vm: u32,
    kind: u64,
    message: u64,
    data: u64,
) -> Option<u64> {
    if string_object(kind).is_none() {
        return recoverable(
            memory,
            vm,
            "TypeError",
            "error kind requires a String value",
            kind,
        );
    }
    if string_object(message).is_none() {
        return recoverable(
            memory,
            vm,
            "TypeError",
            "error message requires a String value",
            message,
        );
    }
    error_from_strings(memory, vm, false, kind, message, data)
}

/// Creates the TypeError for a value that violates a declared type: recoverable when the
/// function's return type admits Errors, fatal otherwise.
pub fn type_mismatch(
    memory: &mut impl Memory,
    vm: u32,
    value: u64,
    recoverable: bool,
) -> Option<u64> {
    new_error(
        memory,
        vm,
        !recoverable,
        "TypeError",
        "value does not satisfy the declared function type",
        value,
    )
}

/// Creates the MissingValue Error that `?` produces for None.
pub fn missing_value(memory: &mut impl Memory, vm: u32) -> Option<u64> {
    recoverable(
        memory,
        vm,
        "MissingValue",
        "cannot propagate a missing value",
        ValueRef::NONE.raw(),
    )
}

/// Creates the TypeError of a condition that is no Bool.
pub fn condition_error(memory: &mut impl Memory, vm: u32, value: u64) -> Option<u64> {
    recoverable(
        memory,
        vm,
        "TypeError",
        "condition requires a Bool value",
        value,
    )
}

/// Evaluates `!value` for a value that is no Bool, which is a TypeError.
pub fn not_error(memory: &mut impl Memory, vm: u32, value: u64) -> Option<u64> {
    recoverable(memory, vm, "TypeError", "! requires a Bool value", value)
}

/// Creates the WireDecodeError of a `let` binding whose value violates its declared type.
pub fn wire_mismatch(memory: &mut impl Memory, vm: u32, value: u64) -> Option<u64> {
    recoverable(
        memory,
        vm,
        "WireDecodeError",
        "host value does not satisfy the declared ExS type",
        value,
    )
}

/// Creates the NotIterable Error of a `for` loop over a value that cannot be iterated.
pub(crate) fn not_iterable(memory: &mut impl Memory, vm: u32, value: u64) -> Option<u64> {
    recoverable(
        memory,
        vm,
        "NotIterable",
        "for requires an iterable or Iterator receiver",
        value,
    )
}

/// Creates the MethodNotFound Error of a method name no built-in or implementation provides.
pub fn method_not_found(memory: &mut impl Memory, vm: u32, receiver: u64) -> Option<u64> {
    recoverable(
        memory,
        vm,
        "MethodNotFound",
        "receiver does not support this method",
        receiver,
    )
}

/// Creates the NotIterable Error of a `for` loop over an Object that implements no Iterator,
/// like v1's built-in `next`.
pub fn not_an_iterator(memory: &mut impl Memory, vm: u32, value: u64) -> Option<u64> {
    recoverable(
        memory,
        vm,
        "NotIterable",
        "next requires an Iterator receiver",
        value,
    )
}

/// Creates the ArityError of an implementation method called with the wrong number of
/// arguments, like v1.
pub fn nominal_arity_error(memory: &mut impl Memory, vm: u32, receiver: u64) -> Option<u64> {
    recoverable(
        memory,
        vm,
        "ArityError",
        "method received an incorrect number of arguments",
        receiver,
    )
}

/// Creates the ArityError of a built-in method called with the wrong number of arguments,
/// with the `count` arguments in the rooted slots at `slots` as a List in its data, like v1.
pub fn method_arity_error(
    memory: &mut impl Memory,
    vm: u32,
    slots: u32,
    count: u32,
    expected: u32,
) -> Option<u64> {
    let arguments = crate::lists::list_new(memory, vm, slots, count)?;
    let message = match expected {
        0 => "method expects no arguments",
        1 => "method expects exactly one argument",
        _ => "method expects exactly two arguments",
    };
    recoverable(memory, vm, "ArityError", message, arguments)
}

/// Allocates a copy of the Error at `object` sharing its parts, for `clone` to fill in.
pub(crate) fn copy_error(memory: &mut impl Memory, vm: u32, object: u32) -> Option<u64> {
    let copy = allocate(
        memory,
        vm,
        size_of::<ErrorObject>() as u32,
        ObjectKind::Error,
    )?;
    memory.copy(copy + 4, object + 4, size_of::<ErrorObject>() as u32 - 4);
    ValueRef::from_parts(type_kind(TYPE_ERROR), 0, copy).map(ValueRef::raw)
}

/// Returns the addresses of the data and cause fields of the Error at `object`.
pub(crate) const fn error_value_fields(object: u32) -> [u32; 2] {
    [object + DATA, object + CAUSE]
}
