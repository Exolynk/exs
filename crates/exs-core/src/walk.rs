//! Walking a value that crosses the runner boundary, for code outside the module.
//!
//! The runner reads results and host-call arguments of runtime v2 modules from their memory:
//! the walk reports every part of a value to a [`Visitor`] in the order of the runner codec,
//! so the visitor builds the runner's value directly. Typed Objects are maps of their present
//! declared fields in entry order followed by their extra keys, enums carry their identity and
//! variant names, and Errors their kind, message, data, origin, trace and cause. Closures,
//! Cells and nesting beyond [`MAX_DEPTH`] cannot cross the boundary.

use exs_value::{Immediate, ValueRef};

use crate::bytes::{bytes_len, bytes_object};
use crate::enums::{enum_object, field_address as enum_field, names, payload_count};
use crate::errors::{error_object, error_parts};
use crate::lists::{item_address, len, list_object};
use crate::memory::Memory;
use crate::numbers::{Number, number, reference};
use crate::objects::{shape, untyped_object, value_address};
use crate::shapes::{key, key_count};
use crate::strings::{DATA, byte_len, string_object};
use crate::typed::{
    extra, ordered_slot, slot_address, slot_count, type_entry, type_id, typed_object,
};

/// Deepest nesting of Lists, Objects, enums and Errors the walk follows; deeper values,
/// including cyclic ones, cannot cross the boundary.
pub const MAX_DEPTH: u32 = 128;

/// Why a walk stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WalkError<E> {
    /// The value cannot cross the boundary, such as a closure or a cyclic graph.
    Unsupported,
    /// The visitor stopped the walk.
    Stopped(E),
}

/// Receives the parts of a walked value in order.
///
/// A List of `count` items, an Object of `count` entries (each a key and a value), an enum
/// (its identity, variant name and `count` payload fields) and an Error are followed by their
/// parts; the visitor counts them.
pub trait Visitor<M: Memory> {
    /// Why the visitor stops a walk.
    type Error;

    /// Receives None.
    ///
    /// # Errors
    ///
    /// Returns an error to stop the walk.
    fn none(&mut self) -> Result<(), Self::Error>;

    /// Receives a Bool.
    ///
    /// # Errors
    ///
    /// Returns an error to stop the walk.
    fn boolean(&mut self, value: bool) -> Result<(), Self::Error>;

    /// Receives an Int.
    ///
    /// # Errors
    ///
    /// Returns an error to stop the walk.
    fn int(&mut self, value: i64) -> Result<(), Self::Error>;

    /// Receives a Float.
    ///
    /// # Errors
    ///
    /// Returns an error to stop the walk.
    fn float(&mut self, value: f64) -> Result<(), Self::Error>;

    /// Receives a String, or an Object key, of the `len` UTF-8 bytes at `address`.
    ///
    /// # Errors
    ///
    /// Returns an error to stop the walk.
    fn string(&mut self, memory: &M, address: u32, len: u32) -> Result<(), Self::Error>;

    /// Receives Bytes of the `len` octets at `address`.
    ///
    /// # Errors
    ///
    /// Returns an error to stop the walk.
    fn bytes(&mut self, memory: &M, address: u32, len: u32) -> Result<(), Self::Error>;

    /// Starts a List of `count` items.
    ///
    /// # Errors
    ///
    /// Returns an error to stop the walk.
    fn list(&mut self, count: u32) -> Result<(), Self::Error>;

    /// Starts an Object of `count` entries.
    ///
    /// # Errors
    ///
    /// Returns an error to stop the walk.
    fn object(&mut self, count: u32) -> Result<(), Self::Error>;

    /// Starts an enum of `count` payload fields; its identity and variant name follow as
    /// Strings, then the fields.
    ///
    /// # Errors
    ///
    /// Returns an error to stop the walk.
    fn enumeration(&mut self, count: u32) -> Result<(), Self::Error>;

    /// Starts an Error with its severity, origin (0 for none) and the `count` trace entries at
    /// `trace`; its kind, message, data and cause follow, the cause as [`Visitor::no_cause`]
    /// when it has none.
    ///
    /// # Errors
    ///
    /// Returns an error to stop the walk.
    fn error(
        &mut self,
        memory: &M,
        fatal: bool,
        origin: u32,
        trace: u32,
        count: u32,
    ) -> Result<(), Self::Error>;

    /// Receives the missing cause of an Error.
    ///
    /// # Errors
    ///
    /// Returns an error to stop the walk.
    fn no_cause(&mut self) -> Result<(), Self::Error>;
}

/// Walks `raw`, a value in `memory` of the VM at `vm`, reporting its parts to `visitor`.
///
/// Every read goes through `memory`, nesting is bounded and the visitor can stop the walk, so
/// even a corrupted heap cannot make it unbounded.
///
/// # Errors
///
/// Returns `Unsupported` for a value that cannot cross the boundary, or the visitor's error.
pub fn walk_value<M: Memory, V: Visitor<M>>(
    memory: &M,
    vm: u32,
    raw: u64,
    visitor: &mut V,
) -> Result<(), WalkError<V::Error>> {
    Walk {
        memory,
        vm,
        visitor,
    }
    .value(raw, 0)
}

/// One walk in progress.
struct Walk<'w, M, V> {
    /// The module's memory.
    memory: &'w M,
    /// Address of the VM state block.
    vm: u32,
    /// The visitor receiving the parts.
    visitor: &'w mut V,
}

impl<M: Memory, V: Visitor<M>> Walk<'_, M, V> {
    /// Reports the String at `object`.
    fn string(&mut self, object: u32) -> Result<(), WalkError<V::Error>> {
        let len = byte_len(self.memory, object);
        self.visitor
            .string(self.memory, object + DATA, len)
            .map_err(WalkError::Stopped)
    }

    /// Reports the value `raw` at nesting depth `depth`.
    fn value(&mut self, raw: u64, depth: u32) -> Result<(), WalkError<V::Error>> {
        let memory = self.memory;
        if let Some(object) = string_object(raw) {
            return self.string(object);
        }
        if let Some(object) = bytes_object(raw) {
            let len = bytes_len(memory, object);
            return self
                .visitor
                .bytes(memory, object + DATA, len)
                .map_err(WalkError::Stopped);
        }
        if let Some(list) = list_object(raw) {
            return self.list(list, depth);
        }
        if let Some(object) = typed_object(memory, raw) {
            return self.typed_object(object, depth);
        }
        if let Some(object) = error_object(raw) {
            return self.error(object, depth);
        }
        if let Some(object) = enum_object(memory, raw) {
            return self.enumeration(object, depth);
        }
        if let Some(object) = untyped_object(memory, raw) {
            if depth >= MAX_DEPTH {
                return Err(WalkError::Unsupported);
            }
            let count = key_count(memory, shape(memory, object));
            self.visitor.object(count).map_err(WalkError::Stopped)?;
            return self.entries(object, depth);
        }
        let reported = match reference(raw).and_then(ValueRef::immediate) {
            Some(Immediate::None) => self.visitor.none(),
            Some(Immediate::Bool(flag)) => self.visitor.boolean(flag),
            _ => match number(memory, raw).ok_or(WalkError::Unsupported)? {
                Number::Int(value) => self.visitor.int(value),
                Number::Float(value) => self.visitor.float(value),
            },
        };
        reported.map_err(WalkError::Stopped)
    }

    /// Reports the List at `list`.
    fn list(&mut self, list: u32, depth: u32) -> Result<(), WalkError<V::Error>> {
        if depth >= MAX_DEPTH {
            return Err(WalkError::Unsupported);
        }
        let count = len(self.memory, list);
        self.visitor.list(count).map_err(WalkError::Stopped)?;
        for index in 0..count {
            let item = self.memory.load_u64(item_address(self.memory, list, index));
            self.value(item, depth + 1)?;
        }
        Ok(())
    }

    /// Reports the typed Object at `object` as its present declared fields in entry order
    /// followed by its extra keys.
    fn typed_object(&mut self, object: u32, depth: u32) -> Result<(), WalkError<V::Error>> {
        let memory = self.memory;
        if depth >= MAX_DEPTH {
            return Err(WalkError::Unsupported);
        }
        let entry = type_entry(memory, self.vm, type_id(memory, object))
            .filter(|entry| entry.field_count <= slot_count(memory, object))
            .ok_or(WalkError::Unsupported)?;
        let extra = extra(memory, object);
        let extra_count = if extra == 0 {
            0
        } else {
            key_count(memory, shape(memory, extra))
        };
        // Declared fields removed by `delete` or `clear` hold an empty slot and are skipped.
        let present = (0..entry.field_count)
            .filter(|index| memory.load_u64(slot_address(object, *index)) != 0)
            .count() as u32;
        let count = present
            .checked_add(extra_count)
            .ok_or(WalkError::Unsupported)?;
        self.visitor.object(count).map_err(WalkError::Stopped)?;
        for position in 0..entry.field_count {
            let index = ordered_slot(memory, object, position);
            let value = memory.load_u64(slot_address(object, index));
            if value == 0 {
                continue;
            }
            let name = memory.load_u64(entry.field_names + index * 8);
            self.string(string_object(name).ok_or(WalkError::Unsupported)?)?;
            self.value(value, depth + 1)?;
        }
        if extra != 0 {
            self.entries(extra, depth)?;
        }
        Ok(())
    }

    /// Reports the enum at `object`.
    fn enumeration(&mut self, object: u32, depth: u32) -> Result<(), WalkError<V::Error>> {
        let memory = self.memory;
        if depth >= MAX_DEPTH {
            return Err(WalkError::Unsupported);
        }
        let (identity, name) = names(memory, self.vm, object).ok_or(WalkError::Unsupported)?;
        let count = payload_count(memory, object);
        self.visitor
            .enumeration(count)
            .map_err(WalkError::Stopped)?;
        self.string(identity)?;
        self.string(name)?;
        for index in 0..count {
            let value = memory.load_u64(enum_field(object, index));
            self.value(value, depth + 1)?;
        }
        Ok(())
    }

    /// Reports the Error at `object`.
    fn error(&mut self, object: u32, depth: u32) -> Result<(), WalkError<V::Error>> {
        let memory = self.memory;
        if depth >= MAX_DEPTH {
            return Err(WalkError::Unsupported);
        }
        let parts = error_parts(memory, object);
        let (trace, count) = parts.trace;
        self.visitor
            .error(memory, parts.fatal, parts.origin, trace, count)
            .map_err(WalkError::Stopped)?;
        for text in [parts.kind, parts.message] {
            self.string(string_object(text).ok_or(WalkError::Unsupported)?)?;
        }
        self.value(parts.data, depth + 1)?;
        if parts.cause == ValueRef::NONE.raw() {
            self.visitor.no_cause().map_err(WalkError::Stopped)
        } else {
            self.value(parts.cause, depth + 1)
        }
    }

    /// Reports the keys and values of the untyped Object at `object` in key order.
    fn entries(&mut self, object: u32, depth: u32) -> Result<(), WalkError<V::Error>> {
        let memory = self.memory;
        let shape = shape(memory, object);
        for index in 0..key_count(memory, shape) {
            let name = string_object(key(memory, shape, index)).ok_or(WalkError::Unsupported)?;
            self.string(name)?;
            let value = memory.load_u64(value_address(memory, object, index));
            self.value(value, depth + 1)?;
        }
        Ok(())
    }
}
