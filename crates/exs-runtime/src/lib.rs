#![cfg_attr(target_arch = "wasm32", no_std)]

//! `ExS` runtime code and its committed WebAssembly template.

extern crate alloc;

pub mod value;

pub use value::{RtValue, RuntimeList, RuntimeObject, RuntimeString};

#[cfg(target_arch = "wasm32")]
mod state;

#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(not(target_arch = "wasm32"))]
/// The committed runtime template linked into every compiled ExS module.
pub const WASM_TEMPLATE: &[u8] = include_bytes!("../exs-runtime.wasm");
