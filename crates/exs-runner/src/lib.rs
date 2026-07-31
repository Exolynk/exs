//! Wasmtime-backed `ExS` server runner.

use std::fmt;

use exs_abi::{
    ABI_VERSION, ABI_VERSION_EXPORT, CANCEL_EXPORT, INPUT_ALLOC_EXPORT, RESULT_LENGTH_EXPORT,
    RESULT_POINTER_EXPORT, RESUME_HOST_EXPORT, START_EXPORT, STATUS_CANCELLED, STATUS_COMPLETE,
    STATUS_PENDING,
};
use wasmtime::{Engine, Instance, Linker, Module, Store};

mod cancellation;
mod cbor;
mod host_abi;
mod host_function;
mod registry;

pub use self::cancellation::ExecutionCancellation;
pub use self::cbor::{HostCborError, decode_arguments, encode_result};
pub use self::host_function::{AsyncHostFunction, HostCall, HostFuture, SyncHostFunction};
pub use self::registry::{HostFunctionRegistry, RegistryError};
pub use exs_abi::{ErrorSeverity, ExsError, ExsValue, SourcePositionId};

/// A reusable Wasmtime server runner with dynamically registered host functions.
#[derive(Default)]
pub struct ServerRunner {
    /// Runner-owned host implementations used for every execution.
    registry: HostFunctionRegistry,
}

impl ServerRunner {
    /// Creates a server runner with no registered host functions.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns mutable access to the runner-owned dynamic host-function registry.
    pub fn registry_mut(&mut self) -> &mut HostFunctionRegistry {
        &mut self.registry
    }

    /// Executes one linked module and awaits any asynchronous host completions.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid Wasm, ABI violations, engine traps, or an invalid suspend
    /// protocol. Recoverable language failures remain `Ok(ExsValue::Error(...))`.
    pub async fn execute(
        &self,
        wasm: &[u8],
        inputs: &[ExsValue],
        cancellation: &ExecutionCancellation,
    ) -> Result<ExsValue, RunnerError> {
        if cancellation.is_cancelled() {
            return Err(RunnerError::Cancelled);
        }
        let engine = Engine::default();
        let module =
            Module::new(&engine, wasm).map_err(|error| RunnerError::Wasm(error.to_string()))?;
        let mut store = Store::new(&engine, host_abi::HostAbiState::new(self.registry.clone()));
        let mut linker = Linker::new(&engine);
        host_abi::define(&mut linker).map_err(|error| RunnerError::Wasm(error.to_string()))?;
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|error| RunnerError::Wasm(error.to_string()))?;
        check_abi(&mut store, &instance)?;
        let (input_pointer, input_length) = write_input(&mut store, &instance, inputs)?;
        let start = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, START_EXPORT)
            .map_err(|error| RunnerError::Abi(error.to_string()))?;
        let mut status = start
            .call(&mut store, (input_pointer, input_length))
            .map_err(|error| RunnerError::Wasm(error.to_string()))?;
        loop {
            match status {
                STATUS_COMPLETE => return result(&mut store, &instance),
                STATUS_PENDING => {
                    let (call_id, future) = store.data_mut().take_pending().ok_or_else(|| {
                        RunnerError::Deadlock(
                            "program reported Pending without a runner host future".to_owned(),
                        )
                    })?;
                    let response = match cancellation::CancellableHostFuture::new(
                        future,
                        cancellation,
                    )
                    .await
                    {
                        Ok(response) => response,
                        Err(()) => {
                            cancel_execution(&mut store, &instance)?;
                            return Err(RunnerError::Cancelled);
                        }
                    };
                    if cancellation.is_cancelled() {
                        cancel_execution(&mut store, &instance)?;
                        return Err(RunnerError::Cancelled);
                    }
                    let (pointer, length) = write_response(&mut store, &instance, &response)?;
                    let resume = instance
                        .get_typed_func::<(i64, i32, i32), i32>(&mut store, RESUME_HOST_EXPORT)
                        .map_err(|error| RunnerError::Abi(error.to_string()))?;
                    status = resume
                        .call(&mut store, (call_id, pointer, length))
                        .map_err(|error| RunnerError::Wasm(error.to_string()))?;
                }
                STATUS_CANCELLED => return Err(RunnerError::Cancelled),
                status => return Err(RunnerError::Status(status)),
            }
        }
    }
}

/// A technical error from Wasm loading or execution.
#[derive(Debug)]
pub enum RunnerError {
    /// Wasmtime rejected the module or failed to instantiate it.
    Wasm(String),
    /// A mandatory ABI export was absent or incompatible.
    Abi(String),
    /// The runner cannot make progress because no host completion can resume the scheduler.
    Deadlock(String),
    /// The caller cancelled the currently pending execution.
    Cancelled,
    /// The module returned an unsupported execution status.
    Status(i32),
}

impl fmt::Display for RunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wasm(message) => write!(formatter, "WebAssembly error: {message}"),
            Self::Abi(message) => write!(formatter, "ABI error: {message}"),
            Self::Deadlock(message) => write!(formatter, "scheduler deadlock: {message}"),
            Self::Cancelled => formatter.write_str("execution cancelled"),
            Self::Status(status) => write!(formatter, "unexpected execution status: {status}"),
        }
    }
}

impl std::error::Error for RunnerError {}

/// Cancels the active scheduler task in one suspended resumable module.
fn cancel_execution(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
) -> Result<(), RunnerError> {
    let cancel = instance
        .get_typed_func::<(), ()>(&mut *store, CANCEL_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?;
    cancel
        .call(&mut *store, ())
        .map_err(|error| RunnerError::Wasm(error.to_string()))
}

/// Executes a linked `ExS` module using the supplied ordered main arguments.
///
/// # Errors
///
/// Returns an error when the module cannot be instantiated, violates the ABI, traps, or does not complete.
pub fn execute(wasm: &[u8], inputs: &[ExsValue]) -> Result<ExsValue, RunnerError> {
    execute_with_registry(wasm, inputs, HostFunctionRegistry::new())
}

/// Executes a module through the synchronous compatibility path with one registry snapshot.
fn execute_with_registry(
    wasm: &[u8],
    inputs: &[ExsValue],
    registry: HostFunctionRegistry,
) -> Result<ExsValue, RunnerError> {
    let engine = Engine::default();
    let module =
        Module::new(&engine, wasm).map_err(|error| RunnerError::Wasm(error.to_string()))?;
    let mut store = Store::new(&engine, host_abi::HostAbiState::new(registry));
    let mut linker = Linker::new(&engine);
    host_abi::define(&mut linker).map_err(|error| RunnerError::Wasm(error.to_string()))?;
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|error| RunnerError::Wasm(error.to_string()))?;
    check_abi(&mut store, &instance)?;
    let (input_pointer, input_length) = write_input(&mut store, &instance, inputs)?;
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

/// Encodes one completed asynchronous response into the runtime-owned reusable input buffer.
fn write_response(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
    response: &ExsValue,
) -> Result<(i32, i32), RunnerError> {
    let bytes = encode_result(response).map_err(|error| {
        RunnerError::Abi(format!("could not encode host response CBOR: {error}"))
    })?;
    let length = i32::try_from(bytes.len())
        .map_err(|_| RunnerError::Abi("host response exceeds Wasm i32 length".to_owned()))?;
    let allocate = instance
        .get_typed_func::<i32, i32>(&mut *store, INPUT_ALLOC_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?;
    let pointer = allocate
        .call(&mut *store, length)
        .map_err(|error| RunnerError::Wasm(error.to_string()))?;
    let pointer_usize = usize::try_from(pointer)
        .map_err(|_| RunnerError::Abi("negative response pointer".to_owned()))?;
    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or_else(|| RunnerError::Abi("missing exported linear memory".to_owned()))?;
    memory
        .write(&mut *store, pointer_usize, &bytes)
        .map_err(|error| RunnerError::Wasm(error.to_string()))?;
    Ok((pointer, length))
}

/// Decodes the runtime-owned completed result without exposing its `ValueRef` to the host.
fn result(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
) -> Result<ExsValue, RunnerError> {
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

/// Encodes ordered main arguments and writes them to the runtime-owned input buffer.
fn write_input(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
    inputs: &[ExsValue],
) -> Result<(i32, i32), RunnerError> {
    let bytes = ExsValue::List(inputs.to_vec())
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
    store: &mut Store<host_abi::HostAbiState>,
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
fn check_abi(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
) -> Result<(), RunnerError> {
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
