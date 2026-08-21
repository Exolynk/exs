//! Wasmtime imports implementing the dynamic ExS Host ABI.

use std::collections::{BTreeMap, HashMap};

use exs_abi::{
    ErrorSeverity, ExsError, ExsValue, HOST_CALL_PENDING, HOST_CALL_READY,
    HOST_CALL_RESPONSE_COPY_IMPORT, HOST_CALL_RESPONSE_LENGTH_IMPORT, HOST_CALL_START_IMPORT,
    HOST_IMPORT_MODULE, HOST_SLEEP_HOST_NAME, RUNNER_IMPORT_MODULE, RUNNER_TASK_ACQUIRE_IMPORT,
    RUNNER_TASK_RELEASE_IMPORT, SourcePositionId,
};
use wasmtime::{Caller, Extern, Linker, ResourceLimiter, StoreLimits, StoreLimitsBuilder};

use crate::{
    ExecutionLimits, HostCborError, HostFunctionRegistry, HostFuture, LimitKind, RegistryError,
    decode_arguments_with_limits, encode_result_with_limits,
};

/// Runner-owned state accessed by imported host functions for one Wasm instance.
pub(crate) struct HostAbiState {
    /// Registry used to dynamically resolve language-provided host names.
    registry: HostFunctionRegistry,
    /// CBOR responses that completed synchronously and await runtime retrieval.
    ready_responses: HashMap<i64, Vec<u8>>,
    /// Futures that have suspended a runtime task and await later runner resumption.
    pending_calls: BTreeMap<i64, HostFuture>,
    /// Number of active language tasks currently holding runner permits.
    active_tasks: usize,
    /// Number of host calls started during this root execution.
    host_calls_started: usize,
    /// Number of host calls currently awaiting asynchronous completion.
    pending_host_calls: usize,
    /// Policy applied to each host-boundary payload.
    limits: ExecutionLimits,
    /// Wasmtime-owned resource limiter for this instance's linear memory.
    store_limits: StoreLimits,
    /// The most recent hard-limit violation reported through a Wasm import.
    limit_violation: Option<LimitKind>,
}

impl HostAbiState {
    /// Creates one isolated Host ABI state from a runner-owned registry.
    pub(crate) fn new(registry: HostFunctionRegistry, limits: ExecutionLimits) -> Self {
        Self {
            registry,
            ready_responses: HashMap::new(),
            pending_calls: BTreeMap::new(),
            active_tasks: 0,
            host_calls_started: 0,
            pending_host_calls: 0,
            store_limits: StoreLimitsBuilder::new()
                .memory_size(limits.max_memory_bytes)
                .trap_on_grow_failure(true)
                .build(),
            limits,
            limit_violation: None,
        }
    }

    /// Removes every pending host future together with its runtime-assigned identifier.
    pub(crate) fn take_pending_all(&mut self) -> Vec<(i64, HostFuture)> {
        std::mem::take(&mut self.pending_calls)
            .into_iter()
            .collect()
    }

    /// Removes and returns the hard-limit violation reported by a host ABI import.
    pub(crate) fn take_limit_violation(&mut self) -> Option<LimitKind> {
        self.limit_violation.take()
    }

    /// Returns the Wasmtime store limiter for this isolated execution.
    pub(crate) fn store_limits(&mut self) -> &mut dyn ResourceLimiter {
        &mut self.store_limits
    }

    /// Records one hard-limit violation before trapping back through Wasm.
    fn report_limit_violation(&mut self, kind: LimitKind) {
        self.limit_violation = Some(kind);
    }

    /// Acquires one active language-task permit when the configured budget allows it.
    fn acquire_task(&mut self) -> bool {
        if self.active_tasks >= self.limits.max_tasks {
            self.report_limit_violation(LimitKind::Tasks);
            return false;
        }
        let Some(next) = self.active_tasks.checked_add(1) else {
            self.report_limit_violation(LimitKind::Tasks);
            return false;
        };
        self.active_tasks = next;
        true
    }

    /// Releases one active language-task permit and reports unmatched releases as ABI failures.
    fn release_task(&mut self) -> bool {
        let Some(next) = self.active_tasks.checked_sub(1) else {
            return false;
        };
        self.active_tasks = next;
        true
    }

    /// Counts one started host call against the root execution's total call budget.
    fn start_host_call(&mut self) -> bool {
        if self.host_calls_started >= self.limits.max_host_calls {
            self.report_limit_violation(LimitKind::HostCalls);
            return false;
        }
        let Some(next) = self.host_calls_started.checked_add(1) else {
            self.report_limit_violation(LimitKind::HostCalls);
            return false;
        };
        self.host_calls_started = next;
        true
    }

    /// Acquires one concurrent pending-host-call slot.
    fn acquire_pending_host_call(&mut self) -> bool {
        if self.pending_host_calls >= self.limits.max_pending_host_calls {
            self.report_limit_violation(LimitKind::PendingHostCalls);
            return false;
        }
        let Some(next) = self.pending_host_calls.checked_add(1) else {
            self.report_limit_violation(LimitKind::PendingHostCalls);
            return false;
        };
        self.pending_host_calls = next;
        true
    }

    /// Releases one concurrent pending-host-call slot after its completion is selected.
    pub(crate) fn complete_pending_host_call(&mut self) -> bool {
        let Some(next) = self.pending_host_calls.checked_sub(1) else {
            return false;
        };
        self.pending_host_calls = next;
        true
    }
}

/// Defines the stable Host ABI imports required by one linked ExS module.
///
/// # Errors
///
/// Returns an error when Wasmtime rejects one import definition.
pub(crate) fn define(linker: &mut Linker<HostAbiState>) -> Result<(), wasmtime::Error> {
    linker.func_wrap(HOST_IMPORT_MODULE, HOST_CALL_START_IMPORT, host_call_start)?;
    linker.func_wrap(
        HOST_IMPORT_MODULE,
        HOST_CALL_RESPONSE_LENGTH_IMPORT,
        host_call_response_length,
    )?;
    linker.func_wrap(
        HOST_IMPORT_MODULE,
        HOST_CALL_RESPONSE_COPY_IMPORT,
        host_call_response_copy,
    )?;
    linker.func_wrap(
        RUNNER_IMPORT_MODULE,
        RUNNER_TASK_ACQUIRE_IMPORT,
        task_acquire,
    )?;
    linker.func_wrap(
        RUNNER_IMPORT_MODULE,
        RUNNER_TASK_RELEASE_IMPORT,
        task_release,
    )?;
    Ok(())
}

/// Acquires one generic active-task permit for an instrumented language runtime.
fn task_acquire(mut caller: Caller<'_, HostAbiState>) -> i32 {
    i32::from(!caller.data_mut().acquire_task())
}

/// Releases one generic active-task permit for an instrumented language runtime.
fn task_release(mut caller: Caller<'_, HostAbiState>) -> i32 {
    i32::from(!caller.data_mut().release_task())
}

/// Starts one dynamically resolved host function and records its ready result or future.
fn host_call_start(
    mut caller: Caller<'_, HostAbiState>,
    call_id: i64,
    name_pointer: i32,
    name_length: i32,
    request_pointer: i32,
    request_length: i32,
    source_position: i32,
) -> Result<i32, wasmtime::Error> {
    if !caller.data_mut().start_host_call() {
        return Err(wasmtime::Error::msg("host-call limit exceeded"));
    }
    let limits = caller.data().limits.clone();
    let name_bytes = read_memory(
        &mut caller,
        name_pointer,
        name_length,
        limits.max_cbor_payload_bytes,
    )?;
    let name = std::str::from_utf8(&name_bytes)
        .map_err(|_| wasmtime::Error::msg("host function name is not valid UTF-8"))?;
    let request = read_memory(
        &mut caller,
        request_pointer,
        request_length,
        limits.max_cbor_payload_bytes,
    )?;
    let arguments = decode_arguments_with_limits(&request, limits.cbor_limits())
        .map_err(|error| host_cbor_error(&mut caller, error))?;
    let origin = u32::try_from(source_position).ok().map(SourcePositionId);

    let call = if name == HOST_SLEEP_HOST_NAME {
        Ok(crate::host_sleep::start(arguments, origin))
    } else {
        caller.data().registry.start(name, arguments)
    };
    match call {
        Ok(crate::HostCall::Ready(value)) => {
            store_ready_response(&mut caller, call_id, value)?;
            Ok(HOST_CALL_READY)
        }
        Ok(crate::HostCall::Pending(future)) => {
            if caller.data().pending_calls.contains_key(&call_id) {
                return Err(wasmtime::Error::msg(
                    "runtime reused an active host-call identifier",
                ));
            }
            if !caller.data_mut().acquire_pending_host_call() {
                return Err(wasmtime::Error::msg("pending host-call limit exceeded"));
            }
            let previous = caller.data_mut().pending_calls.insert(call_id, future);
            if previous.is_some() {
                return Err(wasmtime::Error::msg(
                    "runtime reused an active host-call identifier",
                ));
            }
            Ok(HOST_CALL_PENDING)
        }
        Err(RegistryError::UnknownName(name)) => {
            store_ready_response(&mut caller, call_id, missing_host_error(name, origin))?;
            Ok(HOST_CALL_READY)
        }
        Err(error) => Err(wasmtime::Error::msg(format!(
            "host registry failed while starting a call: {error}"
        ))),
    }
}

/// Returns the byte length of one ready CBOR host response.
fn host_call_response_length(
    caller: Caller<'_, HostAbiState>,
    call_id: i64,
) -> Result<i32, wasmtime::Error> {
    let response = caller
        .data()
        .ready_responses
        .get(&call_id)
        .ok_or_else(|| wasmtime::Error::msg("host response is not ready"))?;
    i32::try_from(response.len())
        .map_err(|_| wasmtime::Error::msg("host response exceeds Wasm i32 length"))
}

/// Copies one ready CBOR host response into runtime-owned linear memory.
fn host_call_response_copy(
    mut caller: Caller<'_, HostAbiState>,
    call_id: i64,
    destination_pointer: i32,
    destination_length: i32,
) -> Result<i32, wasmtime::Error> {
    let response = caller
        .data_mut()
        .ready_responses
        .remove(&call_id)
        .ok_or_else(|| wasmtime::Error::msg("host response is not ready"))?;
    let destination_length = memory_range(destination_pointer, destination_length)?.1;
    if response.len() != destination_length {
        return Err(wasmtime::Error::msg(
            "runtime host-response buffer has an unexpected length",
        ));
    }
    let pointer = usize::try_from(destination_pointer)
        .map_err(|_| wasmtime::Error::msg("negative host-response destination pointer"))?;
    memory(&mut caller)?
        .write(&mut caller, pointer, &response)
        .map_err(|_| {
            wasmtime::Error::msg("runtime host-response buffer lies outside linear memory")
        })?;
    Ok(0)
}

/// Reads one validated byte range from the importing instance's exported memory.
fn read_memory(
    caller: &mut Caller<'_, HostAbiState>,
    pointer: i32,
    length: i32,
    maximum_length: usize,
) -> Result<Vec<u8>, wasmtime::Error> {
    let (pointer, length) = memory_range(pointer, length)?;
    if length > maximum_length {
        caller
            .data_mut()
            .report_limit_violation(LimitKind::CborPayload);
        return Err(wasmtime::Error::msg(
            "host-call payload exceeds configured limit",
        ));
    }
    let bytes = memory(caller)?
        .data(&*caller)
        .get(pointer..pointer + length)
        .ok_or_else(|| wasmtime::Error::msg("host-call buffer lies outside linear memory"))?;
    Ok(bytes.to_vec())
}

/// Validates an i32 Wasm pointer-length pair and converts it to a native range.
fn memory_range(pointer: i32, length: i32) -> Result<(usize, usize), wasmtime::Error> {
    let pointer = usize::try_from(pointer)
        .map_err(|_| wasmtime::Error::msg("negative host-call buffer pointer"))?;
    let length = usize::try_from(length)
        .map_err(|_| wasmtime::Error::msg("negative host-call buffer length"))?;
    let _end = pointer
        .checked_add(length)
        .ok_or_else(|| wasmtime::Error::msg("host-call buffer range overflow"))?;
    Ok((pointer, length))
}

/// Returns the importing instance's required exported linear memory.
fn memory(caller: &mut Caller<'_, HostAbiState>) -> Result<wasmtime::Memory, wasmtime::Error> {
    caller
        .get_export("memory")
        .and_then(Extern::into_memory)
        .ok_or_else(|| wasmtime::Error::msg("missing exported linear memory"))
}

/// Builds the recoverable language value used for an unregistered dynamic host name.
fn missing_host_error(name: String, origin: Option<SourcePositionId>) -> ExsValue {
    ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: "HostFunctionNotFound".to_owned(),
        message: format!("Host function `{name}` is not registered."),
        data: Box::new(ExsValue::None),
        origin,
        trace: Vec::new(),
        cause: None,
    })
}

/// Serializes a ready language value into the response table for one call identifier.
fn store_ready_response(
    caller: &mut Caller<'_, HostAbiState>,
    call_id: i64,
    value: ExsValue,
) -> Result<(), wasmtime::Error> {
    let limits = caller.data().limits.clone();
    let response = encode_result_with_limits(&value, limits.cbor_limits())
        .map_err(|error| host_cbor_error(caller, error))?;
    if response.len() > limits.max_cbor_payload_bytes {
        caller
            .data_mut()
            .report_limit_violation(LimitKind::CborPayload);
        return Err(wasmtime::Error::msg(
            "host response exceeds configured CBOR payload limit",
        ));
    }
    let previous = caller.data_mut().ready_responses.insert(call_id, response);
    if previous.is_some() {
        return Err(wasmtime::Error::msg(
            "runtime reused an active host-call identifier",
        ));
    }
    Ok(())
}

/// Converts a host-boundary CBOR failure into a technical Wasmtime failure.
fn host_cbor_error(caller: &mut Caller<'_, HostAbiState>, error: HostCborError) -> wasmtime::Error {
    match error {
        HostCborError::Invalid(exs_abi::CborError::PayloadLimitExceeded) => {
            caller
                .data_mut()
                .report_limit_violation(LimitKind::CborPayload);
        }
        HostCborError::Invalid(exs_abi::CborError::NestingLimitExceeded) => {
            caller
                .data_mut()
                .report_limit_violation(LimitKind::CborNesting);
        }
        HostCborError::Invalid(exs_abi::CborError::CollectionLimitExceeded) => {
            caller
                .data_mut()
                .report_limit_violation(LimitKind::CborCollectionEntries);
        }
        _ => {}
    }
    wasmtime::Error::msg(format!("could not process host CBOR: {error}"))
}
