#![no_std]

//! Linear-memory layout shared by the ExS compiler, the stateless core library and runners.
//!
//! A compiled module keeps every value and all mutable execution state in its linear memory.
//! This crate defines that memory once: the tagged `ValueRef` and its type masks, the object
//! header, the payload layout of every object kind, heap pages, size classes, the VM state
//! block and where each object kind stores references. Every layout is `#[repr(C)]` with compile-time checked offsets, so generated
//! code can address fields by constant offsets that cannot drift from the Rust definitions.
//!
//! Addresses are `u32` offsets into linear memory. A heap object is always 8-byte aligned and
//! starts with an [`ObjectHeader`].

mod builtin_method;
mod case_mapping;
#[cfg(test)]
mod case_mapping_tests;
mod case_tables;
mod codes;
mod header;
mod objects;
mod page;
mod size_class;
mod statics;
mod trace;
mod types;
pub mod value;
mod vm;
mod wire;

pub use builtin_method::BuiltinMethod;
pub use case_mapping::{CASE_TABLES, CASE_TABLES_UNICODE_VERSION, CaseTables};
pub use codes::{RANGE_BEGIN_FALLBACK, RANGE_STATE_FINISHED, RANGE_STATE_LAST, STATUS_READY};
pub use header::{
    GC_MARK, GC_STATIC, HEADER_CLASS_OFFSET, HEADER_GC_OFFSET, HEADER_KIND_OFFSET, HEADER_SIZE,
    HEADER_WORD_OFFSET, OBJECT_ALIGN, ObjectHeader, ObjectKind,
};
pub use objects::{
    BigIntObject, BufferObject, BytesObject, CellObject, ClosureObject, ERROR_FATAL,
    ERROR_RECOVERABLE, EnumObject, ErrorObject, FRAME_UNTRACED, FloatObject, FrameObject, FreeCell,
    HOST_ENUM_VARIANT, ListObject, ObjectObject, ShapeObject, StringObject, TaskObject,
    TypedObject,
};
pub use page::{PAGE_HEADER_SIZE, PAGE_SIZE, PageHeader, PageKind};
pub use size_class::{CLASS_COUNT, MAX_CLASS_SIZE, SIZE_CLASSES, class_for_size};
pub use statics::StaticFunction;
pub use trace::{TraceLayout, ValueSlots, trace_layout};
pub use types::{
    STANDARD_ORDERING_TYPE_ID, STANDARD_ORDERING_TYPE_IDENTITY, TYPE_ANY, TYPE_BOOL, TYPE_BYTES,
    TYPE_ERROR, TYPE_FLOAT, TYPE_FN, TYPE_INT, TYPE_LIST, TYPE_NONE, TYPE_OBJECT, TYPE_STRING,
};
pub use value::{Immediate, ValueRef};
pub use vm::{
    BoundaryState, ErrorState, HeapState, ProgramState, RootRange, RootStack, RootState,
    TASK_ROOT_COUNT, TaskState, TypeEntry, VM_FLAG_COLLECT_EVERY_ALLOCATION, VmState,
};
pub use wire::{WireContract, WireEnum, WireField, WireObject, WireVariant};
