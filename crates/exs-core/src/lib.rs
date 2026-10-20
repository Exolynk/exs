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
//! operations, and the CBOR conversion of runner inputs and results.

#[cfg(test)]
extern crate std;

mod allocator;
mod cbor;
mod collector;
mod execution;
mod memory;
mod numbers;
mod pages;
mod vm;

#[cfg(test)]
mod tests;

pub use allocator::{allocate, init_heap};
pub use cbor::{CborError, decode_inputs, encode_result};
pub use collector::collect;
pub use execution::{init_execution, reserve_input, result};
pub use exs_layout::ObjectKind;
#[cfg(target_arch = "wasm32")]
pub use memory::LinearMemory;
pub use memory::Memory;
pub use numbers::{
    Arithmetic, Number, NumericError, Ordering, arithmetic, compare, divide, equal, float_value,
    int_value, negate, number,
};
pub use vm::{collection_count, set_heap_flags, set_root_ranges};
