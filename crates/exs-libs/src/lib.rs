//! Optional ExS library modules with their host functions, ready for any embedding
//! application.
//!
//! Each Cargo feature enables one library: `json` converts between values and JSON text, `url`
//! encodes query strings and URL parts, `rand` returns secure random values, and `http` sends
//! HTTP requests (see [`http`] for its transport). Scripts name library items without imports, such
//! as `json::parse(text)`.
//!
//! An application integrates the enabled libraries in two places:
//!
//! - its `ModuleResolver` returns [`library`] (feature `compiler`) or [`source`] from
//!   `library(path)` and [`libraries`] from `libraries()`,
//! - its runner registers the host functions with [`register`].

// Without the `server` or `browser` feature only the library sources are used.
#![cfg_attr(
    not(any(
        all(feature = "server", not(target_arch = "wasm32")),
        all(feature = "browser", target_arch = "wasm32")
    )),
    allow(dead_code, unused_imports)
)]

mod error;
#[cfg(feature = "http")]
pub mod http;
#[cfg(feature = "json")]
mod json;
mod libraries;
#[cfg(feature = "rand")]
mod rand;
#[cfg(any(
    all(feature = "server", not(target_arch = "wasm32")),
    all(feature = "browser", target_arch = "wasm32")
))]
mod register;
#[cfg(feature = "url")]
mod url;

#[cfg(feature = "compiler")]
pub use libraries::library;
pub use libraries::{libraries, source};
#[cfg(any(
    all(feature = "server", not(target_arch = "wasm32")),
    all(feature = "browser", target_arch = "wasm32")
))]
pub use register::register;
#[cfg(all(
    feature = "http",
    any(
        all(feature = "server", not(target_arch = "wasm32")),
        all(feature = "browser", target_arch = "wasm32")
    )
))]
pub use register::register_with_http;
