#![cfg_attr(target_arch = "wasm32", no_std)]

//! The Wasm template of the ExS runtime v2 core library.
//!
//! Built for `wasm32-unknown-unknown` as a `cdylib` by `build-template.nu`, this crate exports
//! the stateless `exs-core` functions under `__exs_core_*` names for the v2 compiler backend to
//! link against. Every function receives the address of the module's VM state block and traps
//! instead of returning an error. On other targets it only provides the committed template.

#[cfg(target_arch = "wasm32")]
mod exports;

/// The committed Wasm template linked into every module compiled by the v2 backend.
pub const WASM_TEMPLATE: &[u8] = include_bytes!("../exs-core-template.wasm");
