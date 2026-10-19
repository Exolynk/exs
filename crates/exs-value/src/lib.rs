#![no_std]

//! Shared `ExS` values and their type metadata.
//!
//! [`ValueRef`] is the 64-bit tagged reference passed between compiler-generated code and the
//! runtime. Its bit layout, including the immediate encoding of None, Bool and small Ints, is
//! defined here once so the compiler and the runtime cannot disagree about it.

mod immediate;
pub mod layout;
mod reference;

#[cfg(test)]
mod tests;

pub use immediate::Immediate;
pub use reference::ValueRef;
