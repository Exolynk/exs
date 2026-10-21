#![no_std]

//! Linear-memory layout shared by the ExS compiler, the stateless core library and runners.
//!
//! Runtime v2 keeps every value and all mutable execution state in the module's linear memory.
//! This crate defines that memory once: the object header, the payload layout of every object
//! kind, heap pages, size classes, the VM state block and where each object kind stores
//! references. Every layout is `#[repr(C)]` with compile-time checked offsets, so generated
//! code can address fields by constant offsets that cannot drift from the Rust definitions.
//!
//! Addresses are `u32` offsets into linear memory. A heap object is always 8-byte aligned and
//! starts with an [`ObjectHeader`].

mod header;
mod objects;
mod page;
mod size_class;
mod trace;
mod vm;

pub use header::{
    GC_MARK, GC_STATIC, HEADER_CLASS_OFFSET, HEADER_GC_OFFSET, HEADER_KIND_OFFSET, HEADER_SIZE,
    HEADER_WORD_OFFSET, OBJECT_ALIGN, ObjectHeader, ObjectKind,
};
pub use objects::{
    BigIntObject, BufferObject, BytesObject, CellObject, ClosureObject, ERROR_FATAL,
    ERROR_RECOVERABLE, EnumObject, ErrorObject, FloatObject, FrameObject, FreeCell, ListObject,
    ObjectObject, ShapeObject, StringObject, TypedObject,
};
pub use page::{PAGE_HEADER_SIZE, PAGE_SIZE, PageHeader, PageKind};
pub use size_class::{CLASS_COUNT, MAX_CLASS_SIZE, SIZE_CLASSES, class_for_size};
pub use trace::{TraceLayout, ValueSlots, trace_layout};
pub use vm::{
    BoundaryState, ErrorState, HeapState, ProgramState, RootRange, RootStack, RootState, TypeEntry,
    VM_FLAG_COLLECT_EVERY_ALLOCATION, VmState,
};
