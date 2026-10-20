#![no_std]

//! The stateless core library of ExS runtime v2.
//!
//! Every function here works on state that lives in the module's linear memory, laid out by
//! `exs-layout`, and keeps nothing for itself: no `static` state and no allocator of its own.
//! All functions are generic over [`Memory`], so the same code runs directly on Wasm linear
//! memory and, in native tests, on a byte vector.
//!
//! This first part provides the heap: a size-class page allocator and a non-moving
//! mark-and-sweep collector.

#[cfg(test)]
extern crate std;

mod allocator;
mod collector;
mod memory;
mod pages;
mod vm;

#[cfg(test)]
mod tests;

pub use allocator::{allocate, init_heap};
pub use collector::collect;
#[cfg(target_arch = "wasm32")]
pub use memory::LinearMemory;
pub use memory::Memory;
pub use vm::{collection_count, set_heap_flags, set_root_ranges};
