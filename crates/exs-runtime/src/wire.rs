//! Decoding of compiler-emitted host wire schemas.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use exs_abi::{
    TYPE_BOOL, TYPE_BYTES, TYPE_ERROR, TYPE_FLOAT, TYPE_FN, TYPE_INT, TYPE_LIST, TYPE_NONE,
    TYPE_OBJECT, TYPE_STRING,
};
use exs_value::ValueRef;

use crate::gc;
use crate::runtime;
use crate::value::{RtValue, RuntimeEnum, RuntimeList, RuntimeObject};

/// Decodes one host boundary value according to an opaque compiler-emitted schema string.
pub(crate) fn decode(value: ValueRef, schema: ValueRef) -> ValueRef {
    let RtValue::String(schema) = runtime::value(schema) else {
        return runtime::recoverable_error(
            "WireDecodeError",
            "wire schema must be a String",
            schema,
        );
    };
    let mut parser = SchemaParser::new(schema.as_str());
    let contract = match parser.contract().and_then(|contract| {
        if parser.is_done() {
            Ok(contract)
        } else {
            Err("wire schema has trailing data")
        }
    }) {
        Ok(contract) => contract,
        Err(message) => return runtime::recoverable_error("WireDecodeError", message, value),
    };
    decode_contract(value, &contract)
}

/// One parsed recursive type contract.
struct Contract {
    /// Accepted scalar runtime categories.
    builtin_mask: u32,
    /// Optional element contract for lists.
    list: Option<Box<Contract>>,
    /// Nominal Object alternatives.
    objects: Vec<ObjectSchema>,
    /// Nominal enum alternatives.
    enums: Vec<EnumSchema>,
}

/// One nominal Object schema.
struct ObjectSchema {
    /// Compiler-owned nominal Object tag.
    type_id: u32,
    /// Fields copied from the host Object.
    fields: Vec<FieldSchema>,
}

/// One named nominal enum schema.
struct EnumSchema {
    /// Compiler-owned nominal enum tag.
    type_id: u32,
    /// Stable host-boundary enum identity.
    identity: String,
    /// Available variants.
    variants: Vec<VariantSchema>,
}

/// One enum variant schema.
struct VariantSchema {
    /// Source-visible variant name.
    name: String,
    /// Ordered payload fields.
    fields: Vec<FieldSchema>,
}

/// One object or enum payload field schema.
struct FieldSchema {
    /// Source-visible field name.
    name: String,
    /// Recursive field contract.
    contract: Contract,
}

/// Parses the compact, length-prefixed compiler schema language.
struct SchemaParser<'a> {
    /// Remaining schema bytes.
    bytes: &'a [u8],
    /// Current byte offset.
    offset: usize,
}

impl<'a> SchemaParser<'a> {
    /// Starts parsing one compiler-emitted schema string.
    fn new(schema: &'a str) -> Self {
        Self {
            bytes: schema.as_bytes(),
            offset: 0,
        }
    }

    /// Returns whether every input byte was consumed.
    fn is_done(&self) -> bool {
        self.offset == self.bytes.len()
    }

    /// Parses one recursive contract.
    fn contract(&mut self) -> Result<Contract, &'static str> {
        self.token(b'C')?;
        let builtin_mask = self.number()?;
        let list = match self.byte()? {
            b'N' => None,
            b'L' => Some(Box::new(self.contract()?)),
            _ => return Err("wire schema has an invalid list contract"),
        };
        let object_count = self.number()?;
        let mut objects = Vec::with_capacity(object_count as usize);
        for _ in 0..object_count {
            objects.push(self.object()?);
        }
        let enum_count = self.number()?;
        let mut enums = Vec::with_capacity(enum_count as usize);
        for _ in 0..enum_count {
            enums.push(self.enumeration()?);
        }
        Ok(Contract {
            builtin_mask,
            list,
            objects,
            enums,
        })
    }

    /// Parses one nominal Object schema.
    fn object(&mut self) -> Result<ObjectSchema, &'static str> {
        self.token(b'O')?;
        let type_id = self.number()?;
        let fields = self.fields()?;
        Ok(ObjectSchema { type_id, fields })
    }

    /// Parses one nominal enum schema.
    fn enumeration(&mut self) -> Result<EnumSchema, &'static str> {
        self.token(b'E')?;
        let type_id = self.number()?;
        let identity = self.text()?;
        let count = self.number()?;
        let mut variants = Vec::with_capacity(count as usize);
        for _ in 0..count {
            self.token(b'V')?;
            variants.push(VariantSchema {
                name: self.text()?,
                fields: self.fields()?,
            });
        }
        Ok(EnumSchema {
            type_id,
            identity,
            variants,
        })
    }

    /// Parses an ordered field list.
    fn fields(&mut self) -> Result<Vec<FieldSchema>, &'static str> {
        let count = self.number()?;
        let mut fields = Vec::with_capacity(count as usize);
        for _ in 0..count {
            self.token(b'F')?;
            fields.push(FieldSchema {
                name: self.text()?,
                contract: self.contract()?,
            });
        }
        Ok(fields)
    }

    /// Consumes one exact grammar token.
    fn token(&mut self, expected: u8) -> Result<(), &'static str> {
        if self.byte()? == expected {
            Ok(())
        } else {
            Err("wire schema has an unexpected token")
        }
    }

    /// Parses one semicolon-terminated unsigned number.
    fn number(&mut self) -> Result<u32, &'static str> {
        let start = self.offset;
        while self.offset < self.bytes.len() && self.bytes[self.offset].is_ascii_digit() {
            self.offset += 1;
        }
        if start == self.offset || self.byte()? != b';' {
            return Err("wire schema has an invalid number");
        }
        core::str::from_utf8(&self.bytes[start..self.offset - 1])
            .ok()
            .and_then(|value| value.parse().ok())
            .ok_or("wire schema number is out of range")
    }

    /// Parses one byte-length-prefixed UTF-8 text value.
    fn text(&mut self) -> Result<String, &'static str> {
        let length = self.number()? as usize;
        let end = self
            .offset
            .checked_add(length)
            .ok_or("wire schema text length is invalid")?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or("wire schema text is truncated")?;
        self.offset = end;
        core::str::from_utf8(bytes)
            .map(String::from)
            .map_err(|_| "wire schema text is not UTF-8")
    }

    /// Consumes one source byte.
    fn byte(&mut self) -> Result<u8, &'static str> {
        let byte = self
            .bytes
            .get(self.offset)
            .copied()
            .ok_or("wire schema is truncated")?;
        self.offset += 1;
        Ok(byte)
    }
}

/// Recursively decodes one value according to one parsed contract.
fn decode_contract(value: ValueRef, contract: &Contract) -> ValueRef {
    if matches!(runtime::value(value), RtValue::Error(_)) {
        return value;
    }
    if matches!(runtime::value(value), RtValue::List(_))
        && let Some(item) = &contract.list
    {
        return decode_list(value, item);
    }
    if matches!(runtime::value(value), RtValue::Object(_)) {
        if let Some(decoded) = decode_enum(value, &contract.enums) {
            return decoded;
        }
        if let Some(decoded) = decode_object(value, &contract.objects) {
            return decoded;
        }
    }
    if value_type_mask(runtime::value(value)) & contract.builtin_mask != 0 {
        value
    } else {
        runtime::recoverable_error(
            "WireDecodeError",
            "host value does not satisfy the declared ExS type",
            value,
        )
    }
}

/// Decodes every list element and returns a fresh ExS List.
fn decode_list(value: ValueRef, item_contract: &Contract) -> ValueRef {
    let RtValue::List(source) = runtime::value(value) else {
        return runtime::recoverable_error("WireDecodeError", "expected a List from host", value);
    };
    let source = source.elements.clone();
    let checkpoint = gc::temporary_root_checkpoint();
    let mut elements = Vec::with_capacity(source.len());
    for item in source {
        let decoded = decode_contract(item, item_contract);
        if matches!(runtime::value(decoded), RtValue::Error(_)) {
            gc::restore_temporary_roots(checkpoint);
            return decoded;
        }
        gc::push_temporary_root(decoded);
        elements.push(decoded);
    }
    let result = runtime::allocate(RtValue::List(Box::new(RuntimeList { elements })));
    gc::restore_temporary_roots(checkpoint);
    result
}

/// Selects and decodes one host Object as an enum, when it carries a known `$variant` tag.
fn decode_enum(value: ValueRef, schemas: &[EnumSchema]) -> Option<ValueRef> {
    let variant = object_field(value, "$variant")?;
    let RtValue::String(variant) = runtime::value(variant) else {
        return Some(runtime::recoverable_error(
            "WireDecodeError",
            "enum `$variant` must be a String",
            value,
        ));
    };
    let schema = schemas.iter().find_map(|schema| {
        schema
            .variants
            .iter()
            .find(|candidate| candidate.name == variant.as_str())
            .map(|candidate| (schema, candidate))
    })?;
    if matches!(runtime::value(value), RtValue::Object(object) if object.type_id == Some(schema.0.type_id) && object.enum_data.is_some())
    {
        return Some(value);
    }
    Some(decode_enum_variant(value, schema.0, schema.1))
}

/// Decodes a known host enum variant into the private runtime enum representation.
fn decode_enum_variant(value: ValueRef, schema: &EnumSchema, variant: &VariantSchema) -> ValueRef {
    let checkpoint = gc::temporary_root_checkpoint();
    let mut fields = Vec::with_capacity(variant.fields.len());
    for (index, field) in variant.fields.iter().enumerate() {
        let raw = object_field(value, &field.name)
            .or_else(|| enum_tuple_field(value, index))
            .unwrap_or_else(|| runtime::allocate(RtValue::None));
        let decoded = decode_contract(raw, &field.contract);
        if matches!(runtime::value(decoded), RtValue::Error(_)) {
            gc::restore_temporary_roots(checkpoint);
            return decoded;
        }
        gc::push_temporary_root(decoded);
        fields.push(decoded);
    }
    let result = runtime::allocate(RtValue::Object(Box::new(RuntimeObject::enumeration(
        Some(schema.type_id),
        RuntimeEnum {
            type_identity: schema.identity.clone().into_boxed_str(),
            variant: variant.name.clone().into_boxed_str(),
            fields,
        },
    ))));
    gc::restore_temporary_roots(checkpoint);
    result
}

/// Selects and decodes one host Object as the first compatible nominal Object schema.
fn decode_object(value: ValueRef, schemas: &[ObjectSchema]) -> Option<ValueRef> {
    let schema = schemas.first()?;
    if matches!(runtime::value(value), RtValue::Object(object) if object.type_id == Some(schema.type_id) && object.enum_data.is_none())
    {
        return Some(value);
    }
    let checkpoint = gc::temporary_root_checkpoint();
    let mut entries = Vec::with_capacity(schema.fields.len());
    for field in &schema.fields {
        let raw =
            object_field(value, &field.name).unwrap_or_else(|| runtime::allocate(RtValue::None));
        let decoded = decode_contract(raw, &field.contract);
        if matches!(runtime::value(decoded), RtValue::Error(_)) {
            gc::restore_temporary_roots(checkpoint);
            return Some(decoded);
        }
        gc::push_temporary_root(decoded);
        entries.push((field.name.clone().into_boxed_str(), decoded));
    }
    let result = runtime::allocate(RtValue::Object(Box::new(RuntimeObject {
        type_id: Some(schema.type_id),
        entries,
        enum_data: None,
    })));
    gc::restore_temporary_roots(checkpoint);
    Some(result)
}

/// Returns a direct object property without allocating an absent `None` value.
fn object_field(value: ValueRef, name: &str) -> Option<ValueRef> {
    match runtime::value(value) {
        RtValue::Object(object) => object
            .entries
            .iter()
            .find_map(|(key, value)| (key.as_ref() == name).then_some(*value)),
        _ => None,
    }
}

/// Returns one positional serde tuple-enum payload field when present.
fn enum_tuple_field(value: ValueRef, index: usize) -> Option<ValueRef> {
    let values = object_field(value, "$values")?;
    match runtime::value(values) {
        RtValue::List(values) => values.elements.get(index).copied(),
        _ => None,
    }
}

/// Returns the ABI category mask for one runtime value.
fn value_type_mask(value: &RtValue) -> u32 {
    match value {
        RtValue::None => TYPE_NONE,
        RtValue::Error(_) => TYPE_ERROR,
        RtValue::Bool(_) => TYPE_BOOL,
        RtValue::Int(_) => TYPE_INT,
        RtValue::Float(_) => TYPE_FLOAT,
        RtValue::String(_) => TYPE_STRING,
        RtValue::Bytes(_) => TYPE_BYTES,
        RtValue::List(_) => TYPE_LIST,
        RtValue::Object(_) => TYPE_OBJECT,
        RtValue::Closure(_) => TYPE_FN,
        RtValue::Cell(_) | RtValue::BoxedFutureValue(_) => 0,
    }
}
