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
    /// Per size class, the address of the first never-linked cell of the class's current page,
    /// 0 when the class has no partly linked page. Cells from there on hold no object and may
    /// hold stale bytes, so the collector never looks at them.
    pub class_cursors: [u32; CLASS_COUNT],
    /// Reserved, keeps the block 8-byte aligned.
    pub reserved: u32,
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
    /// The `ValueRef` fields of [`TaskState`], the second entry of the root range array.
    pub task_roots: RootRange,
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
    /// Nonzero once the heap and root stack are initialized.
    pub initialized: u32,
    /// The completed entry's result, which the runner reads from memory.
    pub result_value: u64,
}

/// Language Error context: the trace of active source functions and the current position.
///
/// A trace entry is a `u64` of `function_id | call_site << 32`; generated functions push one
/// on entry and pop it before returning.
#[repr(C)]
pub struct ErrorState {
    /// Address of the first trace entry.
    pub trace_base: u32,
    /// Address of the next free trace entry.
    pub trace_top: u32,
    /// Address just past the trace storage.
    pub trace_limit: u32,
    /// Source position id of the call site the next function entry records, 0 for none.
    pub pending_call_site: u32,
    /// Source position id new Errors record as their origin, 0 for none.
    pub source_position: u32,
    /// Reserved, keeps the block 8-byte aligned.
    pub reserved: u32,
}

/// Compiler-provided static metadata about the program's types.
#[repr(C)]
pub struct ProgramState {
    /// Address of an array of [`TypeEntry`]s sorted by nominal type id, one per type the program
    /// uses, 0 when there is none.
    pub types: u32,
    /// Number of entries in `types`.
    pub type_count: u32,
    /// Address of the empty root shape of untyped Objects, 0 before the first Object.
    ///
    /// The collector treats it as a root, so shapes are never collected.
    pub root_shape: u32,
    /// Address of the program's copy of `CASE_TABLES`, 0 when it changes no case.
    pub case_tables: u32,
}

/// Static metadata of one nominal type.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypeEntry {
    /// Number of declared fields of an Object type, or of variants of an enum.
    pub field_count: u32,
    /// Address of `field_count` raw `ValueRef`s of static field-name or variant-name Strings.
    pub field_names: u32,
    /// Raw `ValueRef` of the static identity String of an enum, or of the source name of an
    /// Object type.
    pub identity: u64,
    /// The nominal type id the entry describes.
    pub type_id: u32,
    /// [`TYPE_ENTRY_OBJECT`] or [`TYPE_ENTRY_ENUM`], combined with
    /// [`TYPE_ENTRY_CUSTOM_EQUALITY`] and [`TYPE_ENTRY_CUSTOM_RENDERING`].
    pub kind: u32,
}

/// [`TypeEntry::kind`] of a nominal Object type.
pub const TYPE_ENTRY_OBJECT: u32 = 0;
/// [`TypeEntry::kind`] bit of an enum.
pub const TYPE_ENTRY_ENUM: u32 = 1;
/// [`TypeEntry::kind`] bit of a type with an `Eq` or `Compare` implementation, whose
/// equality only the program decides.
pub const TYPE_ENTRY_CUSTOM_EQUALITY: u32 = 2;
/// [`TypeEntry::kind`] bit of a type with a `Debug` or `ToString` implementation, whose
/// rendering only the program decides.
pub const TYPE_ENTRY_CUSTOM_RENDERING: u32 = 4;

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
    /// Language Error context.
    pub errors: ErrorState,
    /// The execution's task of resumable functions.
    pub task: TaskState,
}

/// The task that runs the frames of resumable functions.
///
/// The first seven fields are references the collector scans as a root range.
#[repr(C)]
pub struct TaskState {
    /// The running task's active frame, a reference of kind `KIND_FRAME`, or 0.
    pub frame: u64,
    /// The host result the running task consumes next, or 0.
    pub ready: u64,
    /// The result of the completed root task, or 0.
    pub completed: u64,
    /// The running task, a reference of kind `KIND_FRAME`, or 0 while every task waits.
    pub current: u64,
    /// The first task of the run queue, or 0.
    pub queue_head: u64,
    /// The last task of the run queue, or 0.
    pub queue_tail: u64,
    /// The first live task, or 0.
    pub tasks: u64,
    /// The id of the host call the running task waits for or consumes, 0 for none.
    pub host_call: u64,
    /// The id of the next host call.
    pub next_host_call: u64,
    /// The running task's remaining scheduler checkpoints.
    pub quantum: u64,
}

/// Number of leading [`TaskState`] fields the collector scans as roots.
pub const TASK_ROOT_COUNT: u32 = 7;

const _: () = assert!(core::mem::size_of::<VmState>().is_multiple_of(8));
