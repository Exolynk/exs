//! Requests, responses, and the transport that sends them.

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

/// Largest response body a request accepts.
pub const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

/// Most redirects a request follows.
pub const MAX_REDIRECTS: usize = 5;

/// One HTTP request of a script.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRequest {
    /// Method in uppercase, such as `GET`.
    pub method: String,
    /// Absolute URL.
    pub url: String,
    /// Header names and values in the order the script gave them.
    pub headers: Vec<(String, String)>,
    /// Body; empty when the request has none.
    pub body: Vec<u8>,
    /// Milliseconds after which the request fails, if the script set a timeout.
    pub timeout_ms: Option<u64>,
}

/// The response to an [`HttpRequest`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpResponse {
    /// Status code.
    pub status: u16,
    /// Header names in lowercase with their values.
    pub headers: Vec<(String, String)>,
    /// Body, at most [`MAX_RESPONSE_BYTES`] long.
    pub body: Vec<u8>,
}

/// The future of a sent request: the response, or a message why it failed.
#[cfg(not(target_arch = "wasm32"))]
pub type TransportFuture = Pin<Box<dyn Future<Output = Result<HttpResponse, String>> + Send>>;
/// The future of a sent request: the response, or a message why it failed.
#[cfg(target_arch = "wasm32")]
pub type TransportFuture = Pin<Box<dyn Future<Output = Result<HttpResponse, String>>>>;

/// Sends the requests of scripts.
#[cfg(not(target_arch = "wasm32"))]
pub trait HttpTransport: Send + Sync {
    /// Sends `request` and returns its response.
    fn send(&self, request: HttpRequest) -> TransportFuture;
}

/// Sends the requests of scripts.
#[cfg(target_arch = "wasm32")]
pub trait HttpTransport {
    /// Sends `request` and returns its response.
    fn send(&self, request: HttpRequest) -> TransportFuture;
}

/// A transport shared by the host functions of one registry.
#[cfg(not(target_arch = "wasm32"))]
pub type SharedTransport = std::sync::Arc<dyn HttpTransport>;
/// A transport shared by the host functions of one registry.
#[cfg(target_arch = "wasm32")]
pub type SharedTransport = std::rc::Rc<dyn HttpTransport>;

/// The default transport: `reqwest` on native targets and `fetch` in browsers.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultTransport;

impl HttpTransport for DefaultTransport {
    /// Sends `request` with the platform's HTTP client.
    fn send(&self, request: HttpRequest) -> TransportFuture {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Box::pin(super::native::send(request))
        }
        #[cfg(target_arch = "wasm32")]
        {
            Box::pin(super::fetch::send(request))
        }
    }
}
