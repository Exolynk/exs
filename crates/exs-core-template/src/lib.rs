#![cfg_attr(all(target_arch = "wasm32", feature = "template"), no_std)]

//! The Wasm template of the ExS core library.
//!
//! Built for `wasm32-unknown-unknown` as a `cdylib` by `build-template.nu`, this crate exports
//! the stateless `exs-core` functions under `__exs_core_*` names for the compiler to link
//! against. Every function receives the address of the module's VM state block and traps
//! instead of returning an error. Without the `template` feature, as when the compiler runs in
//! a browser, it only provides the committed template.

#[cfg(all(target_arch = "wasm32", feature = "template"))]
mod exports;

/// The committed Wasm template linked into every compiled module.
pub const WASM_TEMPLATE: &[u8] = include_bytes!("../exs-core-template.wasm");
