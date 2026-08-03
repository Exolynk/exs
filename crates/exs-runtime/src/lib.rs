#![cfg_attr(all(feature = "runtime", target_arch = "wasm32"), no_std)]

//! `ExS` runtime code and its committed WebAssembly template.

#[cfg(all(feature = "runtime", target_arch = "wasm32"))]
extern crate alloc;

#[cfg(all(feature = "runtime", target_arch = "wasm32"))]
mod value;

#[cfg(all(feature = "runtime", target_arch = "wasm32"))]
mod gc;

#[cfg(all(feature = "runtime", target_arch = "wasm32"))]
mod runtime;

#[cfg(all(feature = "runtime", target_arch = "wasm32"))]
mod scheduler;

#[cfg(all(feature = "runtime", target_arch = "wasm32"))]
mod state;

#[cfg(all(feature = "runtime", target_arch = "wasm32"))]
mod wasm;

/// The committed runtime template linked into every compiled ExS module.
pub const WASM_TEMPLATE: &[u8] = include_bytes!("../exs-runtime.wasm");
