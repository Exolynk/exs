//! Registration of the host functions of every enabled library.

#[cfg(all(feature = "browser", target_arch = "wasm32"))]
use exs_runner::{BrowserHostFunctionRegistry as Registry, BrowserRegistryError as RegistryError};
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
use exs_runner::{HostFunctionRegistry as Registry, RegistryError};

use crate::error::HostFunction;

/// Returns the host name and implementation of every host function of the enabled libraries.
fn host_functions() -> Vec<(&'static str, HostFunction)> {
    let mut functions: Vec<(&'static str, HostFunction)> = Vec::new();
    #[cfg(feature = "json")]
    functions.extend_from_slice(&crate::json::HOST_FUNCTIONS);
    #[cfg(feature = "url")]
    functions.extend_from_slice(&crate::url::HOST_FUNCTIONS);
    functions
}

/// Registers the host functions of every enabled library with the runner's registry.
///
/// # Errors
///
/// Returns an error when one of the host names is already registered.
pub fn register(registry: &mut Registry) -> Result<(), RegistryError> {
    for (name, function) in host_functions() {
        registry.fn_sync_raw(name, function)?;
    }
    Ok(())
}
