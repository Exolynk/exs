//! The `__exs_core_*` exports, each a trapping wrapper around one `exs-core` function.

use exs_core::{Arithmetic, LinearMemory, Ordering};

/// Returns access to the module's linear memory.
fn memory() -> LinearMemory {
    // SAFETY: generated code only passes addresses of its VM state block and heap.
    unsafe { LinearMemory::new() }
}

/// Stops execution after an unrecoverable failure.
fn trap() -> ! {
    core::arch::wasm32::unreachable()
}

/// Converts a Wasm `i32` address argument.
fn address(value: i32) -> u32 {
    value.cast_unsigned()
}

/// Global allocator that refuses every request.
///
/// The core library keeps all data in the module heap and never allocates through Rust; a
/// refused request reaches the panic handler and traps, so an accidental allocation is caught
/// instead of creating a second heap.
struct RefusingAllocator;

// SAFETY: refusing every allocation with a null pointer satisfies the `GlobalAlloc` contract.
unsafe impl core::alloc::GlobalAlloc for RefusingAllocator {
    unsafe fn alloc(&self, _layout: core::alloc::Layout) -> *mut u8 {
        core::ptr::null_mut()
    }

    unsafe fn dealloc(&self, _pointer: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static ALLOCATOR: RefusingAllocator = RefusingAllocator;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    trap()
}

#[link(wasm_import_module = "runner")]
unsafe extern "C" {
    /// Registers one task with the runner's task metering.
    fn __runner_task_acquire() -> i32;
    /// Releases one task from the runner's task metering.
    fn __runner_task_release() -> i32;
}

/// Registers the root task with the runner, trapping when the runner refuses it.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_task_acquire() {
    // SAFETY: the runner provides this import for every module.
    if unsafe { __runner_task_acquire() } != 0 {
        trap();
    }
}

/// Releases the root task from the runner's task metering.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_task_release() {
    // SAFETY: the runner provides this import for every module.
    if unsafe { __runner_task_release() } != 0 {
        trap();
    }
}

/// Initializes the VM state block once, with the heap from `heap_start`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_init(vm: i32, heap_start: i32, root_stack_pages: i32) {
    let result = exs_core::init_execution(
        &mut memory(),
        address(vm),
        address(heap_start),
        address(root_stack_pages),
    );
    if result.is_none() {
        trap();
    }
}

/// Returns storage for `length` runner input bytes.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_reserve_input(vm: i32, length: i32) -> i32 {
    match exs_core::reserve_input(&mut memory(), address(vm), address(length)) {
        Some(input) => input.cast_signed(),
        None => trap(),
    }
}

/// Decodes the runner's arguments into `capacity` root slots and returns the argument count.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_decode_inputs(vm: i32, slots: i32, capacity: i32) -> i32 {
    match exs_core::decode_inputs(
        &mut memory(),
        address(vm),
        address(slots),
        address(capacity),
    ) {
        Ok(count) => count.cast_signed(),
        Err(_) => trap(),
    }
}

/// Encodes the result value for the runner.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_encode_result(vm: i32, value: i64) {
    if exs_core::encode_result(&mut memory(), address(vm), value.cast_unsigned()).is_err() {
        trap();
    }
}

/// Returns the address of the encoded result.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_result_ptr(vm: i32) -> i32 {
    exs_core::result(&memory(), address(vm)).0.cast_signed()
}

/// Returns the byte length of the encoded result.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_result_len(vm: i32) -> i32 {
    exs_core::result(&memory(), address(vm)).1.cast_signed()
}

/// Allocates one zeroed heap object of `size` bytes with header kind `kind`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_allocate(vm: i32, size: i32, kind: i32) -> i32 {
    let Some(kind) = u8::try_from(kind).ok().and_then(exs_layout_kind) else {
        trap();
    };
    match exs_core::allocate(&mut memory(), address(vm), address(size), kind) {
        Some(object) => object.cast_signed(),
        None => trap(),
    }
}

/// Converts one kind byte.
fn exs_layout_kind(byte: u8) -> Option<exs_core::ObjectKind> {
    exs_core::ObjectKind::from_byte(byte)
}

/// Runs one full collection.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_collect(vm: i32) {
    exs_core::collect(&mut memory(), address(vm));
}

/// Applies `+` (0), `-` (1) or `*` (2) to two numeric values.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_arithmetic(vm: i32, operator: i32, left: i64, right: i64) -> i64 {
    let operator = match operator {
        0 => Arithmetic::Add,
        1 => Arithmetic::Subtract,
        2 => Arithmetic::Multiply,
        _ => trap(),
    };
    match exs_core::arithmetic(
        &mut memory(),
        address(vm),
        operator,
        left.cast_unsigned(),
        right.cast_unsigned(),
    ) {
        Ok(value) => value.cast_signed(),
        Err(_) => trap(),
    }
}

/// Divides two numeric values into a Float.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_divide(vm: i32, left: i64, right: i64) -> i64 {
    match exs_core::divide(
        &mut memory(),
        address(vm),
        left.cast_unsigned(),
        right.cast_unsigned(),
    ) {
        Ok(value) => value.cast_signed(),
        Err(_) => trap(),
    }
}

/// Negates one numeric value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_negate(vm: i32, value: i64) -> i64 {
    match exs_core::negate(&mut memory(), address(vm), value.cast_unsigned()) {
        Ok(value) => value.cast_signed(),
        Err(_) => trap(),
    }
}

/// Compares two numeric values with `<` (0), `<=` (1), `>` (2) or `>=` (3).
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_compare(operator: i32, left: i64, right: i64) -> i32 {
    let operator = match operator {
        0 => Ordering::Less,
        1 => Ordering::LessOrEqual,
        2 => Ordering::Greater,
        3 => Ordering::GreaterOrEqual,
        _ => trap(),
    };
    match exs_core::compare(
        &memory(),
        operator,
        left.cast_unsigned(),
        right.cast_unsigned(),
    ) {
        Ok(result) => i32::from(result),
        Err(_) => trap(),
    }
}

/// Returns 1 when two values are equal by the language's equality.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_equal(left: i64, right: i64) -> i32 {
    i32::from(exs_core::equal(
        &memory(),
        left.cast_unsigned(),
        right.cast_unsigned(),
    ))
}

/// Returns the reference of one Int, allocating it outside the `i32` range.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_int(vm: i32, value: i64) -> i64 {
    match exs_core::int_value(&mut memory(), address(vm), value) {
        Some(value) => value.cast_signed(),
        None => trap(),
    }
}

/// Returns the reference of one newly allocated Float with the given bits.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_float(vm: i32, bits: i64) -> i64 {
    match exs_core::float_value(
        &mut memory(),
        address(vm),
        f64::from_bits(bits.cast_unsigned()),
    ) {
        Some(value) => value.cast_signed(),
        None => trap(),
    }
}
