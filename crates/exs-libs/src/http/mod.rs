//! The `http` library: sends HTTP requests and reads their responses.
//!
//! Requests go through an [`HttpTransport`]. The default transport uses `reqwest` on native
//! targets, which needs a tokio runtime, and `fetch` in browsers. An application can replace it,
//! for example to send browser requests through its server.

#[cfg(any(
    all(feature = "server", not(target_arch = "wasm32")),
    all(feature = "browser", target_arch = "wasm32")
))]
mod host;
mod library;
mod request;
mod transport;

#[cfg(target_arch = "wasm32")]
mod fetch;
#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(any(
    all(feature = "server", not(target_arch = "wasm32")),
    all(feature = "browser", target_arch = "wasm32")
))]
pub(crate) use host::register;
pub(crate) use library::SOURCE;
#[cfg(not(target_arch = "wasm32"))]
pub use native::send;
pub use transport::{
    DefaultTransport, HttpRequest, HttpResponse, HttpTransport, MAX_REDIRECTS, MAX_RESPONSE_BYTES,
    SharedTransport, TransportFuture,
};
