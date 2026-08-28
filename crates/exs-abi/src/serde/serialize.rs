use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use serde::Serialize;
use serde::ser::{
    Impossible, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant,
    SerializeTuple, SerializeTupleStruct, SerializeTupleVariant, Serializer,
};

use super::{ExsValue, SerdeError};

/// Serializes one Rust value into an owned ExS boundary value.
pub(super) fn serialize<T: Serialize>(value: &T) -> Result<ExsValue, SerdeError> {
    value.serialize(ValueSerializer)
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
