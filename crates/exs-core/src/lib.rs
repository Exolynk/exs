#![no_std]

//! The stateless core library of ExS runtime v2.
//!
//! Every function here works on state that lives in the module's linear memory, laid out by
//! `exs-layout`, and keeps nothing for itself: no `static` state and no allocator of its own.
//! All functions are generic over [`Memory`], so the same code runs directly on Wasm linear
//! memory and, in native tests, on a byte vector.
//!
//! It provides the heap (a size-class page allocator and a non-moving mark-and-sweep
//! collector), execution start-up with the root stack and runner buffers, numeric values and
//! operations, language Errors with traces, Strings, Lists, typed and untyped Objects with shapes, and the CBOR conversion of runner inputs and
//! results.

#[cfg(test)]
extern crate std;

mod access;
mod allocator;
mod boundary;
mod bytes;
mod cbor;
mod closures;
mod collector;
mod datetime;
mod enums;
mod errors;
mod execution;
mod lists;
mod memory;
mod methods;
mod numbers;
mod objects;
mod pages;
mod ranges;
mod render;
mod shapes;
mod strings;
mod tasks;
mod typed;
mod values;
mod vm;
mod wire;

#[cfg(test)]
mod tests;

pub use access::{index_get, index_set, property_get, property_set};
pub use allocator::{allocate, init_heap};
pub use bytes::bytes_object;
pub use cbor::{CborError, arity_error, decode_inputs, decode_variadic_inputs, encode_result};
pub use closures::{cell_new, closure_arity_error, closure_new, not_callable};
pub use collector::collect;
pub use datetime::{DATETIME_FIELDS, datetime_parse};
pub use enums::{enum_new, match_error, missing_field};
pub use errors::{
    condition_error, error_new, error_object, method_arity_error, method_not_found, missing_value,
    nominal_arity_error, not_an_iterator, not_error, type_mismatch, wire_mismatch,
};
pub use execution::{init_execution, reserve_input, result};
pub use exs_abi::BuiltinMethod;
pub use exs_layout::{ObjectKind, StaticFunction};
pub use lists::{for_snapshot, list_add, list_copy, list_new, list_object};
#[cfg(target_arch = "wasm32")]
pub use memory::LinearMemory;
pub use memory::Memory;
pub use methods::{call_builtin, call_static};
pub use numbers::{
    Arithmetic, Number, NumericError, Ordering, arithmetic, divide, float_value, int_value, negate,
    number,
};
pub use objects::{object_lookup, object_with_shape};
pub use ranges::{RANGE_DONE, RANGE_FALLBACK, range_begin, range_finish, range_step};
pub use shapes::shape_for;
pub use strings::{concat, format, string_object};
pub use tasks::{
    Host, frame_call, frame_return, frame_start, host_call_resume, host_call_start, host_call_take,
    take_completed, task_cancel,
};
pub use values::{compare, compare_value, equal, ordering_test};
pub use vm::{
    collection_count, set_case_tables, set_heap_flags, set_program_types, set_root_ranges,
};
pub use wire::wire_decode;
