//! Wasmtime-backed Phase-1 `ExS` runner.

use std::fmt;

use exs_abi::{
    ABI_VERSION, ABI_VERSION_EXPORT, RESULT_VALUE_EXPORT, START_EXPORT, STATUS_COMPLETE,
};
use exs_value::Value;
use wasmtime::{Engine, Instance, Module, Store};

/// A technical error from Wasm loading or execution.
#[derive(Debug)]
pub enum RunnerError {
    /// Wasmtime rejected the module or failed to instantiate it.
    Wasm(String),
    /// A mandatory ABI export was absent or incompatible.
    Abi(String),
    /// The module returned an unsupported execution status.
    Status(i32),
}

impl fmt::Display for RunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wasm(message) => write!(formatter, "WebAssembly error: {message}"),
            Self::Abi(message) => write!(formatter, "ABI error: {message}"),
            Self::Status(status) => write!(formatter, "unexpected execution status: {status}"),
        }
    }
}

impl std::error::Error for RunnerError {}

/// Executes a Phase-1 linked `ExS` module using Wasmtime.
///
/// # Errors
///
/// Returns an error when the module cannot be instantiated, violates the ABI, traps, or does not complete.
pub fn execute(wasm: &[u8]) -> Result<Value, RunnerError> {
    let engine = Engine::default();
    let module =
        Module::new(&engine, wasm).map_err(|error| RunnerError::Wasm(error.to_string()))?;
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[])
        .map_err(|error| RunnerError::Wasm(error.to_string()))?;
    check_abi(&mut store, &instance)?;
    let start = instance
        .get_typed_func::<(), i32>(&mut store, START_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?;
    let status = start
        .call(&mut store, ())
        .map_err(|error| RunnerError::Wasm(error.to_string()))?;
    if status != STATUS_COMPLETE {
        return Err(RunnerError::Status(status));
    }
    let result = instance
        .get_typed_func::<(), i64>(&mut store, RESULT_VALUE_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?
        .call(&mut store, ())
        .map_err(|error| RunnerError::Wasm(error.to_string()))?;
    Ok(Value::from_bits(result.cast_unsigned()))
}

/// Checks the versioned ABI before starting program code.
fn check_abi(store: &mut Store<()>, instance: &Instance) -> Result<(), RunnerError> {
    let version = instance
        .get_typed_func::<(), i32>(&mut *store, ABI_VERSION_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?
        .call(&mut *store, ())
        .map_err(|error| RunnerError::Wasm(error.to_string()))?;
    if version != ABI_VERSION.cast_signed() {
        return Err(RunnerError::Abi(format!(
            "expected ABI version {ABI_VERSION}, received {version}"
        )));
    }
    Ok(())
}
