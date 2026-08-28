//! Native and browser execution backends for compiled `ExS` modules.

#[cfg(all(feature = "server", not(feature = "browser"), target_arch = "wasm32"))]
compile_error!(
    "the `server` feature requires a native target; use `default-features = false, features = [\"browser\"]` for browser builds"
);

use std::fmt;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
use std::future::{Future, poll_fn};
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
use std::pin::Pin;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
use std::task::Poll;

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
use exs_abi::{
    ABI_VERSION, ABI_VERSION_EXPORT, CANCEL_EXPORT, CborError, INPUT_ALLOC_EXPORT,
    RESULT_LENGTH_EXPORT, RESULT_POINTER_EXPORT, RESUME_HOST_EXPORT, RUNNER_IMPORT_MODULE,
    RUNNER_TASK_ACQUIRE_IMPORT, RUNNER_TASK_RELEASE_IMPORT, START_EXPORT_PREFIX, STATUS_CANCELLED,
    STATUS_COMPLETE, STATUS_PENDING,
};
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
use wasmtime::{Config, Engine, Instance, Linker, Module, Store};

#[cfg(all(feature = "browser", target_arch = "wasm32"))]
mod browser;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod cancellation;
mod cbor;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod deadline;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod host_abi;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod host_function;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod host_sleep;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod host_time;
mod limits;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod registry;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod timer;

#[cfg(all(feature = "browser", target_arch = "wasm32"))]
pub use self::browser::{
    BrowserHostFunctionRegistry, BrowserHostStream, BrowserHostStreamFunction,
    BrowserHostStreamFuture, BrowserHostStreamItem, BrowserRegistryError, BrowserRunner,
    BrowserRunnerConfig,
};
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use self::cancellation::ExecutionCancellation;
pub use self::cbor::{
    HostCborError, decode_arguments, decode_arguments_with_limits, encode_result,
    encode_result_with_limits,
};
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use self::host_function::{
    AsyncHostFunction, HostCall, HostFuture, HostStream, HostStreamFunction, HostStreamFuture,
    HostStreamItem, SyncHostFunction,
};
pub use self::limits::{ExecutionLimits, LimitKind};
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use self::registry::{HostFunctionRegistry, RegistryError};
pub use exs_abi::{Bytes, ErrorSeverity, ExsError, ExsValue, SourcePositionId};

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
/// A reusable Wasmtime server runner with dynamically registered host functions.
pub struct ServerRunner {
    /// Runner-owned host implementations used for every execution.
    registry: HostFunctionRegistry,
    /// Resource policy applied to every native root execution.
    limits: ExecutionLimits,
}

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
impl ServerRunner {
    /// Creates a server runner with one explicit resource policy and no host functions.
    #[must_use]
    pub fn new(limits: ExecutionLimits) -> Self {
        Self {
            registry: HostFunctionRegistry::new(),
            limits,
        }
    }

    /// Returns the resource policy applied to each execution.
    #[must_use]
    pub fn limits(&self) -> &ExecutionLimits {
        &self.limits
    }

    /// Returns mutable access to the runner-owned dynamic host-function registry.
    pub fn registry_mut(&mut self) -> &mut HostFunctionRegistry {
        &mut self.registry
    }

    /// Executes one named public function in a linked module and awaits asynchronous host calls.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid Wasm, ABI violations, engine traps, or an invalid suspend
    /// protocol. Recoverable language failures remain `Ok(ExsValue::Error(...))`.
    pub async fn execute(
        &self,
        wasm: &[u8],
        function: &str,
        inputs: &[ExsValue],
        cancellation: &ExecutionCancellation,
    ) -> Result<ExsValue, RunnerError> {
        if cancellation.is_cancelled() {
            return Err(RunnerError::Cancelled);
        }
        let execution_started_at = std::time::Instant::now();
        let engine = limited_engine(&self.limits)?;
        let module =
            Module::new(&engine, wasm).map_err(|error| RunnerError::Wasm(error.to_string()))?;
        check_task_metering_imports(&module)?;
        let deadline = deadline::ExecutionDeadline::new(engine.clone(), self.limits.timeout)
            .map_err(|error| {
                RunnerError::Wasm(format!("could not start execution deadline: {error}"))
            })?;
        let mut store = Store::new(
            &engine,
            host_abi::HostAbiState::new(
                self.registry.clone(),
                self.limits.clone(),
                execution_started_at,
                deadline.expires_at(),
            ),
        );
        store.limiter(host_abi::HostAbiState::store_limits);
        store
            .set_fuel(self.limits.max_fuel)
            .map_err(|error| RunnerError::Wasm(error.to_string()))?;
        store.set_epoch_deadline(1);
        let mut linker = Linker::new(&engine);
        host_abi::define(&mut linker).map_err(|error| RunnerError::Wasm(error.to_string()))?;
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(wasmtime_error)?;
        check_abi(&mut store, &instance)?;
        let (input_pointer, input_length) =
            write_input(&mut store, &instance, inputs, &self.limits)?;
        let start = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, &entry_export_name(function))
            .map_err(|error| RunnerError::Abi(error.to_string()))?;
        let mut status = {
            let interruption = cancellation.register_interrupt(engine.clone());
            if interruption.is_cancelled() {
                return Err(RunnerError::Cancelled);
            }
            start.call(&mut store, (input_pointer, input_length))
        }
        .map_err(|error| execution_error(&mut store, &deadline, cancellation, error))?;
        let mut pending = Vec::new();
        loop {
            match status {
                STATUS_COMPLETE => {
                    if cancellation.is_cancelled() {
                        return Err(RunnerError::Cancelled);
                    }
                    if deadline.is_expired() {
                        return Err(RunnerError::LimitExceeded(LimitKind::Timeout));
                    }
                    let result = result(&mut store, &instance, &self.limits)?;
                    if deadline.is_expired() {
                        return Err(RunnerError::LimitExceeded(LimitKind::Timeout));
                    }
                    return Ok(result);
                }
                STATUS_PENDING => {
                    pending.extend(store.data_mut().take_pending_all().into_iter().map(
                        |(call_id, future)| {
                            (
                                call_id,
                                cancellation::CancellableHostFuture::new(
                                    future,
                                    cancellation,
                                    &deadline,
                                ),
                            )
                        },
                    ));
                    if pending.is_empty() {
                        return Err(RunnerError::Deadlock(
                            "program reported Pending without a runner host future".to_owned(),
                        ));
                    }
                    let (call_id, response) = match poll_fn(|context| {
                        for index in 0..pending.len() {
                            match Pin::new(&mut pending[index].1).poll(context) {
                                Poll::Ready(Ok(response)) => {
                                    let (call_id, _) = pending.swap_remove(index);
                                    return Poll::Ready(Ok((call_id, response)));
                                }
                                Poll::Ready(Err(())) => return Poll::Ready(Err(())),
                                Poll::Pending => {}
                            }
                        }
                        Poll::Pending
                    })
                    .await
                    {
                        Ok(response) => response,
                        Err(()) => {
                            if deadline.is_expired() {
                                return Err(RunnerError::LimitExceeded(LimitKind::Timeout));
                            }
                            cancel_execution(&mut store, &instance)?;
                            return Err(RunnerError::Cancelled);
                        }
                    };
                    if !store
                        .data_mut()
                        .complete_pending_host_call(call_id, &response)
                    {
                        return Err(RunnerError::Abi(
                            "runner completed an untracked pending host call".to_owned(),
                        ));
                    }
                    if cancellation.is_cancelled() {
                        cancel_execution(&mut store, &instance)?;
                        return Err(RunnerError::Cancelled);
                    }
                    let (pointer, length) =
                        write_response(&mut store, &instance, &response, &self.limits)?;
                    let resume = instance
                        .get_typed_func::<(i64, i32, i32), i32>(&mut store, RESUME_HOST_EXPORT)
                        .map_err(|error| RunnerError::Abi(error.to_string()))?;
                    status = {
                        let interruption = cancellation.register_interrupt(engine.clone());
                        if interruption.is_cancelled() {
                            return Err(RunnerError::Cancelled);
                        }
                        resume.call(&mut store, (call_id, pointer, length))
                    }
                    .map_err(|error| execution_error(&mut store, &deadline, cancellation, error))?;
                }
                STATUS_CANCELLED => return Err(RunnerError::Cancelled),
                status => return Err(RunnerError::Status(status)),
            }
        }
    }
}

/// Creates a native Wasmtime engine instrumented for one explicit runner policy.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
fn limited_engine(limits: &ExecutionLimits) -> Result<Engine, RunnerError> {
    let mut configuration = Config::new();
    configuration.consume_fuel(true);
    configuration.epoch_interruption(true);
    configuration.max_wasm_stack(limits.max_wasm_stack_bytes);
    Engine::new(&configuration).map_err(|error| RunnerError::Wasm(error.to_string()))
}

/// A technical error from Wasm loading or execution.
#[derive(Debug)]
pub enum RunnerError {
    /// The root execution exceeded one configured hard resource limit.
    LimitExceeded(LimitKind),
    /// The active WebAssembly backend rejected the module or failed to instantiate it.
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
            Self::LimitExceeded(kind) => write!(formatter, "execution limit exceeded: {kind}"),
            Self::Wasm(message) => write!(formatter, "WebAssembly error: {message}"),
            Self::Abi(message) => write!(formatter, "ABI error: {message}"),
            Self::Deadlock(message) => write!(formatter, "scheduler deadlock: {message}"),
            Self::Cancelled => formatter.write_str("execution cancelled"),
            Self::Status(status) => write!(formatter, "unexpected execution status: {status}"),
        }
    }
}

impl std::error::Error for RunnerError {}

/// Builds the stable Wasm export name for one requested root ExS function.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
fn entry_export_name(function: &str) -> String {
    format!("{START_EXPORT_PREFIX}{function}")
}

/// Cancels the active scheduler task in one suspended resumable module.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
fn cancel_execution(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
) -> Result<(), RunnerError> {
    let cancel = instance
        .get_typed_func::<(), ()>(&mut *store, CANCEL_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?;
    cancel.call(&mut *store, ()).map_err(wasmtime_error)
}

/// Converts a failed native Wasm call into a recorded limit error or technical runner error.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
fn execution_error(
    store: &mut Store<host_abi::HostAbiState>,
    deadline: &deadline::ExecutionDeadline,
    cancellation: &ExecutionCancellation,
    error: wasmtime::Error,
) -> RunnerError {
    if cancellation.is_cancelled() {
        return RunnerError::Cancelled;
    }
    if deadline.is_expired() {
        return RunnerError::LimitExceeded(LimitKind::Timeout);
    }
    match store.data_mut().take_limit_violation() {
        Some(kind) => RunnerError::LimitExceeded(kind),
        None => wasmtime_error(error),
    }
}

/// Converts a Wasmtime resource trap into the corresponding runner limit error.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
fn wasmtime_error(error: wasmtime::Error) -> RunnerError {
    let limit = match error.downcast_ref::<wasmtime::Trap>() {
        Some(wasmtime::Trap::OutOfFuel) => Some(LimitKind::Fuel),
        Some(wasmtime::Trap::Interrupt) => Some(LimitKind::Timeout),
        Some(wasmtime::Trap::StackOverflow) => Some(LimitKind::WasmStack),
        _ if error
            .to_string()
            .contains("forcing trap when growing memory") =>
        {
            Some(LimitKind::Memory)
        }
        _ => None,
    };
    match limit {
        Some(kind) => RunnerError::LimitExceeded(kind),
        None => RunnerError::Wasm(error.to_string()),
    }
}

/// Verifies that a module can be held to the generic runner active-task budget.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
fn check_task_metering_imports(module: &Module) -> Result<(), RunnerError> {
    let has_acquire = module.imports().any(|import| {
        import.module() == RUNNER_IMPORT_MODULE && import.name() == RUNNER_TASK_ACQUIRE_IMPORT
    });
    let has_release = module.imports().any(|import| {
        import.module() == RUNNER_IMPORT_MODULE && import.name() == RUNNER_TASK_RELEASE_IMPORT
    });
    if has_acquire && has_release {
        Ok(())
    } else {
        Err(RunnerError::Abi(
            "module does not import the required runner task-metering ABI".to_owned(),
        ))
    }
}

/// Encodes one runner-owned CBOR value and applies its configured byte and structural limits.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
fn encode_limited_cbor(
    value: &ExsValue,
    limits: &ExecutionLimits,
    size_kind: LimitKind,
    label: &str,
) -> Result<Vec<u8>, RunnerError> {
    let bytes = value
        .to_cbor_with_limits(limits.cbor_limits())
        .map_err(|error| cbor_error(error, &format!("could not encode {label} CBOR")))?;
    if bytes.len() > limits.max_cbor_payload_bytes {
        return Err(RunnerError::LimitExceeded(size_kind));
    }
    Ok(bytes)
}

/// Maps a bounded CBOR failure to a typed runner limit or an ABI diagnostic.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
fn cbor_error(error: CborError, context: &str) -> RunnerError {
    match error {
        CborError::PayloadLimitExceeded => RunnerError::LimitExceeded(LimitKind::CborPayload),
        CborError::NestingLimitExceeded => RunnerError::LimitExceeded(LimitKind::CborNesting),
        CborError::CollectionLimitExceeded => {
            RunnerError::LimitExceeded(LimitKind::CborCollectionEntries)
        }
        error => RunnerError::Abi(format!("{context}: {error}")),
    }
}

/// Encodes one completed asynchronous response into the runtime-owned reusable input buffer.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
fn write_response(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
    response: &ExsValue,
    limits: &ExecutionLimits,
) -> Result<(i32, i32), RunnerError> {
    let bytes = encode_limited_cbor(response, limits, LimitKind::CborPayload, "host response")?;
    let length = i32::try_from(bytes.len())
        .map_err(|_| RunnerError::Abi("host response exceeds Wasm i32 length".to_owned()))?;
    let allocate = instance
        .get_typed_func::<i32, i32>(&mut *store, INPUT_ALLOC_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?;
    let pointer = allocate.call(&mut *store, length).map_err(wasmtime_error)?;
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
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
fn result(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
    limits: &ExecutionLimits,
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
    if length > limits.max_result_bytes {
        return Err(RunnerError::LimitExceeded(LimitKind::Result));
    }
    let end = pointer
        .checked_add(length)
        .ok_or_else(|| RunnerError::Abi("result buffer range overflow".to_owned()))?;
    let bytes = memory
        .data(&*store)
        .get(pointer..end)
        .ok_or_else(|| RunnerError::Abi("result buffer lies outside linear memory".to_owned()))?;
    ExsValue::from_cbor_with_limits(bytes, limits.cbor_limits())
        .map_err(|error| cbor_error(error, "invalid result CBOR"))
}

/// Encodes ordered main arguments and writes them to the runtime-owned input buffer.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
fn write_input(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
    inputs: &[ExsValue],
    limits: &ExecutionLimits,
) -> Result<(i32, i32), RunnerError> {
    let input = ExsValue::List(inputs.to_vec());
    let bytes = encode_limited_cbor(&input, limits, LimitKind::CborPayload, "input")?;
    let length = i32::try_from(bytes.len())
        .map_err(|_| RunnerError::Abi("input CBOR exceeds Wasm i32 length".to_owned()))?;
    let allocate = instance
        .get_typed_func::<i32, i32>(&mut *store, INPUT_ALLOC_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?;
    let pointer = allocate.call(&mut *store, length).map_err(wasmtime_error)?;
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
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
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
        .map_err(wasmtime_error)
}

/// Checks the versioned ABI before starting program code.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
fn check_abi(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
) -> Result<(), RunnerError> {
    let version = instance
        .get_typed_func::<(), i32>(&mut *store, ABI_VERSION_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?
        .call(&mut *store, ())
        .map_err(wasmtime_error)?;
    if version != ABI_VERSION.cast_signed() {
        return Err(RunnerError::Abi(format!(
            "expected ABI version {ABI_VERSION}, received {version}"
        )));
    }
    Ok(())
}
