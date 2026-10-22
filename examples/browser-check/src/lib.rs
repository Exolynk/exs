//! Checks that the browser runner returns what the server runner returns.
//!
//! The same programs run as v1 and runtime v2 modules in the browser runner under Node and in
//! the server runner; `check.nu` builds the browser side, runs it and compares the results in
//! the native test `tests/server.rs`.

pub mod cases;
#[cfg(target_arch = "wasm32")]
mod node;
