//! Wasmtime imports implementing the dynamic ExS Host ABI.

use std::collections::{BTreeMap, HashMap};

use exs_abi::{
    ErrorSeverity, ExsError, ExsValue, HOST_CALL_PENDING, HOST_CALL_READY,
    HOST_CALL_RESPONSE_COPY_IMPORT, HOST_CALL_RESPONSE_LENGTH_IMPORT, HOST_CALL_START_IMPORT,
    HOST_IMPORT_MODULE, SourcePositionId,
};
use wasmtime::{Caller, Extern, Linker};

use crate::{HostCborError, HostFunctionRegistry, HostFuture, RegistryError};

/// Runner-owned state accessed by imported host functions for one Wasm instance.
pub(crate) struct HostAbiState {
    /// Registry used to dynamically resolve language-provided host names.
    registry: HostFunctionRegistry,
    /// CBOR responses that completed synchronously and await runtime retrieval.
    ready_responses: HashMap<i64, Vec<u8>>,
    /// Futures that have suspended a runtime task and await later runner resumption.
    pending_calls: BTreeMap<i64, HostFuture>,
}

impl HostAbiState {
    /// Creates one isolated Host ABI state from a runner-owned registry.
    pub(crate) fn new(registry: HostFunctionRegistry) -> Self {
        Self {
            registry,
            ready_responses: HashMap::new(),
            pending_calls: BTreeMap::new(),
        }
    }

    /// Removes the next pending host future together with its runtime-assigned identifier.
    pub(crate) fn take_pending(&mut self) -> Option<(i64, HostFuture)> {
        let call_id = self.pending_calls.keys().next().copied()?;
        self.pending_calls
            .remove(&call_id)
            .map(|future| (call_id, future))
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
    Ok(())
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
    let name_bytes = read_memory(&mut caller, name_pointer, name_length)?;
    let name = std::str::from_utf8(&name_bytes)
        .map_err(|_| wasmtime::Error::msg("host function name is not valid UTF-8"))?;
    let request = read_memory(&mut caller, request_pointer, request_length)?;
    let arguments = crate::decode_arguments(&request)
        .map_err(|error| wasmtime::Error::msg(format!("invalid host-call request: {error}")))?;
    let origin = u32::try_from(source_position).ok().map(SourcePositionId);

    match caller.data().registry.start(name, arguments) {
        Ok(crate::HostCall::Ready(value)) => {
            store_ready_response(&mut caller, call_id, value)?;
            Ok(HOST_CALL_READY)
        }
        Ok(crate::HostCall::Pending(future)) => {
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
) -> Result<Vec<u8>, wasmtime::Error> {
    let (pointer, length) = memory_range(pointer, length)?;
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
    let response = crate::encode_result(&value).map_err(host_cbor_error)?;
    let previous = caller.data_mut().ready_responses.insert(call_id, response);
    if previous.is_some() {
        return Err(wasmtime::Error::msg(
            "runtime reused an active host-call identifier",
        ));
    }
    Ok(())
}

/// Converts a host-boundary CBOR failure into a technical Wasmtime failure.
fn host_cbor_error(error: HostCborError) -> wasmtime::Error {
    wasmtime::Error::msg(format!("could not encode host response: {error}"))
}
