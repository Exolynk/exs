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
//! operations, Strings, Lists, typed Objects, and the CBOR conversion of runner inputs and
//! results.

#[cfg(test)]
extern crate std;

mod access;
mod allocator;
mod cbor;
mod collector;
mod execution;
mod lists;
mod memory;
mod numbers;
mod pages;
mod strings;
mod typed;
mod values;
mod vm;

#[cfg(test)]
mod tests;

pub use access::{AccessError, index_get, index_set, property_get, property_set};
pub use allocator::{allocate, init_heap};
pub use cbor::{CborError, decode_inputs, encode_result};
pub use collector::collect;
pub use execution::{init_execution, reserve_input, result};
pub use exs_layout::ObjectKind;
pub use lists::{list_add, list_copy, list_new, list_object};
#[cfg(target_arch = "wasm32")]
pub use memory::LinearMemory;
pub use memory::Memory;
pub use numbers::{
    Arithmetic, Number, NumericError, Ordering, arithmetic, divide, float_value, int_value, negate,
    number,
};
pub use strings::{StringError, concat, string_object};
pub use values::{compare, equal};
pub use vm::{collection_count, set_heap_flags, set_program_types, set_root_ranges};
