#![no_std]

//! `ExS` runtime code and its committed WebAssembly template.

#[cfg(target_arch = "wasm32")]
extern crate alloc;

#[cfg(target_arch = "wasm32")]
mod value;

#[cfg(target_arch = "wasm32")]
mod gc;

#[cfg(target_arch = "wasm32")]
mod runtime;

#[cfg(target_arch = "wasm32")]
mod scheduler;

#[cfg(target_arch = "wasm32")]
mod state;

#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(not(target_arch = "wasm32"))]
/// The committed runtime template linked into every compiled ExS module.
pub const WASM_TEMPLATE: &[u8] = include_bytes!("../exs-runtime.wasm");
