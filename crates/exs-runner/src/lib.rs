//! Wasmtime-backed Phase-1 `ExS` runner.

use std::fmt;

use exs_abi::{
    ABI_VERSION, ABI_VERSION_EXPORT, INPUT_ALLOC_EXPORT, RESULT_LENGTH_EXPORT,
    RESULT_POINTER_EXPORT, START_EXPORT, STATUS_COMPLETE,
};
use wasmtime::{Engine, Instance, Module, Store};

pub use exs_abi::{ErrorSeverity, ExsError, ExsValue, SourcePositionId};

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
pub fn execute(wasm: &[u8], input: ExsValue) -> Result<ExsValue, RunnerError> {
    let engine = Engine::default();
    let module =
        Module::new(&engine, wasm).map_err(|error| RunnerError::Wasm(error.to_string()))?;
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[])
        .map_err(|error| RunnerError::Wasm(error.to_string()))?;
    check_abi(&mut store, &instance)?;
    let (input_pointer, input_length) = write_input(&mut store, &instance, input)?;
    let start = instance
        .get_typed_func::<(i32, i32), i32>(&mut store, START_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?;
    let status = start
        .call(&mut store, (input_pointer, input_length))
        .map_err(|error| RunnerError::Wasm(error.to_string()))?;
    if status != STATUS_COMPLETE {
        return Err(RunnerError::Status(status));
    }
    result(&mut store, &instance)
}

/// Decodes the runtime-owned completed result without exposing its `ValueRef` to the host.
fn result(store: &mut Store<()>, instance: &Instance) -> Result<ExsValue, RunnerError> {
    let pointer = call_result_accessor::<i32>(store, instance, RESULT_POINTER_EXPORT)?;
    let length = call_result_accessor::<i32>(store, instance, RESULT_LENGTH_EXPORT)?;
    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or_else(|| RunnerError::Abi("missing exported linear memory".to_owned()))?;
    let pointer = usize::try_from(pointer)
        .map_err(|_| RunnerError::Abi("negative result pointer".to_owned()))?;
    let length = usize::try_from(length)
        .map_err(|_| RunnerError::Abi("negative result length".to_owned()))?;
    let end = pointer
        .checked_add(length)
        .ok_or_else(|| RunnerError::Abi("result buffer range overflow".to_owned()))?;
    let bytes = memory
        .data(&*store)
        .get(pointer..end)
        .ok_or_else(|| RunnerError::Abi("result buffer lies outside linear memory".to_owned()))?;
    ExsValue::from_cbor(bytes)
        .map_err(|error| RunnerError::Abi(format!("invalid result CBOR: {error}")))
}

/// Encodes input and writes it to the runtime-owned linear-memory input buffer.
fn write_input(
    store: &mut Store<()>,
    instance: &Instance,
    input: ExsValue,
) -> Result<(i32, i32), RunnerError> {
    let bytes = input
        .to_cbor()
        .map_err(|error| RunnerError::Abi(format!("could not encode input CBOR: {error}")))?;
    let length = i32::try_from(bytes.len())
        .map_err(|_| RunnerError::Abi("input CBOR exceeds Wasm i32 length".to_owned()))?;
    let allocate = instance
        .get_typed_func::<i32, i32>(&mut *store, INPUT_ALLOC_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?;
    let pointer = allocate
        .call(&mut *store, length)
        .map_err(|error| RunnerError::Wasm(error.to_string()))?;
    let pointer = usize::try_from(pointer)
        .map_err(|_| RunnerError::Abi("negative input pointer".to_owned()))?;
    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or_else(|| RunnerError::Abi("missing exported linear memory".to_owned()))?;
    memory
        .write(&mut *store, pointer, &bytes)
        .map_err(|error| RunnerError::Wasm(error.to_string()))?;
    let pointer = i32::try_from(pointer)
        .map_err(|_| RunnerError::Abi("input pointer exceeds Wasm i32 range".to_owned()))?;
    Ok((pointer, length))
}

/// Calls one zero-argument runtime result accessor.
fn call_result_accessor<Return>(
    store: &mut Store<()>,
    instance: &Instance,
    name: &str,
) -> Result<Return, RunnerError>
where
    Return: wasmtime::WasmResults,
{
    instance
        .get_typed_func::<(), Return>(&mut *store, name)
        .map_err(|error| RunnerError::Abi(error.to_string()))?
        .call(&mut *store, ())
        .map_err(|error| RunnerError::Wasm(error.to_string()))
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
