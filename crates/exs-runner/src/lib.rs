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
use std::sync::Arc;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
use std::task::Poll;

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
use self::limits::ExecutionLimits;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
use exs_abi::{
    ABI_VERSION, ABI_VERSION_EXPORT, BoundaryError, CANCEL_EXPORT, INPUT_ALLOC_EXPORT,
    RESULT_IMAGE_EXPORT, RESUME_HOST_EXPORT, RUNNER_IMPORT_MODULE, RUNNER_TASK_ACQUIRE_IMPORT,
    RUNNER_TASK_RELEASE_IMPORT, START_EXPORT_PREFIX, STATUS_CANCELLED, STATUS_COMPLETE,
    STATUS_PENDING,
};
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
use wasmtime::{Instance, Module, Store};

#[cfg(any(
    all(feature = "server", not(target_arch = "wasm32")),
    all(feature = "browser", target_arch = "wasm32")
))]
mod arguments;
#[cfg(all(feature = "browser", target_arch = "wasm32"))]
mod browser;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod cancellation;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod compiler;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod config;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod deadline;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod direct;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod guest_limiter;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod host_abi;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod host_function;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod host_sleep;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod host_time;
#[cfg(any(
    all(feature = "server", not(target_arch = "wasm32")),
    all(feature = "browser", target_arch = "wasm32")
))]
mod host_values;
mod limits;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod module_cache;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod registry;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod report;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod timer;
#[cfg(feature = "serde")]
mod typed_registry;
#[cfg(any(
    all(feature = "server", not(target_arch = "wasm32")),
    all(feature = "browser", target_arch = "wasm32")
))]
mod value_reader;

#[cfg(all(feature = "browser", target_arch = "wasm32"))]
pub use self::browser::{
    BrowserHostFunctionRegistry, BrowserHostStream, BrowserHostStreamFunction,
    BrowserHostStreamFuture, BrowserHostStreamItem, BrowserRegistryError, BrowserRunner,
    BrowserRunnerConfig,
};
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use self::cancellation::ExecutionCancellation;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use self::compiler::CompilerStrategy;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use self::config::ServerRunnerConfig;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use self::host_function::{
    AsyncHostFunction, HostCall, HostFuture, HostStream, HostStreamFunction, HostStreamFuture,
    HostStreamItem, SyncHostFunction,
};
pub use self::limits::{LimitKind, ProtectionLevel};
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use self::module_cache::SharedModuleCache;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use self::registry::{HostFunctionRegistry, RegistryError};
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use self::report::ExecutionReport;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use self::timer::RunnerTimer;
pub use exs_abi::{Bytes, ErrorSeverity, ExsError, ExsValue, SourcePositionId};

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
/// A reusable Wasmtime server runner with dynamically registered host functions.
pub struct ServerRunner {
    /// Runner-owned host implementations used for every execution.
    registry: HostFunctionRegistry,
    /// Resource policy applied to every native root execution.
    limits: ExecutionLimits,
    /// Engine and compiled modules, shared with every runner given the same cache.
    module_cache: SharedModuleCache,
    /// Timer that wakes sleeping and timed-out executions.
    timer: Arc<dyn RunnerTimer>,
}

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
impl ServerRunner {
    /// Creates a server runner with the policy and engine settings of `config` and no host
    /// functions.
    ///
    /// The runner uses `config.module_cache` when its engine matches the compiler and
    /// protection level of `config`, and otherwise a private cache of
    /// `config.module_cache_capacity` modules.
    #[must_use]
    pub fn new(config: ServerRunnerConfig) -> Self {
        let limits = ExecutionLimits::new(
            config.max_memory_bytes,
            config.max_fuel,
            config.timeout,
            config.protection,
        );
        let module_cache = match config.module_cache {
            Some(cache) if cache.fits(config.compiler, limits.max_wasm_stack_bytes) => cache,
            _ => SharedModuleCache::with_engine_settings(
                config.compiler,
                limits.max_wasm_stack_bytes,
                config.module_cache_capacity,
            ),
        };
        Self {
            registry: HostFunctionRegistry::new(),
            limits,
            module_cache,
            timer: config.timer.unwrap_or_else(|| Arc::new(timer::ThreadTimer)),
        }
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
        self.execute_with_report(wasm, function, inputs, cancellation)
            .await
            .map(|report| report.value)
    }

    /// Executes one named public function like [`Self::execute`] and reports consumed fuel.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::execute`].
    pub async fn execute_with_report(
        &self,
        wasm: &[u8],
        function: &str,
        inputs: &[ExsValue],
        cancellation: &ExecutionCancellation,
    ) -> Result<ExecutionReport, RunnerError> {
        if cancellation.is_cancelled() {
            return Err(RunnerError::Cancelled);
        }
        if wasm.len() > self.limits.max_module_bytes {
            return Err(RunnerError::LimitExceeded(LimitKind::Module));
        }
        let execution_started_at = std::time::Instant::now();
        let instance_pre = self
            .module_cache
            .instance_pre(wasm)
            .map_err(RunnerError::Wasm)?;
        check_task_metering_imports(instance_pre.module())?;
        let engine = self.module_cache.engine().map_err(RunnerError::Wasm)?;
        let deadline = deadline::ExecutionDeadline::new(self.timer.clone(), self.limits.timeout);
        let mut store = Store::new(
            engine,
            host_abi::HostAbiState::new(
                self.registry.clone(),
                self.limits.clone(),
                execution_started_at,
                deadline.expires_at(),
                self.timer.clone(),
            ),
        );
        store.limiter(host_abi::HostAbiState::store_limits);
        store
            .set_fuel(self.limits.max_fuel)
            .map_err(|error| RunnerError::Wasm(error.to_string()))?;
        store
            .fuel_async_yield_interval(Some(YIELD_FUEL))
            .map_err(|error| RunnerError::Wasm(error.to_string()))?;
        let instance = instance_pre
            .instantiate_async(&mut store)
            .await
            .map_err(wasmtime_error)?;
        check_abi(&mut store, &instance).await?;
        if direct::is_direct(&mut store, &instance) {
            store.data_mut().use_direct_boundary();
        } else if instance
            .get_typed_func::<(), i64>(&mut store, RESULT_IMAGE_EXPORT)
            .is_err()
        {
            return Err(RunnerError::Abi(format!(
                "module exports neither `{}` nor `{RESULT_IMAGE_EXPORT}`",
                exs_abi::RESULT_VALUE_EXPORT
            )));
        }
        let compile_time = execution_started_at.elapsed();
        let run_started_at = std::time::Instant::now();
        let (input_pointer, input_length) =
            write_input(&mut store, &instance, inputs, &self.limits).await?;
        let start = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, &entry_export_name(function))
            .map_err(|error| RunnerError::Abi(error.to_string()))?;
        let mut status = {
            if cancellation.is_cancelled() {
                return Err(RunnerError::Cancelled);
            }
            guarded(
                start.call_async(&mut store, (input_pointer, input_length)),
                &deadline,
                cancellation,
            )
            .await?
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
                    let result = if store.data().boundary() == host_abi::Boundary::Direct {
                        direct::result(&mut store, &instance, &self.limits).await?
                    } else {
                        result(&mut store, &instance, &self.limits).await?
                    };
                    let execution_time = run_started_at.elapsed();
                    if deadline.is_expired() {
                        return Err(RunnerError::LimitExceeded(LimitKind::Timeout));
                    }
                    let remaining_fuel = store
                        .get_fuel()
                        .map_err(|error| RunnerError::Wasm(error.to_string()))?;
                    return Ok(ExecutionReport {
                        value: result,
                        fuel_consumed: self.limits.max_fuel.saturating_sub(remaining_fuel),
                        compile_time,
                        execution_time,
                    });
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
                                Poll::Ready(Err(interruption)) => {
                                    return Poll::Ready(Err(interruption));
                                }
                                Poll::Pending => {}
                            }
                        }
                        Poll::Pending
                    })
                    .await
                    {
                        Ok(response) => response,
                        Err(cancellation::Interruption::Timeout) => {
                            return Err(RunnerError::LimitExceeded(LimitKind::Timeout));
                        }
                        Err(cancellation::Interruption::Cancelled) => {
                            cancel_execution(&mut store, &instance).await?;
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
                        cancel_execution(&mut store, &instance).await?;
                        return Err(RunnerError::Cancelled);
                    }
                    let (pointer, length) =
                        write_response(&mut store, &instance, &response, &self.limits).await?;
                    let resume = instance
                        .get_typed_func::<(i64, i32, i32), i32>(&mut store, RESUME_HOST_EXPORT)
                        .map_err(|error| RunnerError::Abi(error.to_string()))?;
                    status = {
                        if cancellation.is_cancelled() {
                            return Err(RunnerError::Cancelled);
                        }
                        guarded(
                            resume.call_async(&mut store, (call_id, pointer, length)),
                            &deadline,
                            cancellation,
                        )
                        .await?
                    }
                    .map_err(|error| execution_error(&mut store, &deadline, cancellation, error))?;
                }
                STATUS_CANCELLED => return Err(RunnerError::Cancelled),
                status => return Err(RunnerError::Status(status)),
            }
        }
    }
}

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
impl Default for ServerRunner {
    /// Creates a runner with the standard configuration.
    fn default() -> Self {
        Self::new(ServerRunnerConfig::default())
    }
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
async fn cancel_execution(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
) -> Result<(), RunnerError> {
    let cancel = instance
        .get_typed_func::<(), ()>(&mut *store, CANCEL_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?;
    cancel
        .call_async(&mut *store, ())
        .await
        .map_err(wasmtime_error)
}

/// Fuel the guest consumes between two yields to the executor.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
const YIELD_FUEL: u64 = 100_000;

/// Runs one guest call, which yields to the executor every [`YIELD_FUEL`], until it
/// returns; stops it when the deadline expires or the execution is cancelled.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
async fn guarded<Output>(
    call: impl Future<Output = wasmtime::Result<Output>>,
    deadline: &deadline::ExecutionDeadline,
    cancellation: &ExecutionCancellation,
) -> Result<wasmtime::Result<Output>, RunnerError> {
    let mut call = std::pin::pin!(call);
    poll_fn(|context| {
        if cancellation.is_cancelled() {
            return Poll::Ready(Err(RunnerError::Cancelled));
        }
        if deadline.is_expired() {
            return Poll::Ready(Err(RunnerError::LimitExceeded(LimitKind::Timeout)));
        }
        call.as_mut().poll(context).map(Ok)
    })
    .await
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
        // A module whose memory refused to grow traps once nothing is left to reclaim.
        None if store.data().memory_refused() => RunnerError::LimitExceeded(LimitKind::Memory),
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

/// Encodes one runner-owned value for the module as an image within the configured limits.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
fn encode_boundary_value(
    value: &ExsValue,
    limits: &ExecutionLimits,
    label: &str,
) -> Result<Vec<u8>, RunnerError> {
    value
        .to_image_with_limits(limits.boundary_limits())
        .map_err(|error| boundary_error(error, &format!("could not encode {label} image")))
}

/// Maps a boundary failure to a typed runner limit or an ABI diagnostic.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
fn boundary_error(error: BoundaryError, context: &str) -> RunnerError {
    match error {
        BoundaryError::PayloadLimitExceeded => RunnerError::LimitExceeded(LimitKind::Payload),
        BoundaryError::NestingLimitExceeded => RunnerError::LimitExceeded(LimitKind::Nesting),
        BoundaryError::CollectionLimitExceeded => {
            RunnerError::LimitExceeded(LimitKind::CollectionEntries)
        }
        error => RunnerError::Abi(format!("{context}: {error}")),
    }
}

/// Encodes one completed asynchronous response into the runtime-owned reusable input buffer.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
async fn write_response(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
    response: &ExsValue,
    limits: &ExecutionLimits,
) -> Result<(i32, i32), RunnerError> {
    let bytes = encode_boundary_value(response, limits, "host response")?;
    let length = i32::try_from(bytes.len())
        .map_err(|_| RunnerError::Abi("host response exceeds Wasm i32 length".to_owned()))?;
    let allocate = instance
        .get_typed_func::<i32, i32>(&mut *store, INPUT_ALLOC_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?;
    let pointer = allocate
        .call_async(&mut *store, length)
        .await
        .map_err(wasmtime_error)?;
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

/// Decodes the result image of a completed Rust guest.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
async fn result(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
    limits: &ExecutionLimits,
) -> Result<ExsValue, RunnerError> {
    let location = call_result_accessor::<i64>(store, instance, RESULT_IMAGE_EXPORT)
        .await?
        .cast_unsigned();
    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or_else(|| RunnerError::Abi("missing exported linear memory".to_owned()))?;
    let pointer = (location & u64::from(u32::MAX)) as usize;
    let length = (location >> 32) as usize;
    let end = pointer
        .checked_add(length)
        .ok_or_else(|| RunnerError::Abi("result buffer range overflow".to_owned()))?;
    let bytes = memory
        .data(&*store)
        .get(pointer..end)
        .ok_or_else(|| RunnerError::Abi("result buffer lies outside linear memory".to_owned()))?;
    // The result limit applies to the payload size of the result.
    let mut result_limits = limits.boundary_limits();
    result_limits.max_payload_bytes = result_limits.max_payload_bytes.min(limits.max_result_bytes);
    ExsValue::from_image_with_limits(bytes, result_limits).map_err(|error| match error {
        BoundaryError::PayloadLimitExceeded => RunnerError::LimitExceeded(LimitKind::Result),
        error => boundary_error(error, "invalid result image"),
    })
}

/// Encodes ordered main arguments and writes them to the runtime-owned input buffer.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
async fn write_input(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
    inputs: &[ExsValue],
    limits: &ExecutionLimits,
) -> Result<(i32, i32), RunnerError> {
    let input = ExsValue::List(inputs.to_vec());
    let bytes = encode_boundary_value(&input, limits, "input")?;
    let length = i32::try_from(bytes.len())
        .map_err(|_| RunnerError::Abi("input exceeds Wasm i32 length".to_owned()))?;
    let allocate = instance
        .get_typed_func::<i32, i32>(&mut *store, INPUT_ALLOC_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?;
    let pointer = allocate
        .call_async(&mut *store, length)
        .await
        .map_err(wasmtime_error)?;
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
async fn call_result_accessor<Return>(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
    name: &str,
) -> Result<Return, RunnerError>
where
    Return: wasmtime::WasmResults + Send + Sync,
{
    instance
        .get_typed_func::<(), Return>(&mut *store, name)
        .map_err(|error| RunnerError::Abi(error.to_string()))?
        .call_async(&mut *store, ())
        .await
        .map_err(wasmtime_error)
}

/// Checks the versioned ABI before starting program code.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
async fn check_abi(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
) -> Result<(), RunnerError> {
    let version = instance
        .get_typed_func::<(), i32>(&mut *store, ABI_VERSION_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?
        .call_async(&mut *store, ())
        .await
        .map_err(wasmtime_error)?;
    if version != ABI_VERSION.cast_signed() {
        return Err(RunnerError::Abi(format!(
            "expected ABI version {ABI_VERSION}, received {version}"
        )));
    }
    Ok(())
}
