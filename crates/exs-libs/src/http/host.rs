//! The asynchronous host function `http.request`.

#[cfg(all(feature = "browser", target_arch = "wasm32"))]
use exs_runner::{BrowserHostFunctionRegistry as Registry, BrowserRegistryError as RegistryError};
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
use exs_runner::{HostFunctionRegistry as Registry, RegistryError};

use exs_abi::ExsValue;

use super::request::{parse, response_value};
use super::transport::SharedTransport;
use crate::error::error;

/// Registers `http.request`, which sends the requests of scripts through `transport`.
pub(crate) fn register(
    registry: &mut Registry,
    transport: SharedTransport,
) -> Result<(), RegistryError> {
    registry.fn_async_raw("http.request", move |arguments: Vec<ExsValue>| {
        let request = parse(arguments);
        let transport = transport.clone();
        async move {
            let request = match request {
                Ok(request) => request,
                Err(error) => return error,
            };
            match transport.send(request).await {
                Ok(response) => response_value(response),
                Err(message) => error("HttpError", message),
            }
        }
    })
}
