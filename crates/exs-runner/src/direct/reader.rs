//! Reading results and host-call arguments from a runtime v2 module's memory.
//!
//! The runner walks the value with the core library's encoder on a bounds-checked view of the
//! memory, so a corrupted heap can neither read outside the memory nor make the walk
//! unbounded: nesting is limited and the produced bytes have a budget. The walk produces the
//! runner codec's CBOR, which is decoded with the usual limits.

use exs_abi::{ErrorSeverity, ExsError, ExsValue, RESULT_VALUE_EXPORT, VM_EXPORT};
use exs_core::{CborError as CoreCborError, CborOutput, SliceMemory};
use wasmtime::{Instance, Store};

use crate::limits::ExecutionLimits;
use crate::{LimitKind, RunnerError, cbor_error, host_abi};

/// Why a value in module memory could not be read.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum ReadError {
    /// The value cannot cross the boundary, with the message of v1's SerializationError.
    Unsupported(&'static str),
    /// The encoded value exceeds the byte budget.
    TooLarge,
    /// The module's memory does not hold a well-formed value.
    Malformed,
}

/// Output of the walk: the runner codec's CBOR, bounded to `budget` bytes.
struct Bytes {
    /// The bytes produced so far.
    bytes: Vec<u8>,
    /// The most bytes the output may hold.
    budget: usize,
}

impl Bytes {
    /// Reserves room for `count` more bytes within the budget.
    fn reserve(&mut self, count: usize) -> Result<(), CoreCborError> {
        if self.bytes.len().saturating_add(count) > self.budget {
            return Err(CoreCborError::OutOfMemory);
        }
        Ok(())
    }
}

impl<'a> CborOutput<SliceMemory<'a>> for Bytes {
    fn bytes(&mut self, _memory: &mut SliceMemory<'a>, bytes: &[u8]) -> Result<(), CoreCborError> {
        self.reserve(bytes.len())?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn copy(
        &mut self,
        memory: &mut SliceMemory<'a>,
        source: u32,
        len: u32,
    ) -> Result<(), CoreCborError> {
        let len = len as usize;
        self.reserve(len)?;
        let bytes = exs_core::Memory::bytes(memory, source, len as u32);
        if bytes.len() != len {
            return Err(CoreCborError::Malformed);
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}

/// Encodes the value `raw` in the module memory `data`, whose VM state block is at `vm`, as
/// the runner codec's CBOR of at most `budget` bytes.
pub(crate) fn encode(data: &[u8], vm: u32, raw: u64, budget: usize) -> Result<Vec<u8>, ReadError> {
    let walk = std::panic::catch_unwind(|| {
        let mut memory = SliceMemory::new(data);
        let mut output = Bytes {
            bytes: Vec::new(),
            budget,
        };
        match exs_core::encode_value(&mut memory, vm, raw, &mut output) {
            Ok(()) => Ok(output.bytes),
            Err(CoreCborError::Unsupported) => Err(ReadError::Unsupported(
                exs_core::serialization_message(&memory, raw),
            )),
            Err(CoreCborError::OutOfMemory) => Err(ReadError::TooLarge),
            Err(CoreCborError::Malformed) => Err(ReadError::Malformed),
        }
    });
    walk.unwrap_or(Err(ReadError::Malformed))
}

/// Returns whether the module of `instance` uses the direct boundary.
pub(crate) fn is_direct(store: &mut Store<host_abi::HostAbiState>, instance: &Instance) -> bool {
    instance
        .get_typed_func::<(), i64>(&mut *store, RESULT_VALUE_EXPORT)
        .is_ok()
}

/// Returns the address of the VM state block of the direct-boundary module of `instance`.
fn vm_address(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
) -> Result<u32, RunnerError> {
    let vm = instance
        .get_typed_func::<(), i32>(&mut *store, VM_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?
        .call(&mut *store, ())
        .map_err(crate::wasmtime_error)?;
    Ok(vm.cast_unsigned())
}

/// Reads the completed result of the direct-boundary module of `instance`.
///
/// Like v1, a result that cannot cross the boundary becomes a fatal SerializationError
/// without origin and trace.
pub(crate) fn result(
    store: &mut Store<host_abi::HostAbiState>,
    instance: &Instance,
    limits: &ExecutionLimits,
) -> Result<ExsValue, RunnerError> {
    let raw = instance
        .get_typed_func::<(), i64>(&mut *store, RESULT_VALUE_EXPORT)
        .map_err(|error| RunnerError::Abi(error.to_string()))?
        .call(&mut *store, ())
        .map_err(crate::wasmtime_error)?
        .cast_unsigned();
    let vm = vm_address(store, instance)?;
    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or_else(|| RunnerError::Abi("missing exported linear memory".to_owned()))?;
    let bytes = match encode(memory.data(&*store), vm, raw, limits.max_result_bytes) {
        Ok(bytes) => bytes,
        Err(ReadError::Unsupported(message)) => {
            return Ok(ExsValue::Error(serialization_error(message, true)));
        }
        Err(ReadError::TooLarge) => return Err(RunnerError::LimitExceeded(LimitKind::Result)),
        Err(ReadError::Malformed) => {
            return Err(RunnerError::Abi(
                "result lies outside the module's memory".to_owned(),
            ));
        }
    };
    ExsValue::from_cbor_with_limits(&bytes, limits.cbor_limits())
        .map_err(|error| cbor_error(error, "invalid result"))
}

/// Returns v1's SerializationError with `message`: fatal for a result, recoverable for
/// host-call arguments.
fn serialization_error(message: &str, fatal: bool) -> ExsError {
    ExsError {
        severity: if fatal {
            ErrorSeverity::Fatal
        } else {
            ErrorSeverity::Recoverable
        },
        kind: "SerializationError".to_owned(),
        message: message.to_owned(),
        data: Box::new(ExsValue::None),
        origin: None,
        trace: Vec::new(),
        cause: None,
    }
}
