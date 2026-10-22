//! Enum values: an `EnumObject` of a nominal enum type, a variant index and payload fields.
//!
//! Enums are Objects: their references carry the Object kind and the enum's type
//! tag, and they have no keys. The program's type table names their identity and variants.

use core::mem::{offset_of, size_of};

use exs_layout::ValueRef;
use exs_layout::value::layout::{KIND_SHIFT, nominal_extra, type_kind};
use exs_layout::{EnumObject, HOST_ENUM_VARIANT, ObjectKind};
use exs_layout::{STANDARD_ORDERING_TYPE_ID, TYPE_OBJECT};

use crate::allocator::allocate;
use crate::memory::Memory;
use crate::strings::string_object;
use crate::typed::type_entry;

/// Byte offset of the enum type id in the header word.
const TYPE_ID: u32 = 4;
/// Byte offset of [`EnumObject::variant`].
const VARIANT: u32 = offset_of!(EnumObject, variant) as u32;
/// Byte offset of [`EnumObject::field_count`].
const FIELD_COUNT: u32 = offset_of!(EnumObject, field_count) as u32;
/// Byte offset of the first payload field.
const FIELDS: u32 = size_of::<EnumObject>() as u32;

/// Returns the address of the enum behind one raw reference, or `None` for other values.
pub(crate) fn enum_object(memory: &impl Memory, raw: u64) -> Option<u32> {
    let object = raw as u32;
    (raw >> KIND_SHIFT == u64::from(type_kind(TYPE_OBJECT))
        && object != 0
        && memory.load_u8(object) == ObjectKind::Enum as u8)
        .then_some(object)
}

/// Returns the enum type id of the enum at `object`.
pub(crate) fn enum_type(memory: &impl Memory, object: u32) -> u32 {
    memory.load_u32(object + TYPE_ID)
}

/// Returns the variant index of the enum at `object`.
pub(crate) fn variant(memory: &impl Memory, object: u32) -> u32 {
    memory.load_u32(object + VARIANT)
}

/// Returns the number of slots of the enum at `object`: its payload fields, and for a host
/// enum also its two name slots.
pub(crate) fn field_count(memory: &impl Memory, object: u32) -> u32 {
    memory.load_u32(object + FIELD_COUNT)
}

/// Returns the number of payload fields of the enum at `object`.
pub(crate) fn payload_count(memory: &impl Memory, object: u32) -> u32 {
    let count = field_count(memory, object);
    if variant(memory, object) == HOST_ENUM_VARIANT {
        count.saturating_sub(2)
    } else {
        count
    }
}

/// Type id of host enums whose identity the program does not declare, which no declared type
/// has.
const FOREIGN_ENUM_TYPE_ID: u32 = u32::MAX;

/// Creates a host enum with the rooted `identity` and `name` Strings and `count` empty
/// payload fields: a value of a variant or an identity the program does not declare.
pub(crate) fn host_enum(
    memory: &mut impl Memory,
    vm: u32,
    identity: u64,
    name: u64,
    count: u32,
) -> Option<u64> {
    let identity_object = string_object(identity)?;
    let type_id = (0..crate::vm::Vm(vm).program_type_count(memory))
        .filter_map(|index| crate::typed::type_entry_at(memory, vm, index))
        .find(|entry| {
            string_object(entry.identity).is_some_and(|declared| {
                crate::strings::text(memory, declared)
                    == crate::strings::text(memory, identity_object)
            })
        })
        .map_or(FOREIGN_ENUM_TYPE_ID, |entry| entry.type_id);
    let raw = allocate_enum(
        memory,
        vm,
        type_id,
        HOST_ENUM_VARIANT,
        count.checked_add(2)?,
    )?;
    memory.store_u64(field_address(raw as u32, count), identity);
    memory.store_u64(field_address(raw as u32, count + 1), name);
    Some(raw)
}

/// Returns the address of payload field `index` of the enum at `object`.
pub(crate) const fn field_address(object: u32, index: u32) -> u32 {
    object + FIELDS + index * 8
}

/// Allocates an enum of `type_id` and `variant` with `count` empty payload fields and
/// returns its reference.
pub(crate) fn allocate_enum(
    memory: &mut impl Memory,
    vm: u32,
    type_id: u32,
    variant: u32,
    count: u32,
) -> Option<u64> {
    let size = count.checked_mul(8)?.checked_add(FIELDS)?;
    let object = allocate(memory, vm, size, ObjectKind::Enum)?;
    memory.store_u32(object + TYPE_ID, type_id);
    memory.store_u32(object + VARIANT, variant);
    memory.store_u32(object + FIELD_COUNT, count);
    let extra = nominal_extra(type_id);
    ValueRef::from_parts(type_kind(TYPE_OBJECT), extra, object).map(ValueRef::raw)
}

/// Creates an enum of `type_id` and `variant` holding the `count` rooted values at `slots`.
pub fn enum_new(
    memory: &mut impl Memory,
    vm: u32,
    type_id: u32,
    variant: u32,
    slots: u32,
    count: u32,
) -> Option<u64> {
    let raw = allocate_enum(memory, vm, type_id, variant, count)?;
    memory.copy(field_address(raw as u32, 0), slots, count * 8);
    Some(raw)
}

/// Returns the identity String and the variant-name String of the enum at `object`, as the
/// program's type table or, for a host enum, its name slots name them.
pub(crate) fn names(memory: &impl Memory, vm: u32, object: u32) -> Option<(u32, u32)> {
    let index = variant(memory, object);
    if index == HOST_ENUM_VARIANT {
        let count = payload_count(memory, object);
        let identity = string_object(memory.load_u64(field_address(object, count)))?;
        let name = string_object(memory.load_u64(field_address(object, count + 1)))?;
        return Some((identity, name));
    }
    let entry = type_entry(memory, vm, enum_type(memory, object))?;
    if index >= entry.field_count {
        return None;
    }
    let identity = string_object(entry.identity)?;
    let name = string_object(memory.load_u64(entry.field_names + index * 8))?;
    Some((identity, name))
}

/// An outcome of the built-in ordering of two values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Order {
    /// The left value orders first.
    Less,
    /// Both values are equal.
    Equal,
    /// The left value orders last.
    Greater,
    /// The values have no order.
    Unordered,
}

/// Creates the `std::Ordering` value of `order`, a new enum.
pub(crate) fn ordering(memory: &mut impl Memory, vm: u32, order: Order) -> Option<u64> {
    let index = match order {
        Order::Less => 0,
        Order::Equal => 1,
        Order::Greater => 2,
        Order::Unordered => 3,
    };
    allocate_enum(memory, vm, STANDARD_ORDERING_TYPE_ID, index, 0)
}

/// Creates the MatchError for a payload field a matched enum value lacks.
pub fn missing_field(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    crate::errors::recoverable(
        memory,
        vm,
        "MatchError",
        "enum payload field is unavailable",
        raw,
    )
}

/// Creates the MatchError of a `match` whose arms accept none of `raw`.
pub fn match_error(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    crate::errors::recoverable(
        memory,
        vm,
        "MatchError",
        "no match arm accepted the value",
        raw,
    )
}

/// Finds the enum type whose identity and the variant whose name equal the text at the
/// `(address, length)` ranges, returning the type id and variant index.
pub(crate) fn resolve_enum(
    memory: &impl Memory,
    vm: u32,
    identity: (u32, u32),
    variant: (u32, u32),
) -> Option<(u32, u32)> {
    let same = |object: u32, text: (u32, u32)| {
        crate::strings::text(memory, object).as_bytes() == memory.bytes(text.0, text.1)
    };
    let count = crate::vm::Vm(vm).program_type_count(memory);
    (0..count).find_map(|index| {
        let entry = crate::typed::type_entry_at(memory, vm, index)?;
        let type_id = entry.type_id;
        let name = string_object(entry.identity)?;
        if !same(name, identity) {
            return None;
        }
        (0..entry.field_count).find_map(|index| {
            let candidate = string_object(memory.load_u64(entry.field_names + index * 8))?;
            same(candidate, variant).then_some((type_id, index))
        })
    })
}
