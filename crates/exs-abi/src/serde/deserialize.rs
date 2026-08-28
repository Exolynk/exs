use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde::de::{
    DeserializeOwned, Deserializer, EnumAccess, IntoDeserializer, MapAccess, SeqAccess,
    VariantAccess, Visitor,
};

use super::{ExsValue, SerdeError};

/// Deserializes one owned ExS boundary value into a Rust value.
pub(super) fn deserialize<T: DeserializeOwned>(value: ExsValue) -> Result<T, SerdeError> {
    T::deserialize(ValueDeserializer(value))
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
