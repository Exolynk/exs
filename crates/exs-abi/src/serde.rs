//! Serde adapters for the host-safe ExS value tree.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use serde::Serialize;
use serde::de::{
    DeserializeOwned, Deserializer, EnumAccess, IntoDeserializer, MapAccess, SeqAccess,
    VariantAccess, Visitor,
};
use serde::ser::{
    Impossible, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant,
    SerializeTuple, SerializeTupleStruct, SerializeTupleVariant, Serializer,
};

use crate::{Bytes, ExsValue};

impl Serialize for Bytes {
    fn serialize<Serializer>(
        &self,
        serializer: Serializer,
    ) -> Result<Serializer::Ok, Serializer::Error>
    where
        Serializer: serde::Serializer,
    {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for Bytes {
    fn deserialize<Deserializer>(deserializer: Deserializer) -> Result<Self, Deserializer::Error>
    where
        Deserializer: serde::Deserializer<'de>,
    {
        struct BytesVisitor;

        impl<'de> Visitor<'de> for BytesVisitor {
            type Value = Bytes;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an ExS Bytes value")
            }

            fn visit_byte_buf<E: serde::de::Error>(self, value: Vec<u8>) -> Result<Self::Value, E> {
                Ok(Bytes::new(value))
            }

            fn visit_bytes<E: serde::de::Error>(self, value: &[u8]) -> Result<Self::Value, E> {
                Ok(Bytes::new(value.into()))
            }
        }

        deserializer.deserialize_byte_buf(BytesVisitor)
    }
}

/// A Serde conversion error raised while translating a Rust value and an ExS boundary value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SerdeError {
    message: String,
}

impl SerdeError {
    /// Creates one conversion error with the supplied explanation.
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SerdeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl core::error::Error for SerdeError {}

impl serde::ser::Error for SerdeError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Self::new(message.to_string())
    }
}

impl serde::de::Error for SerdeError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Self::new(message.to_string())
    }
}

impl ExsValue {
    /// Serializes one Rust value into the host-safe ExS value tree.
    ///
    /// Rust structs become ExS Objects, vectors become Lists, byte buffers become Bytes, and
    /// Rust enums become tagged Objects accepted by the generated ExS boundary decoder.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported map keys or a custom serializer failure.
    pub fn from_serialize<T: Serialize>(value: &T) -> Result<Self, SerdeError> {
        value.serialize(ValueSerializer)
    }

    /// Deserializes this host-safe ExS value into one owned Rust value.
    ///
    /// # Errors
    ///
    /// Returns an error when the ExS value does not satisfy the Rust target type.
    pub fn into_deserialize<T: DeserializeOwned>(self) -> Result<T, SerdeError> {
        T::deserialize(ValueDeserializer(self))
    }
}

/// Serde serializer producing one owned ExS boundary value.
struct ValueSerializer;

impl Serializer for ValueSerializer {
    type Ok = ExsValue;
    type Error = SerdeError;
    type SerializeSeq = SequenceSerializer;
    type SerializeTuple = SequenceSerializer;
    type SerializeTupleStruct = SequenceSerializer;
    type SerializeTupleVariant = SequenceSerializer;
    type SerializeMap = MapSerializer;
    type SerializeStruct = StructSerializer;
    type SerializeStructVariant = StructSerializer;

    fn serialize_bool(self, value: bool) -> Result<Self::Ok, Self::Error> {
        Ok(ExsValue::Bool(value))
    }

    fn serialize_i8(self, value: i8) -> Result<Self::Ok, Self::Error> {
        Ok(ExsValue::Int(i64::from(value)))
    }

    fn serialize_i16(self, value: i16) -> Result<Self::Ok, Self::Error> {
        Ok(ExsValue::Int(i64::from(value)))
    }

    fn serialize_i32(self, value: i32) -> Result<Self::Ok, Self::Error> {
        Ok(ExsValue::Int(i64::from(value)))
    }

    fn serialize_i64(self, value: i64) -> Result<Self::Ok, Self::Error> {
        Ok(ExsValue::Int(value))
    }

    fn serialize_i128(self, value: i128) -> Result<Self::Ok, Self::Error> {
        i64::try_from(value)
            .map(ExsValue::Int)
            .map_err(|_| SerdeError::new("i128 value is outside the ExS Int range"))
    }

    fn serialize_u8(self, value: u8) -> Result<Self::Ok, Self::Error> {
        Ok(ExsValue::Int(i64::from(value)))
    }

    fn serialize_u16(self, value: u16) -> Result<Self::Ok, Self::Error> {
        Ok(ExsValue::Int(i64::from(value)))
    }

    fn serialize_u32(self, value: u32) -> Result<Self::Ok, Self::Error> {
        Ok(ExsValue::Int(i64::from(value)))
    }

    fn serialize_u64(self, value: u64) -> Result<Self::Ok, Self::Error> {
        i64::try_from(value)
            .map(ExsValue::Int)
            .map_err(|_| SerdeError::new("u64 value is outside the ExS Int range"))
    }

    fn serialize_u128(self, value: u128) -> Result<Self::Ok, Self::Error> {
        i64::try_from(value)
            .map(ExsValue::Int)
            .map_err(|_| SerdeError::new("u128 value is outside the ExS Int range"))
    }

    fn serialize_f32(self, value: f32) -> Result<Self::Ok, Self::Error> {
        Ok(ExsValue::Float(f64::from(value)))
    }

    fn serialize_f64(self, value: f64) -> Result<Self::Ok, Self::Error> {
        Ok(ExsValue::Float(value))
    }

    fn serialize_char(self, value: char) -> Result<Self::Ok, Self::Error> {
        Ok(ExsValue::String(value.to_string()))
    }

    fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
        Ok(ExsValue::String(value.into()))
    }

    fn serialize_bytes(self, value: &[u8]) -> Result<Self::Ok, Self::Error> {
        Ok(ExsValue::Bytes(value.into()))
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(ExsValue::None)
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Ok(ExsValue::None)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        self.serialize_unit()
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(enum_object(variant, Vec::new()))
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(enum_object(
            variant,
            vec![("$value".into(), value.serialize(ValueSerializer)?)],
        ))
    }

    fn serialize_seq(self, length: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(SequenceSerializer::new(length, None))
    }

    fn serialize_tuple(self, length: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(SequenceSerializer::new(Some(length), None))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        length: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(SequenceSerializer::new(Some(length), None))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        length: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(SequenceSerializer::new(Some(length), Some(variant.into())))
    }

    fn serialize_map(self, length: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(MapSerializer::new(length))
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        length: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(StructSerializer::new(length, None))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        length: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(StructSerializer::new(length, Some(variant.into())))
    }

    fn collect_str<T: ?Sized + fmt::Display>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        self.serialize_str(&value.to_string())
    }
}

/// Builds the stable tagged Object representation of one Rust enum value.
fn enum_object(variant: &str, mut entries: Vec<(String, ExsValue)>) -> ExsValue {
    entries.insert(0, ("$variant".into(), ExsValue::String(variant.into())));
    ExsValue::Object(entries)
}

/// Serde sequence serializer retaining a possible enum variant tag.
struct SequenceSerializer {
    values: Vec<ExsValue>,
    variant: Option<String>,
}

impl SequenceSerializer {
    /// Creates one sequence serializer with an optional enum variant tag.
    fn new(length: Option<usize>, variant: Option<String>) -> Self {
        Self {
            values: Vec::with_capacity(length.unwrap_or_default()),
            variant,
        }
    }

    /// Finishes the sequence as an ExS List or tagged enum Object.
    fn finish(self) -> ExsValue {
        match self.variant {
            Some(variant) => enum_object(
                &variant,
                vec![("$values".into(), ExsValue::List(self.values))],
            ),
            None => ExsValue::List(self.values),
        }
    }
}

impl SerializeSeq for SequenceSerializer {
    type Ok = ExsValue;
    type Error = SerdeError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        self.values.push(value.serialize(ValueSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(self.finish())
    }
}

impl SerializeTuple for SequenceSerializer {
    type Ok = ExsValue;
    type Error = SerdeError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(self.finish())
    }
}

impl SerializeTupleStruct for SequenceSerializer {
    type Ok = ExsValue;
    type Error = SerdeError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(self.finish())
    }
}

impl SerializeTupleVariant for SequenceSerializer {
    type Ok = ExsValue;
    type Error = SerdeError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(self.finish())
    }
}

/// Serde map serializer requiring String keys for the ExS Object representation.
struct MapSerializer {
    entries: Vec<(String, ExsValue)>,
    key: Option<String>,
}

impl MapSerializer {
    /// Creates one map serializer with an optional initial capacity.
    fn new(length: Option<usize>) -> Self {
        Self {
            entries: Vec::with_capacity(length.unwrap_or_default()),
            key: None,
        }
    }
}

impl SerializeMap for MapSerializer {
    type Ok = ExsValue;
    type Error = SerdeError;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        if self.key.is_some() {
            return Err(SerdeError::new("map key was not followed by a value"));
        }
        self.key = Some(key.serialize(StringKeySerializer)?);
        Ok(())
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        let key = self
            .key
            .take()
            .ok_or_else(|| SerdeError::new("map value was supplied without a key"))?;
        self.entries.push((key, value.serialize(ValueSerializer)?));
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        if self.key.is_some() {
            return Err(SerdeError::new(
                "map ended without a value for its final key",
            ));
        }
        Ok(ExsValue::Object(self.entries))
    }
}

/// Serde struct serializer with an optional enum variant discriminator.
struct StructSerializer {
    entries: Vec<(String, ExsValue)>,
    variant: Option<String>,
}

impl StructSerializer {
    /// Creates one struct serializer with an optional enum variant discriminator.
    fn new(length: usize, variant: Option<String>) -> Self {
        Self {
            entries: Vec::with_capacity(length.saturating_add(usize::from(variant.is_some()))),
            variant,
        }
    }

    /// Finishes this struct as an ordinary or enum-tagged ExS Object.
    fn finish(self) -> ExsValue {
        match self.variant {
            Some(variant) => enum_object(&variant, self.entries),
            None => ExsValue::Object(self.entries),
        }
    }
}

impl SerializeStruct for StructSerializer {
    type Ok = ExsValue;
    type Error = SerdeError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.entries
            .push((key.into(), value.serialize(ValueSerializer)?));
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(self.finish())
    }
}

impl SerializeStructVariant for StructSerializer {
    type Ok = ExsValue;
    type Error = SerdeError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        SerializeStruct::serialize_field(self, key, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(self.finish())
    }
}

/// Serializer accepting only String map keys.
struct StringKeySerializer;

impl Serializer for StringKeySerializer {
    type Ok = String;
    type Error = SerdeError;
    type SerializeSeq = Impossible<String, SerdeError>;
    type SerializeTuple = Impossible<String, SerdeError>;
    type SerializeTupleStruct = Impossible<String, SerdeError>;
    type SerializeTupleVariant = Impossible<String, SerdeError>;
    type SerializeMap = Impossible<String, SerdeError>;
    type SerializeStruct = Impossible<String, SerdeError>;
    type SerializeStructVariant = Impossible<String, SerdeError>;

    fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
        Ok(value.into())
    }

    fn serialize_char(self, value: char) -> Result<Self::Ok, Self::Error> {
        Ok(value.to_string())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(variant.into())
    }

    fn serialize_bool(self, _value: bool) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_i8(self, _value: i8) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_i16(self, _value: i16) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_i32(self, _value: i32) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_i64(self, _value: i64) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_i128(self, _value: i128) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_u8(self, _value: u8) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_u16(self, _value: u16) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_u32(self, _value: u32) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_u64(self, _value: u64) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_u128(self, _value: u128) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_f32(self, _value: f32) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_f64(self, _value: f64) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_bytes(self, _value: &[u8]) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_some<T: ?Sized + Serialize>(self, _value: &T) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_seq(self, _length: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_tuple(self, _length: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _length: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _length: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_map(self, _length: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_struct(
        self,
        _name: &'static str,
        _length: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _length: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(SerdeError::new("ExS Object keys must be Strings"))
    }
}

/// Owned Serde deserializer reading one ExS boundary value.
struct ValueDeserializer(ExsValue);

impl<'de> Deserializer<'de> for ValueDeserializer {
    type Error = SerdeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            ExsValue::None => visitor.visit_unit(),
            ExsValue::Bool(value) => visitor.visit_bool(value),
            ExsValue::Int(value) => visitor.visit_i64(value),
            ExsValue::Float(value) => visitor.visit_f64(value),
            ExsValue::String(value) => visitor.visit_string(value),
            ExsValue::Bytes(value) => visitor.visit_byte_buf(value),
            ExsValue::List(values) => visitor.visit_seq(ValueSeqAccess::new(values)),
            ExsValue::Object(entries) => visitor.visit_map(ValueMapAccess::new(entries)),
            ExsValue::Enum {
                variant, fields, ..
            } => visitor.visit_map(ValueMapAccess::new(enum_entries(variant, fields))),
            ExsValue::Error(error) => Err(SerdeError::new(format!(
                "cannot deserialize ExS Error `{}` as data",
                error.kind
            ))),
        }
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            ExsValue::Bool(value) => visitor.visit_bool(value),
            value => unexpected(value),
        }
    }
    fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let value = integer(self.0)?;
        visitor.visit_i8(
            i8::try_from(value).map_err(|_| SerdeError::new("integer is outside i8 range"))?,
        )
    }
    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let value = integer(self.0)?;
        visitor.visit_i16(
            i16::try_from(value).map_err(|_| SerdeError::new("integer is outside i16 range"))?,
        )
    }
    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let value = integer(self.0)?;
        visitor.visit_i32(
            i32::try_from(value).map_err(|_| SerdeError::new("integer is outside i32 range"))?,
        )
    }
    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_i64(integer(self.0)?)
    }
    fn deserialize_i128<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_i128(i128::from(integer(self.0)?))
    }
    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let value = integer(self.0)?;
        visitor.visit_u8(
            u8::try_from(value).map_err(|_| SerdeError::new("integer is outside u8 range"))?,
        )
    }
    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let value = integer(self.0)?;
        visitor.visit_u16(
            u16::try_from(value).map_err(|_| SerdeError::new("integer is outside u16 range"))?,
        )
    }
    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let value = integer(self.0)?;
        visitor.visit_u32(
            u32::try_from(value).map_err(|_| SerdeError::new("integer is outside u32 range"))?,
        )
    }
    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let value = integer(self.0)?;
        visitor.visit_u64(u64::try_from(value).map_err(|_| SerdeError::new("integer is negative"))?)
    }
    fn deserialize_u128<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_u128(
            u128::try_from(integer(self.0)?).map_err(|_| SerdeError::new("integer is negative"))?,
        )
    }
    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let value = float(self.0)?;
        visitor.visit_f32(value as f32)
    }
    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_f64(float(self.0)?)
    }
    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let ExsValue::String(value) = self.0 else {
            return unexpected(self.0);
        };
        let mut chars = value.chars();
        let character = chars
            .next()
            .ok_or_else(|| SerdeError::new("expected one character"))?;
        if chars.next().is_some() {
            return Err(SerdeError::new("expected one character"));
        }
        visitor.visit_char(character)
    }
    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let ExsValue::String(value) = self.0 else {
            return unexpected(self.0);
        };
        visitor.visit_string(value)
    }
    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.deserialize_str(visitor)
    }
    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let ExsValue::Bytes(value) = self.0 else {
            return unexpected(self.0);
        };
        visitor.visit_byte_buf(value)
    }
    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.deserialize_bytes(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            ExsValue::None => visitor.visit_none(),
            value => visitor.visit_some(ValueDeserializer(value)),
        }
    }
    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            ExsValue::None => visitor.visit_unit(),
            value => unexpected(value),
        }
    }
    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_unit(visitor)
    }
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }
    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let ExsValue::List(values) = self.0 else {
            return unexpected(self.0);
        };
        visitor.visit_seq(ValueSeqAccess::new(values))
    }
    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _length: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_seq(visitor)
    }
    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _length: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_seq(visitor)
    }
    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let ExsValue::Object(entries) = self.0 else {
            return unexpected(self.0);
        };
        visitor.visit_map(ValueMapAccess::new(entries))
    }
    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        let access = match self.0 {
            ExsValue::Enum {
                variant, fields, ..
            } => ValueEnumAccess {
                variant,
                content: EnumContent::Values(fields),
            },
            ExsValue::Object(mut entries) => {
                let index = entries
                    .iter()
                    .position(|(key, _)| key == "$variant")
                    .ok_or_else(|| SerdeError::new("enum Object is missing `$variant`"))?;
                let (_, value) = entries.remove(index);
                let ExsValue::String(variant) = value else {
                    return Err(SerdeError::new("enum `$variant` must be a String"));
                };
                ValueEnumAccess {
                    variant,
                    content: EnumContent::Entries(entries),
                }
            }
            value => return unexpected(value),
        };
        visitor.visit_enum(access)
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.deserialize_str(visitor)
    }
    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }
    fn is_human_readable(&self) -> bool {
        false
    }
}

/// Returns one integer payload or a descriptive conversion error.
fn integer(value: ExsValue) -> Result<i64, SerdeError> {
    match value {
        ExsValue::Int(value) => Ok(value),
        value => Err(SerdeError::new(format!(
            "expected Int, found {}",
            value_name(&value)
        ))),
    }
}

/// Returns one floating-point payload, accepting exact integer values too.
fn float(value: ExsValue) -> Result<f64, SerdeError> {
    match value {
        ExsValue::Float(value) => Ok(value),
        ExsValue::Int(value) => Ok(value as f64),
        value => Err(SerdeError::new(format!(
            "expected Float, found {}",
            value_name(&value)
        ))),
    }
}

/// Reports one unexpected ExS value to a typed deserialization visitor.
fn unexpected<T>(value: ExsValue) -> Result<T, SerdeError> {
    Err(SerdeError::new(format!(
        "unexpected ExS {} value",
        value_name(&value)
    )))
}

/// Returns the stable user-facing category name of one ExS value.
fn value_name(value: &ExsValue) -> &'static str {
    match value {
        ExsValue::None => "None",
        ExsValue::Error(_) => "Error",
        ExsValue::Bool(_) => "Bool",
        ExsValue::Int(_) => "Int",
        ExsValue::Float(_) => "Float",
        ExsValue::String(_) => "String",
        ExsValue::Bytes(_) => "Bytes",
        ExsValue::List(_) => "List",
        ExsValue::Object(_) => "Object",
        ExsValue::Enum { .. } => "Enum",
    }
}

/// Serde sequence access over one owned ExS List.
struct ValueSeqAccess {
    values: alloc::vec::IntoIter<ExsValue>,
}

impl ValueSeqAccess {
    fn new(values: Vec<ExsValue>) -> Self {
        Self {
            values: values.into_iter(),
        }
    }
}

impl<'de> SeqAccess<'de> for ValueSeqAccess {
    type Error = SerdeError;
    fn next_element_seed<T: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Self::Error> {
        self.values
            .next()
            .map(|value| seed.deserialize(ValueDeserializer(value)))
            .transpose()
    }
    fn size_hint(&self) -> Option<usize> {
        Some(self.values.len())
    }
}

/// Serde map access over one owned ExS Object.
struct ValueMapAccess {
    entries: alloc::vec::IntoIter<(String, ExsValue)>,
    value: Option<ExsValue>,
}

impl ValueMapAccess {
    fn new(entries: Vec<(String, ExsValue)>) -> Self {
        Self {
            entries: entries.into_iter(),
            value: None,
        }
    }
}

impl<'de> MapAccess<'de> for ValueMapAccess {
    type Error = SerdeError;
    fn next_key_seed<K: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        let Some((key, value)) = self.entries.next() else {
            return Ok(None);
        };
        self.value = Some(value);
        seed.deserialize(key.into_deserializer()).map(Some)
    }
    fn next_value_seed<V: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        let value = self
            .value
            .take()
            .ok_or_else(|| SerdeError::new("map value requested before a key"))?;
        seed.deserialize(ValueDeserializer(value))
    }
    fn size_hint(&self) -> Option<usize> {
        Some(self.entries.len())
    }
}

/// One decoded enum representation before Serde selects its variant shape.
enum EnumContent {
    Values(Vec<ExsValue>),
    Entries(Vec<(String, ExsValue)>),
}

/// Serde enum access over an ExS enum or tagged Object.
struct ValueEnumAccess {
    variant: String,
    content: EnumContent,
}

impl<'de> EnumAccess<'de> for ValueEnumAccess {
    type Error = SerdeError;
    type Variant = ValueVariantAccess;
    fn variant_seed<V: serde::de::DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), Self::Error> {
        let variant = seed.deserialize(self.variant.into_deserializer())?;
        Ok((
            variant,
            ValueVariantAccess {
                content: self.content,
            },
        ))
    }
}

/// Serde variant access over an ExS enum payload.
struct ValueVariantAccess {
    content: EnumContent,
}

impl<'de> VariantAccess<'de> for ValueVariantAccess {
    type Error = SerdeError;
    fn unit_variant(self) -> Result<(), Self::Error> {
        match self.content {
            EnumContent::Values(values) if values.is_empty() => Ok(()),
            EnumContent::Entries(entries) if entries.is_empty() => Ok(()),
            _ => Err(SerdeError::new("unit enum variant has a payload")),
        }
    }
    fn newtype_variant_seed<T: serde::de::DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, Self::Error> {
        match self.content {
            EnumContent::Values(mut values) if values.len() == 1 => {
                seed.deserialize(ValueDeserializer(values.remove(0)))
            }
            EnumContent::Entries(mut entries) => {
                let index = entries
                    .iter()
                    .position(|(key, _)| key == "$value")
                    .ok_or_else(|| SerdeError::new("newtype enum variant is missing `$value`"))?;
                seed.deserialize(ValueDeserializer(entries.remove(index).1))
            }
            _ => Err(SerdeError::new(
                "newtype enum variant has an invalid payload",
            )),
        }
    }
    fn tuple_variant<V: Visitor<'de>>(
        self,
        _length: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        match self.content {
            EnumContent::Values(values) => visitor.visit_seq(ValueSeqAccess::new(values)),
            EnumContent::Entries(mut entries) => {
                let index = entries
                    .iter()
                    .position(|(key, _)| key == "$values")
                    .ok_or_else(|| SerdeError::new("tuple enum variant is missing `$values`"))?;
                let ExsValue::List(values) = entries.remove(index).1 else {
                    return Err(SerdeError::new("enum `$values` must be a List"));
                };
                visitor.visit_seq(ValueSeqAccess::new(values))
            }
        }
    }
    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        match self.content {
            EnumContent::Entries(entries) => visitor.visit_map(ValueMapAccess::new(entries)),
            EnumContent::Values(values) => {
                let entries = values
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| (index.to_string(), value))
                    .collect();
                visitor.visit_map(ValueMapAccess::new(entries))
            }
        }
    }
}

/// Converts the existing ExS enum representation into its tagged Object form.
fn enum_entries(variant: String, fields: Vec<ExsValue>) -> Vec<(String, ExsValue)> {
    vec![
        ("$variant".into(), ExsValue::String(variant)),
        ("$values".into(), ExsValue::List(fields)),
    ]
}
