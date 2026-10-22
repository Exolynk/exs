//! The 64-bit tagged `ValueRef` and the encoding of immediate values.
//!
//! [`ValueRef`] is the reference generated code and the core library pass around. Its bit
//! layout, including the immediate encoding of None, Bool and small Ints, is defined here once
//! so the compiler and the core library cannot disagree about it.

mod immediate;
pub mod layout;
mod reference;

#[cfg(test)]
mod tests;

pub use immediate::Immediate;
pub use reference::ValueRef;
