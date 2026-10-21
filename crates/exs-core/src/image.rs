//! Adoption of the images in which the runner passes inputs and host-call responses.
//!
//! The runner lays values out as records of the `exs_abi::image` format. Adopting an image
//! walks its records from the root and builds the values on the heap: Strings and Bytes are
//! copied as they are, without parsing or UTF-8 validation. Every offset is checked against the
//! image, and nesting is bounded like the CBOR codec's.

use exs_abi::image::{
    IMAGE_HEADER_SIZE, IMAGE_MAGIC, RECORD_BOOL, RECORD_BYTES, RECORD_ENUM, RECORD_ERROR,
    RECORD_FLOAT, RECORD_HEADER_SIZE, RECORD_INT, RECORD_LIST, RECORD_NONE, RECORD_OBJECT,
    RECORD_STRING,
};
use exs_value::ValueRef;

use crate::boundary::BoundaryError;
use crate::bytes::new_bytes;
use crate::enums::resolve_enum;
use crate::enums::{allocate_enum, field_address as enum_field, host_enum};
use crate::errors::{ErrorParts, error_from_parts, new_error};
use crate::execution::input;
use crate::lists::{allocate_list, list_reference, push_unchecked};
use crate::memory::Memory;
use crate::numbers::{float_value, int_value};
use crate::objects::{empty_object, find_value, object_append, object_reference, shape};
use crate::shapes::child_key;
use crate::strings::{copy_string, string_object};
use crate::vm::Vm;

/// Deepest nesting of values an image may hold, like the CBOR codec.
const MAX_DEPTH: u32 = 128;

/// One image in memory.
#[derive(Clone, Copy)]
struct Image {
    /// Address of the image.
    base: u32,
    /// Byte length of the image.
    len: u32,
}

/// One record of an image.
#[derive(Clone, Copy)]
struct Record {
    /// Address of the record's payload.
    payload: u32,
    /// The record kind.
    kind: u32,
    /// The record argument.
    argument: u32,
}

impl Image {
    /// Returns the image of `len` bytes at `base`, after checking its header.
    fn at(memory: &impl Memory, base: u32, len: u32) -> Result<Self, BoundaryError> {
        if len < IMAGE_HEADER_SIZE || memory.load_u32(base) != IMAGE_MAGIC {
            return Err(BoundaryError::Malformed);
        }
        base.checked_add(len).ok_or(BoundaryError::Malformed)?;
        Ok(Self { base, len })
    }

    /// Returns the offset of the root record.
    fn root(self, memory: &impl Memory) -> u32 {
        memory.load_u32(self.base + 4)
    }

    /// Returns the address of the `size` bytes at `offset`, which must lie in the image.
    fn range(self, offset: u32, size: u32) -> Result<u32, BoundaryError> {
        offset
            .checked_add(size)
            .filter(|end| *end <= self.len)
            .map(|_| self.base + offset)
            .ok_or(BoundaryError::Malformed)
    }

    /// Returns the record at `offset`, which must be an aligned record inside the image.
    fn record(self, memory: &impl Memory, offset: u32) -> Result<Record, BoundaryError> {
        if offset < IMAGE_HEADER_SIZE || !offset.is_multiple_of(8) {
            return Err(BoundaryError::Malformed);
        }
        let address = self.range(offset, RECORD_HEADER_SIZE)?;
        Ok(Record {
            payload: address + RECORD_HEADER_SIZE,
            kind: memory.load_u32(address),
            argument: memory.load_u32(address + 4),
        })
    }

    /// Returns the address of `size` payload bytes of `record`, which must lie in the image.
    fn payload(self, record: Record, size: u32) -> Result<u32, BoundaryError> {
        self.range(record.payload - self.base, size)
    }

    /// Returns the address and length of the String record at `offset`.
    fn text(self, memory: &impl Memory, offset: u32) -> Result<(u32, u32), BoundaryError> {
        let record = self.record(memory, offset)?;
        if record.kind != RECORD_STRING {
            return Err(BoundaryError::Malformed);
        }
        let start = self.payload(
            record,
            record
                .argument
                .checked_add(8)
                .ok_or(BoundaryError::Malformed)?,
        )?;
        Ok((start + 8, record.argument))
    }
}

/// Adopts the image of `length` bytes at `address`, returning its root value.
pub(crate) fn adopt_at(
    memory: &mut impl Memory,
    vm: u32,
    address: u32,
    length: u32,
) -> Result<u64, BoundaryError> {
    let image = Image::at(memory, address, length)?;
    let root = image.root(memory);
    adopt(memory, vm, image, root, 0)
}

/// Returns the input image and the record of its argument List.
fn input_list(memory: &impl Memory, vm: u32) -> Result<(Image, Record), BoundaryError> {
    let (address, length) = input(memory, vm);
    let image = Image::at(memory, address, length)?;
    let record = image.record(memory, image.root(memory))?;
    if record.kind != RECORD_LIST {
        return Err(BoundaryError::Malformed);
    }
    image.payload(
        record,
        record
            .argument
            .checked_mul(4)
            .ok_or(BoundaryError::Malformed)?,
    )?;
    Ok((image, record))
}

/// Adopts the runner's argument List into the root slots at `slots`.
///
/// Stores at most `capacity` arguments, filling the remaining slots with None, and returns the
/// number of arguments the input holds. The slots must be rooted, because adoption allocates.
///
/// # Errors
///
/// Returns why the input could not be adopted.
pub fn image_inputs(
    memory: &mut impl Memory,
    vm: u32,
    slots: u32,
    capacity: u32,
) -> Result<u32, BoundaryError> {
    let (image, list) = input_list(memory, vm)?;
    let count = list.argument;
    for index in 0..count {
        let offset = memory.load_u32(list.payload + index * 4);
        let raw = adopt(memory, vm, image, offset, 1)?;
        if index < capacity {
            memory.store_u64(slots + index * 8, raw);
        }
    }
    for index in count..capacity {
        memory.store_u64(slots + index * 8, ValueRef::NONE.raw());
    }
    Ok(count)
}

/// Adopts the runner's argument List for an entry with `fixed` parameters and a trailing
/// variadic one: the first `fixed` arguments into the root slots at `slots`, None for missing
/// ones, and a List of the rest into the slot after them, like v1.
///
/// # Errors
///
/// Returns why the input could not be adopted.
pub fn image_variadic_inputs(
    memory: &mut impl Memory,
    vm: u32,
    slots: u32,
    fixed: u32,
) -> Result<(), BoundaryError> {
    let (image, list) = input_list(memory, vm)?;
    let count = list.argument;
    for index in 0..fixed {
        memory.store_u64(slots + index * 8, ValueRef::NONE.raw());
    }
    let rest =
        allocate_list(memory, vm, count.saturating_sub(fixed)).ok_or(BoundaryError::OutOfMemory)?;
    let rest_slot = slots + fixed * 8;
    memory.store_u64(
        rest_slot,
        list_reference(rest).ok_or(BoundaryError::OutOfMemory)?,
    );
    for index in 0..count {
        let offset = memory.load_u32(list.payload + index * 4);
        let raw = adopt(memory, vm, image, offset, 1)?;
        if index < fixed {
            memory.store_u64(slots + index * 8, raw);
        } else {
            push_unchecked(memory, rest, raw);
        }
    }
    Ok(())
}

/// Creates the fatal ArityError of an entry that received more arguments than it declares,
/// with all arguments as a List in its data, like v1.
///
/// # Errors
///
/// Returns why the input could not be adopted.
pub fn image_arity_error(memory: &mut impl Memory, vm: u32) -> Result<u64, BoundaryError> {
    let (address, length) = input(memory, vm);
    let arguments = adopt_at(memory, vm, address, length)?;
    new_error(
        memory,
        vm,
        true,
        "ArityError",
        "main received more input values than declared parameters",
        arguments,
    )
    .ok_or(BoundaryError::OutOfMemory)
}

/// Adopts the record at `offset` at nesting depth `depth`.
///
/// Every value adopted so far must be rooted, because adoption allocates.
fn adopt(
    memory: &mut impl Memory,
    vm: u32,
    image: Image,
    offset: u32,
    depth: u32,
) -> Result<u64, BoundaryError> {
    let record = image.record(memory, offset)?;
    match record.kind {
        RECORD_NONE => Ok(ValueRef::NONE.raw()),
        RECORD_BOOL => Ok(ValueRef::boolean(record.argument != 0).raw()),
        RECORD_INT => {
            let value = memory.load_u64(image.payload(record, 8)?).cast_signed();
            int_value(memory, vm, value).ok_or(BoundaryError::OutOfMemory)
        }
        RECORD_FLOAT => {
            let value = f64::from_bits(memory.load_u64(image.payload(record, 8)?));
            float_value(memory, vm, value).ok_or(BoundaryError::OutOfMemory)
        }
        RECORD_STRING => {
            let (start, len) = image.text(memory, offset)?;
            let chars = memory.load_u32(start - 8);
            copy_string(memory, vm, start, len, chars).ok_or(BoundaryError::OutOfMemory)
        }
        RECORD_BYTES => {
            let start = image.payload(record, record.argument)?;
            let len = record.argument;
            new_bytes(memory, vm, len, |memory, position| {
                memory.copy(position, start, len);
            })
            .ok_or(BoundaryError::OutOfMemory)
        }
        _ if depth >= MAX_DEPTH => Err(BoundaryError::Unsupported),
        RECORD_LIST => adopt_list(memory, vm, image, record, depth),
        RECORD_OBJECT => adopt_object(memory, vm, image, record, depth),
        RECORD_ENUM => adopt_enum(memory, vm, image, record, depth),
        RECORD_ERROR => adopt_error(memory, vm, image, record, depth),
        _ => Err(BoundaryError::Malformed),
    }
}

/// Reads `count` `u32` offsets of `record`'s payload, starting at word `first`.
fn offsets(image: Image, record: Record, first: u32, count: u32) -> Result<u32, BoundaryError> {
    let words = first.checked_add(count).ok_or(BoundaryError::Malformed)?;
    image.payload(
        record,
        words.checked_mul(4).ok_or(BoundaryError::Malformed)?,
    )?;
    Ok(record.payload + first * 4)
}

/// Adopts a List record into a new List.
fn adopt_list(
    memory: &mut impl Memory,
    vm: u32,
    image: Image,
    record: Record,
    depth: u32,
) -> Result<u64, BoundaryError> {
    let count = record.argument;
    let items = offsets(image, record, 0, count)?;
    let list = allocate_list(memory, vm, count).ok_or(BoundaryError::OutOfMemory)?;
    let raw = list_reference(list).ok_or(BoundaryError::OutOfMemory)?;
    let root = Vm(vm)
        .push_root(memory, raw)
        .ok_or(BoundaryError::OutOfMemory)?;
    let mut adopted = Ok(raw);
    for index in 0..count {
        let offset = memory.load_u32(items + index * 4);
        match adopt(memory, vm, image, offset, depth + 1) {
            Ok(item) => push_unchecked(memory, list, item),
            Err(error) => {
                adopted = Err(error);
                break;
            }
        }
    }
    Vm(vm).pop_roots(memory, root);
    adopted
}

/// Adopts an Object record into a new untyped Object, rejecting repeated keys.
fn adopt_object(
    memory: &mut impl Memory,
    vm: u32,
    image: Image,
    record: Record,
    depth: u32,
) -> Result<u64, BoundaryError> {
    let count = record.argument;
    let entries = offsets(
        image,
        record,
        0,
        count.checked_mul(2).ok_or(BoundaryError::Malformed)?,
    )?;
    let object = empty_object(memory, vm, count + 1).ok_or(BoundaryError::OutOfMemory)?;
    let raw = object_reference(object).ok_or(BoundaryError::OutOfMemory)?;
    let state = Vm(vm);
    let root = state
        .push_root(memory, raw)
        .ok_or(BoundaryError::OutOfMemory)?;
    let adopted = (|| {
        for index in 0..count {
            let entry = entries + index * 8;
            let (start, len) = image.text(memory, memory.load_u32(entry))?;
            // Objects of an existing key sequence share its key instead of a copy.
            let name = match child_key(memory, shape(memory, object), start, len) {
                Some(key) => key,
                None => {
                    let chars = memory.load_u32(start - 8);
                    copy_string(memory, vm, start, len, chars).ok_or(BoundaryError::OutOfMemory)?
                }
            };
            let mark = state
                .push_root(memory, name)
                .ok_or(BoundaryError::OutOfMemory)?;
            let value = adopt(memory, vm, image, memory.load_u32(entry + 4), depth + 1)?;
            if find_value(memory, object, name as u32).is_some() {
                return Err(BoundaryError::Malformed);
            }
            state
                .push_root(memory, value)
                .ok_or(BoundaryError::OutOfMemory)?;
            object_append(memory, vm, object, name, value).ok_or(BoundaryError::OutOfMemory)?;
            state.pop_roots(memory, mark);
        }
        Ok(raw)
    })();
    state.pop_roots(memory, root);
    adopted
}

/// Adopts an enum record of a type the program declares.
fn adopt_enum(
    memory: &mut impl Memory,
    vm: u32,
    image: Image,
    record: Record,
    depth: u32,
) -> Result<u64, BoundaryError> {
    let count = record.argument;
    let words = offsets(
        image,
        record,
        0,
        count.checked_add(2).ok_or(BoundaryError::Malformed)?,
    )?;
    let identity = image.text(memory, memory.load_u32(words))?;
    let variant = image.text(memory, memory.load_u32(words + 4))?;
    let state = Vm(vm);
    let mark = state.root_stack_end(memory);
    let raw = match resolve_enum(memory, vm, identity, variant) {
        Some((type_id, index)) => allocate_enum(memory, vm, type_id, index, count),
        None => {
            // A host enum keeps its identity and variant name.
            let mut names = [0_u64; 2];
            for (name, (start, len)) in names.iter_mut().zip([identity, variant]) {
                let chars = memory.load_u32(start - 8);
                *name =
                    copy_string(memory, vm, start, len, chars).ok_or(BoundaryError::OutOfMemory)?;
                state
                    .push_root(memory, *name)
                    .ok_or(BoundaryError::OutOfMemory)?;
            }
            host_enum(memory, vm, names[0], names[1], count)
        }
    }
    .ok_or(BoundaryError::OutOfMemory)?;
    state.pop_roots(memory, mark);
    let root = state
        .push_root(memory, raw)
        .ok_or(BoundaryError::OutOfMemory)?;
    let mut adopted = Ok(raw);
    for field in 0..count {
        let offset = memory.load_u32(words + (field + 2) * 4);
        match adopt(memory, vm, image, offset, depth + 1) {
            Ok(value) => memory.store_u64(enum_field(raw as u32, field), value),
            Err(error) => {
                adopted = Err(error);
                break;
            }
        }
    }
    Vm(vm).pop_roots(memory, root);
    adopted
}

/// Adopts an Error record with its kind, message, data, origin, trace and cause.
fn adopt_error(
    memory: &mut impl Memory,
    vm: u32,
    image: Image,
    record: Record,
    depth: u32,
) -> Result<u64, BoundaryError> {
    let words = offsets(image, record, 0, 6)?;
    let trace_count = memory.load_u32(words + 20);
    let trace_len = trace_count.checked_mul(8).ok_or(BoundaryError::Malformed)?;
    let trace_start = image.payload(
        record,
        trace_len.checked_add(24).ok_or(BoundaryError::Malformed)?,
    )? + 24;
    let state = Vm(vm);
    let root = state.root_stack_end(memory);
    let adopted = (|| {
        let mut texts = [0_u64; 2];
        for (word, text) in (0..).zip(&mut texts) {
            *text = adopt(
                memory,
                vm,
                image,
                memory.load_u32(words + word * 4),
                depth + 1,
            )?;
            if string_object(*text).is_none() {
                return Err(BoundaryError::Malformed);
            }
            state
                .push_root(memory, *text)
                .ok_or(BoundaryError::OutOfMemory)?;
        }
        let data = adopt(memory, vm, image, memory.load_u32(words + 8), depth + 1)?;
        state
            .push_root(memory, data)
            .ok_or(BoundaryError::OutOfMemory)?;
        let origin = memory.load_u32(words + 12).saturating_sub(1);
        let trace = if trace_count == 0 {
            0
        } else {
            let raw = new_bytes(memory, vm, trace_len, |memory, position| {
                memory.copy(position, trace_start, trace_len);
            })
            .ok_or(BoundaryError::OutOfMemory)?;
            state
                .push_root(memory, raw)
                .ok_or(BoundaryError::OutOfMemory)?;
            raw as u32
        };
        let cause = match memory.load_u32(words + 16) {
            0 => ValueRef::NONE.raw(),
            offset => adopt(memory, vm, image, offset, depth + 1)?,
        };
        let parts = ErrorParts {
            fatal: record.argument != 0,
            kind: texts[0],
            message: texts[1],
            data,
            origin,
            trace: (0, 0),
            cause,
        };
        error_from_parts(memory, vm, &parts, trace).ok_or(BoundaryError::OutOfMemory)
    })();
    state.pop_roots(memory, root);
    adopted
}
