//! Static decode schemas of `let` and `for` type annotations.
//!
//! The compiler lays out one record tree per distinct annotation in a program's static data;
//! the core library walks it to check or convert a value. Addresses are absolute and 0 marks an
//! absent part. Names are raw `ValueRef`s of static Strings.

/// One recursive type contract.
#[repr(C)]
pub struct WireContract {
    /// Built-in type flags the value may carry.
    pub mask: u32,
    /// Address of the contract of List elements, or 0 when Lists are not converted.
    pub list: u32,
    /// Address of the [`WireObject`] a host Object converts to, or 0. Only the first
    /// nominal Object alternative of a union is used.
    pub object: u32,
    /// Number of [`WireEnum`] alternatives.
    pub enum_count: u32,
    /// Address of the array of [`WireEnum`] alternatives.
    pub enums: u32,
    /// Reserved, keeps records 8-byte aligned.
    pub reserved: u32,
}

/// A nominal Object type a host Object converts to.
#[repr(C)]
pub struct WireObject {
    /// Nominal type id.
    pub type_id: u32,
    /// Number of declared fields.
    pub field_count: u32,
    /// Address of the array of [`WireField`]s in declaration order.
    pub fields: u32,
    /// Reserved, keeps records 8-byte aligned.
    pub reserved: u32,
}

/// One named field of an Object or enum payload.
#[repr(C)]
pub struct WireField {
    /// Raw `ValueRef` of the static field-name String.
    pub name: u64,
    /// Address of the field's [`WireContract`].
    pub contract: u32,
    /// Reserved, keeps records 8-byte aligned.
    pub reserved: u32,
}

/// An enum type a tagged host Object converts to.
#[repr(C)]
pub struct WireEnum {
    /// Nominal enum type id.
    pub type_id: u32,
    /// Number of [`WireVariant`]s.
    pub variant_count: u32,
    /// Address of the array of [`WireVariant`]s.
    pub variants: u32,
    /// Reserved, keeps records 8-byte aligned.
    pub reserved: u32,
}

/// One enum variant and its payload fields.
#[repr(C)]
pub struct WireVariant {
    /// Raw `ValueRef` of the static variant-name String.
    pub name: u64,
    /// Index of the variant inside its enum declaration.
    pub index: u32,
    /// Number of payload [`WireField`]s.
    pub field_count: u32,
    /// Address of the array of payload [`WireField`]s in declaration order.
    pub fields: u32,
    /// Reserved, keeps records 8-byte aligned.
    pub reserved: u32,
}

/// Asserts that a record ends 8-byte aligned.
macro_rules! assert_aligned {
    ($($record:ty),* $(,)?) => {
        $(const _: () = assert!(core::mem::size_of::<$record>().is_multiple_of(8));)*
    };
}

assert_aligned!(WireContract, WireObject, WireField, WireEnum, WireVariant);
