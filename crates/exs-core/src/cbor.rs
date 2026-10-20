//! CBOR encoding of runner inputs and results.
//!
//! The runner passes the entry arguments as one CBOR array and expects one CBOR value back.
//! This first version handles None, Bool, Int and Float; other values are reported as
//! unsupported.

use exs_value::{Immediate, ValueRef};

use crate::execution::{input, reserve_result};
use crate::memory::Memory;
use crate::numbers::{Number, float_value, int_value, number, reference};

/// Why an input or result could not be converted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CborError {
    /// The bytes are not well-formed CBOR of the expected shape.
    Malformed,
    /// The value uses a type this codec does not handle yet.
    Unsupported,
    /// The heap could not hold a decoded value or the result.
    OutOfMemory,
}

/// CBOR major type of unsigned integers.
const MAJOR_UNSIGNED: u8 = 0;
/// CBOR major type of negative integers.
const MAJOR_NEGATIVE: u8 = 1;
/// CBOR major type of arrays.
const MAJOR_ARRAY: u8 = 4;
/// CBOR major type of simple values and floats.
const MAJOR_SIMPLE: u8 = 7;
/// Encoded CBOR `false`.
const FALSE: u8 = 0xf4;
/// Encoded CBOR `true`.
const TRUE: u8 = 0xf5;
/// Encoded CBOR `null`.
const NULL: u8 = 0xf6;
/// Initial byte of a binary64 float.
const FLOAT64: u8 = 0xfb;

/// A read position inside one byte range of linear memory.
struct Reader {
    /// Address of the next byte.
    position: u32,
    /// Address just past the input.
    end: u32,
}

impl Reader {
    /// Reads one byte.
    fn byte(&mut self, memory: &impl Memory) -> Result<u8, CborError> {
        if self.position >= self.end {
            return Err(CborError::Malformed);
        }
        let byte = memory.load_u8(self.position);
        self.position += 1;
        Ok(byte)
    }

    /// Reads a big-endian unsigned integer of `count` bytes.
    fn big_endian(&mut self, memory: &impl Memory, count: u32) -> Result<u64, CborError> {
        let mut value = 0;
        for _ in 0..count {
            value = (value << 8) | u64::from(self.byte(memory)?);
        }
        Ok(value)
    }

    /// Reads the argument following an initial byte with additional information `info`.
    fn argument(&mut self, memory: &impl Memory, info: u8) -> Result<u64, CborError> {
        match info {
            0..=23 => Ok(u64::from(info)),
            24 => self.big_endian(memory, 1),
            25 => self.big_endian(memory, 2),
            26 => self.big_endian(memory, 4),
            27 => self.big_endian(memory, 8),
            _ => Err(CborError::Malformed),
        }
    }

    /// Reads one argument as a signed Int magnitude.
    fn int_argument(&mut self, memory: &impl Memory, info: u8) -> Result<i64, CborError> {
        i64::try_from(self.argument(memory, info)?).map_err(|_| CborError::Malformed)
    }

    /// Decodes one scalar item without allocating.
    fn scalar(&mut self, memory: &impl Memory) -> Result<Scalar, CborError> {
        let initial = self.byte(memory)?;
        let (major, info) = (initial >> 5, initial & 0x1f);
        Ok(match (major, info) {
            (MAJOR_UNSIGNED, _) => Scalar::Number(Number::Int(self.int_argument(memory, info)?)),
            (MAJOR_NEGATIVE, _) => {
                Scalar::Number(Number::Int(-1 - self.int_argument(memory, info)?))
            }
            (MAJOR_SIMPLE, 20) => Scalar::Immediate(ValueRef::FALSE),
            (MAJOR_SIMPLE, 21) => Scalar::Immediate(ValueRef::TRUE),
            (MAJOR_SIMPLE, 22) => Scalar::Immediate(ValueRef::NONE),
            (MAJOR_SIMPLE, 25) => {
                let bits = self.big_endian(memory, 2)? as u16;
                Scalar::Number(Number::Float(half_to_f64(bits)))
            }
            (MAJOR_SIMPLE, 26) => {
                let bits = self.big_endian(memory, 4)? as u32;
                Scalar::Number(Number::Float(f64::from(f32::from_bits(bits))))
            }
            (MAJOR_SIMPLE, 27) => {
                Scalar::Number(Number::Float(f64::from_bits(self.big_endian(memory, 8)?)))
            }
            _ => return Err(CborError::Unsupported),
        })
    }
}

/// One decoded scalar before any allocation.
enum Scalar {
    /// A value encoded entirely in its reference.
    Immediate(ValueRef),
    /// A number that may need a heap object.
    Number(Number),
}

/// Decodes the runner's argument array into the root slots at `slots`.
///
/// Stores at most `capacity` arguments, filling the remaining slots with None, and returns the
/// number of arguments the input holds. The slots must be rooted, because decoding allocates.
///
/// # Errors
///
/// Returns why the input could not be decoded.
pub fn decode_inputs(
    memory: &mut impl Memory,
    vm: u32,
    slots: u32,
    capacity: u32,
) -> Result<u32, CborError> {
    let (address, length) = input(memory, vm);
    let mut reader = Reader {
        position: address,
        end: address.checked_add(length).ok_or(CborError::Malformed)?,
    };
    let initial = reader.byte(memory)?;
    if initial >> 5 != MAJOR_ARRAY {
        return Err(CborError::Malformed);
    }
    let count = u32::try_from(reader.argument(memory, initial & 0x1f)?)
        .map_err(|_| CborError::Malformed)?;
    for index in 0..count {
        let raw = match reader.scalar(memory)? {
            Scalar::Immediate(value) => value.raw(),
            Scalar::Number(Number::Int(value)) => {
                int_value(memory, vm, value).ok_or(CborError::OutOfMemory)?
            }
            Scalar::Number(Number::Float(value)) => {
                float_value(memory, vm, value).ok_or(CborError::OutOfMemory)?
            }
        };
        if index < capacity {
            memory.store_u64(slots + index * 8, raw);
        }
    }
    for index in count..capacity {
        memory.store_u64(slots + index * 8, ValueRef::NONE.raw());
    }
    Ok(count)
}

/// Converts one IEEE 754 binary16 value to binary64.
fn half_to_f64(bits: u16) -> f64 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let fraction = f64::from(bits & 0x3ff);
    sign * match exponent {
        0 => fraction * f64::from_bits(0x3E70_0000_0000_0000),
        31 if fraction == 0.0 => f64::INFINITY,
        31 => f64::NAN,
        _ => (1.0 + fraction / 1024.0) * power_of_two(exponent - 15),
    }
}

/// Returns `2^exponent` for exponents of normal binary64 values.
fn power_of_two(exponent: i32) -> f64 {
    f64::from_bits(((exponent + 1023) as u64) << 52)
}

/// Encodes one scalar result into the VM's result buffer.
///
/// # Errors
///
/// Returns why the value could not be encoded.
pub fn encode_result(memory: &mut impl Memory, vm: u32, raw: u64) -> Result<(), CborError> {
    let mut bytes = [0_u8; 9];
    let length = match reference(raw).and_then(ValueRef::immediate) {
        Some(Immediate::None) => {
            bytes[0] = NULL;
            1
        }
        Some(Immediate::Bool(flag)) => {
            bytes[0] = if flag { TRUE } else { FALSE };
            1
        }
        _ => match number(memory, raw).ok_or(CborError::Unsupported)? {
            Number::Int(value) => encode_int(&mut bytes, value),
            Number::Float(value) => {
                bytes[0] = FLOAT64;
                bytes[1..].copy_from_slice(&value.to_bits().to_be_bytes());
                9
            }
        },
    };
    let address = reserve_result(memory, vm, length).ok_or(CborError::OutOfMemory)?;
    for (offset, byte) in (0..length).zip(bytes) {
        memory.store_u8(address + offset, byte);
    }
    Ok(())
}

/// Writes the shortest CBOR encoding of one Int and returns its length.
fn encode_int(bytes: &mut [u8; 9], value: i64) -> u32 {
    let (major, argument) = if value < 0 {
        (MAJOR_NEGATIVE, (-1 - value).cast_unsigned())
    } else {
        (MAJOR_UNSIGNED, value.cast_unsigned())
    };
    let (info, width) = match argument {
        0..=23 => (argument as u8, 0),
        24..=0xff => (24, 1),
        0x100..=0xffff => (25, 2),
        0x1_0000..=0xffff_ffff => (26, 4),
        _ => (27, 8),
    };
    bytes[0] = (major << 5) | info;
    for index in 0..width {
        bytes[1 + index] = (argument >> (8 * (width - 1 - index))) as u8;
    }
    1 + width as u32
}
