//! The VM state block holding all mutable execution state outside heap objects.

use crate::size_class::CLASS_COUNT;

/// Flag of [`HeapState::flags`] that collects before every allocation, for root-discipline tests.
pub const VM_FLAG_COLLECT_EVERY_ALLOCATION: u32 = 1;

/// Allocator and collector state.
#[repr(C)]
pub struct HeapState {
    /// Head of each size class's free-cell list, 0 when empty.
    pub free_heads: [u32; CLASS_COUNT],
    /// Address of the first heap page.
    pub heap_start: u32,
    /// Address just past the last page carved from linear memory.
    pub heap_end: u32,
    /// First page of the first free page run, 0 when none.
    pub free_runs: u32,
    /// Bytes allocated since the last collection.
    pub allocated_since_collection: u32,
    /// Allocation volume that triggers the next collection.
    pub collection_threshold: u32,
    /// Address of the mark-stack storage, 0 before the first collection.
    pub mark_stack: u32,
    /// Capacity of the mark stack in `u32` entries.
    pub mark_stack_capacity: u32,
    /// Option bits such as [`VM_FLAG_COLLECT_EVERY_ALLOCATION`].
    pub flags: u32,
    /// Number of completed collections.
    pub collections: u32,
}

/// One range of raw `ValueRef` root slots, `start..end` in bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootRange {
    /// Address of the first 8-byte slot.
    pub start: u32,
    /// Address just past the last slot.
    pub end: u32,
}

/// Where the collector finds roots.
#[repr(C)]
pub struct RootState {
    /// Address of an array of [`RootRange`]s, 0 when there are none.
    pub ranges: u32,
    /// Number of entries in `ranges`.
    pub range_count: u32,
}

/// The shadow stack of root frames that generated functions push and pop inline.
#[repr(C)]
pub struct RootStack {
    /// The live part `start..end` of the stack; `end` is the next free slot. `roots.ranges`
    /// points here, so the collector always scans exactly the pushed frames.
    pub range: RootRange,
    /// Address just past the stack storage.
    pub limit: u32,
    /// Reserved, keeps the block 8-byte aligned.
    pub reserved: u32,
}

/// Buffers exchanged with the runner, each a collector-owned page run.
#[repr(C)]
pub struct BoundaryState {
    /// Address of the input bytes written by the runner, 0 before the first input.
    pub input: u32,
    /// Byte length of the current input.
    pub input_len: u32,
    /// Byte capacity of the input storage.
    pub input_capacity: u32,
    /// Address of the encoded result, 0 before the first result.
    pub result: u32,
    /// Byte length of the current result.
    pub result_len: u32,
    /// Byte capacity of the result storage.
    pub result_capacity: u32,
    /// Nonzero once the heap and root stack are initialized.
    pub initialized: u32,
    /// Reserved, keeps the block 8-byte aligned.
    pub reserved: u32,
}

/// Compiler-provided static metadata about the program's types.
#[repr(C)]
pub struct ProgramState {
    /// Address of an array of [`TypeEntry`]s indexed by nominal type id, 0 when there is none.
    pub types: u32,
    /// Number of entries in `types`.
    pub type_count: u32,
}

/// Static metadata of one nominal type.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypeEntry {
    /// Number of declared fields, 0 for enums.
    pub field_count: u32,
    /// Address of `field_count` raw `ValueRef`s of static field-name Strings.
    pub field_names: u32,
}

/// The block of execution state at a linker-assigned address.
#[repr(C)]
pub struct VmState {
    /// Allocator and collector state.
    pub heap: HeapState,
    /// Root locations.
    pub roots: RootState,
    /// The root-frame shadow stack.
    pub root_stack: RootStack,
    /// Runner input and result buffers.
    pub boundary: BoundaryState,
    /// Static program metadata.
    pub program: ProgramState,
}

const _: () = assert!(core::mem::size_of::<VmState>().is_multiple_of(8));
