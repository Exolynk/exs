//! Deep cloning for runtime-owned ExS value graphs.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use exs_value::ValueRef;

use crate::gc;
use crate::runtime;
use crate::value::{
    RtValue, RuntimeCellValue, RuntimeClosure, RuntimeEnum, RuntimeError, RuntimeList,
    RuntimeObject,
};

/// Deeply clones one source-visible value graph.
///
/// Immutable values are reused. Every allocated clone stays temporarily rooted until the full
/// traversal succeeds or fails, so allocation-triggered collection cannot invalidate a shell.
pub(crate) fn deep_clone(source: ValueRef) -> ValueRef {
    let checkpoint = gc::temporary_root_checkpoint();
    gc::push_temporary_root(source);
    let mut context = CloneContext::new();
    let result = context.clone_value(source);
    let result = match result {
        Ok(value) => value,
        Err(()) => {
            // Do not expose the source graph as Error data: it contains the unsupported value
            // and therefore might not be representable across the CBOR host boundary.
            let data = runtime::allocate(RtValue::None);
            runtime::recoverable_error(
                "CloneError",
                "clone does not support a reachable host-owned runtime resource",
                data,
            )
        }
    };
    gc::restore_temporary_roots(checkpoint);
    result
}

/// Per-operation source identity map used to preserve cycles and aliases.
struct CloneContext {
    /// Every allocated source-to-clone identity pair discovered so far.
    clones: Vec<(ValueRef, ValueRef)>,
}

impl CloneContext {
    /// Creates an empty clone traversal context.
    fn new() -> Self {
        Self { clones: Vec::new() }
    }

    /// Clones one value and all mutable values reachable from it.
    fn clone_value(&mut self, source: ValueRef) -> Result<ValueRef, ()> {
        match runtime::value(source) {
            RtValue::None
            | RtValue::Bool(_)
            | RtValue::Int(_)
            | RtValue::Float(_)
            | RtValue::String(_) => Ok(source),
            RtValue::List(list) => self.clone_list(source, list.elements.clone()),
            RtValue::Object(object) => self.clone_object(source, object),
            RtValue::Cell(cell) => self.clone_cell(source, cell.value),
            RtValue::Closure(closure) => self.clone_closure(
                source,
                closure.function_id,
                closure.slot_count,
                closure.arity,
                closure.captures.clone(),
            ),
            RtValue::Error(error) => self.clone_error(source, error),
            // This reserved boxed shape is the runtime placeholder for future host-owned values.
            RtValue::BoxedFutureValue(_) => Err(()),
        }
    }

    /// Reuses the cloned identity already assigned to one source allocation.
    fn existing_clone(&self, source: ValueRef) -> Option<ValueRef> {
        self.clones
            .iter()
            .find_map(|(original, cloned)| (*original == source).then_some(*cloned))
    }

    /// Records and roots one newly allocated clone shell before cloning its children.
    fn register_shell(&mut self, source: ValueRef, clone: ValueRef) {
        self.clones.push((source, clone));
        gc::push_temporary_root(clone);
    }

    /// Clones one mutable List and preserves references to its clone shell.
    fn clone_list(&mut self, source: ValueRef, elements: Vec<ValueRef>) -> Result<ValueRef, ()> {
        if let Some(clone) = self.existing_clone(source) {
            return Ok(clone);
        }
        let clone = runtime::allocate(RtValue::List(Box::new(RuntimeList::new())));
        self.register_shell(source, clone);
        let elements = self.clone_values(elements)?;
        let RtValue::List(list) = runtime::value_mut(clone) else {
            runtime::trap();
        };
        list.elements = elements;
        Ok(clone)
    }

    /// Clones one nominal, enum, or plain mutable Object.
    fn clone_object(&mut self, source: ValueRef, object: &RuntimeObject) -> Result<ValueRef, ()> {
        if let Some(clone) = self.existing_clone(source) {
            return Ok(clone);
        }
        let type_id = object.type_id;
        let entries = object
            .entries
            .iter()
            .map(|(key, value)| (String::from(key.as_ref()), *value))
            .collect::<Vec<_>>();
        let enum_data = object.enum_data.as_ref().map(|enum_data| {
            (
                String::from(enum_data.type_identity.as_ref()),
                String::from(enum_data.variant.as_ref()),
                enum_data.fields.clone(),
            )
        });
        let clone = runtime::allocate(RtValue::Object(Box::new(RuntimeObject {
            type_id,
            entries: Vec::new(),
            enum_data: None,
        })));
        self.register_shell(source, clone);
        let entries = self.clone_entries(entries)?;
        let enum_data = match enum_data {
            Some((type_identity, variant, fields)) => Some(RuntimeEnum {
                type_identity: type_identity.into_boxed_str(),
                variant: variant.into_boxed_str(),
                fields: self.clone_values(fields)?,
            }),
            None => None,
        };
        let RtValue::Object(object) = runtime::value_mut(clone) else {
            runtime::trap();
        };
        object.entries = entries;
        object.enum_data = enum_data;
        Ok(clone)
    }

    /// Clones one mutable lexical Cell.
    fn clone_cell(&mut self, source: ValueRef, value: ValueRef) -> Result<ValueRef, ()> {
        if let Some(clone) = self.existing_clone(source) {
            return Ok(clone);
        }
        let clone = runtime::allocate(RtValue::Cell(Box::new(RuntimeCellValue::new(value))));
        self.register_shell(source, clone);
        let value = self.clone_value(value)?;
        let RtValue::Cell(cell) = runtime::value_mut(clone) else {
            runtime::trap();
        };
        cell.value = value;
        Ok(clone)
    }

    /// Clones one closure and its captured Cell graph.
    fn clone_closure(
        &mut self,
        source: ValueRef,
        function_id: u32,
        slot_count: u32,
        arity: u32,
        captures: Vec<ValueRef>,
    ) -> Result<ValueRef, ()> {
        if let Some(clone) = self.existing_clone(source) {
            return Ok(clone);
        }
        let clone = runtime::allocate(RtValue::Closure(Box::new(RuntimeClosure::new(
            function_id,
            slot_count,
            arity,
            Vec::new(),
        ))));
        self.register_shell(source, clone);
        let captures = self.clone_values(captures)?;
        let RtValue::Closure(closure) = runtime::value_mut(clone) else {
            runtime::trap();
        };
        closure.captures = captures;
        Ok(clone)
    }

    /// Clones one structured Error including its data and optional cause graph.
    fn clone_error(&mut self, source: ValueRef, error: &RuntimeError) -> Result<ValueRef, ()> {
        if let Some(clone) = self.existing_clone(source) {
            return Ok(clone);
        }
        let severity = error.severity;
        let kind = String::from(error.kind.as_ref()).into_boxed_str();
        let message = String::from(error.message.as_ref()).into_boxed_str();
        let data = error.data;
        let origin = error.origin;
        let trace = error.trace.clone();
        let cause = error.cause;
        let clone = runtime::allocate(RtValue::Error(Box::new(RuntimeError {
            severity,
            kind,
            message,
            data,
            origin,
            trace,
            cause,
        })));
        self.register_shell(source, clone);
        let data = self.clone_value(data)?;
        let cause = match cause {
            Some(cause) => Some(self.clone_value(cause)?),
            None => None,
        };
        let RtValue::Error(error) = runtime::value_mut(clone) else {
            runtime::trap();
        };
        error.data = data;
        error.cause = cause;
        Ok(clone)
    }

    /// Clones a source-order sequence of value references.
    fn clone_values(&mut self, values: Vec<ValueRef>) -> Result<Vec<ValueRef>, ()> {
        values
            .into_iter()
            .map(|value| self.clone_value(value))
            .collect()
    }

    /// Clones Object values while retaining their owned, immutable key strings.
    fn clone_entries(
        &mut self,
        entries: Vec<(String, ValueRef)>,
    ) -> Result<Vec<(Box<str>, ValueRef)>, ()> {
        entries
            .into_iter()
            .map(|(key, value)| Ok((key.into_boxed_str(), self.clone_value(value)?)))
            .collect()
    }
}
