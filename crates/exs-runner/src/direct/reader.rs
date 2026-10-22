//! Reading results and host-call arguments from a compiled module's memory.
//!
//! The runner walks the value with the core library on a bounds-checked view of the memory and
//! builds the value directly, so a corrupted heap can neither read outside the memory nor make
//! the walk unbounded: nesting is limited and the payload has a byte budget.

use exs_abi::{BoundaryLimits, ExsValue, RESULT_VALUE_EXPORT, VM_EXPORT};
use exs_core::SliceMemory;
use wasmtime::{Instance, Store};

use crate::limits::ExecutionLimits;
use crate::value_reader::{ReadError, read_value, serialization_error};
use crate::{LimitKind, RunnerError, host_abi};

/// Reads the value `raw` of the module memory `data`, whose VM state block is at `vm`, on a
/// bounds-checked view, within a payload of `budget` bytes and `limits`. A corrupted
/// heap that makes the walk panic reads as malformed.
pub(crate) fn read(
    data: &[u8],
    vm: u32,
    raw: u64,
    budget: usize,
    limits: BoundaryLimits,
) -> Result<ExsValue, ReadError> {
    std::panic::catch_unwind(|| read_value(&SliceMemory::new(data), vm, raw, budget, limits))
        .unwrap_or(Err(ReadError::Malformed))
}

/// Returns the limit a value exceeded, for the read errors that are limits.
pub(crate) const fn limit_kind(error: ReadError) -> Option<LimitKind> {
    match error {
        ReadError::PayloadLimitExceeded => Some(LimitKind::Payload),
        ReadError::NestingLimitExceeded => Some(LimitKind::Nesting),
        ReadError::CollectionLimitExceeded => Some(LimitKind::CollectionEntries),
        ReadError::Unsupported(_) | ReadError::TooLarge | ReadError::Malformed => None,
    }
}

/// Returns whether the module of `instance` uses the direct boundary.
pub(crate) fn is_direct(store: &mut Store<host_abi::HostAbiState>, instance: &Instance) -> bool {
    instance
        .get_typed_func::<(), i64>(&mut *store, RESULT_VALUE_EXPORT)
        .is_ok()
}

/// Returns the address of the VM state block of the direct-boundary module of `instance`.
async fn vm_address(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
) -> Result<u32, RunnerError> {
    let vm = instance
        .get_typed_func::<(), i32>(&mut *store, VM_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?
        .call_async(&mut *store, ())
        .await
        .map_err(crate::wasmtime_error)?;
    Ok(vm.cast_unsigned())
}

/// Reads the completed result of the direct-boundary module of `instance`.
///
/// A result that cannot cross the boundary becomes a fatal SerializationError
/// without origin and trace.
pub(crate) async fn result(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
    limits: &ExecutionLimits,
) -> Result<ExsValue, RunnerError> {
    let raw = instance
        .get_typed_func::<(), i64>(&mut *store, RESULT_VALUE_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?
        .call_async(&mut *store, ())
        .await
        .map_err(crate::wasmtime_error)?
        .cast_unsigned();
    let vm = vm_address(store, instance).await?;
    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or_else(|| RunnerError::Abi("missing exported linear memory".to_owned()))?;
    match read(
        memory.data(&*store),
        vm,
        raw,
        limits.max_result_bytes,
        limits.boundary_limits(),
    ) {
        Ok(value) => Ok(value),
        Err(ReadError::Unsupported(message)) => {
            Ok(ExsValue::Error(serialization_error(message, true)))
        }
        Err(ReadError::TooLarge) => Err(RunnerError::LimitExceeded(LimitKind::Result)),
        Err(ReadError::Malformed) => Err(RunnerError::Abi(
            "result lies outside the module's memory or is malformed".to_owned(),
        )),
        Err(error) => Err(RunnerError::LimitExceeded(
            limit_kind(error).unwrap_or(LimitKind::Result),
        )),
    }
}
