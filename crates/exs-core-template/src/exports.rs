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

/// Returns a produced value, trapping when memory was exhausted.
fn value(result: Option<u64>) -> i64 {
    match result {
        Some(value) => value.cast_signed(),
        None => trap(),
    }
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

/// Adopts the runner's argument image into `capacity` root slots and returns the argument
/// count.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_adopt_inputs(vm: i32, slots: i32, capacity: i32) -> i32 {
    match exs_core::image_inputs(
        &mut memory(),
        address(vm),
        address(slots),
        address(capacity),
    ) {
        Ok(count) => count.cast_signed(),
        Err(_) => trap(),
    }
}

/// Adopts the runner's argument image of a variadic entry with `fixed` parameters into the
/// rooted `slots`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_adopt_variadic_inputs(vm: i32, slots: i32, fixed: i32) {
    if exs_core::image_variadic_inputs(
        &mut memory(),
        address(vm),
        address(slots),
        fixed.cast_unsigned(),
    )
    .is_err()
    {
        trap();
    }
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

/// Applies `+` (0), `-` (1) or `*` (2) to two numeric values; failures are Error values.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_arithmetic(vm: i32, operator: i32, left: i64, right: i64) -> i64 {
    let operator = match operator {
        0 => Arithmetic::Add,
        1 => Arithmetic::Subtract,
        2 => Arithmetic::Multiply,
        _ => trap(),
    };
    value(exs_core::arithmetic(
        &mut memory(),
        address(vm),
        operator,
        left.cast_unsigned(),
        right.cast_unsigned(),
    ))
}

/// Divides two numeric values into a Float; a non-numeric operand is a TypeError value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_divide(vm: i32, left: i64, right: i64) -> i64 {
    value(exs_core::divide(
        &mut memory(),
        address(vm),
        left.cast_unsigned(),
        right.cast_unsigned(),
    ))
}

/// Negates one numeric value; failures are Error values.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_negate(vm: i32, operand: i64) -> i64 {
    value(exs_core::negate(
        &mut memory(),
        address(vm),
        operand.cast_unsigned(),
    ))
}

/// Compares two values with `<` (0), `<=` (1), `>` (2) or `>=` (3) and returns the Bool, or
/// the TypeError for values without an order.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_compare_value(vm: i32, operator: i32, left: i64, right: i64) -> i64 {
    let operator = match operator {
        0 => Ordering::Less,
        1 => Ordering::LessOrEqual,
        2 => Ordering::Greater,
        3 => Ordering::GreaterOrEqual,
        _ => trap(),
    };
    value(exs_core::compare_value(
        &mut memory(),
        address(vm),
        operator,
        left.cast_unsigned(),
        right.cast_unsigned(),
    ))
}

/// Creates an enum of `type_id` and `variant` holding the `count` rooted values at `slots`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_enum_new(
    vm: i32,
    type_id: i32,
    variant: i32,
    slots: i32,
    count: i32,
) -> i64 {
    value(exs_core::enum_new(
        &mut memory(),
        address(vm),
        type_id.cast_unsigned(),
        variant.cast_unsigned(),
        address(slots),
        count.cast_unsigned(),
    ))
}

/// Creates the MatchError for a payload field the matched enum `matched` lacks.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_enum_missing_field(vm: i32, matched: i64) -> i64 {
    value(exs_core::missing_field(
        &mut memory(),
        address(vm),
        matched.cast_unsigned(),
    ))
}

/// Creates the MatchError of a `match` whose arms accept none of `matched`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_match_error(vm: i32, matched: i64) -> i64 {
    value(exs_core::match_error(
        &mut memory(),
        address(vm),
        matched.cast_unsigned(),
    ))
}

/// Compares two values as far as possible without their parts: 0 different, 1 equal, 2 when
/// their parts decide (see `exs_core::equality`).
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_equal(vm: i32, left: i64, right: i64) -> i32 {
    exs_core::equality(
        &memory(),
        address(vm),
        left.cast_unsigned(),
        right.cast_unsigned(),
    )
    .cast_signed()
}

/// Returns a new List of the parts of a composite value, or None (see `exs_core::parts`).
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_parts(vm: i32, raw: i64) -> i64 {
    value(exs_core::parts(
        &mut memory(),
        address(vm),
        raw.cast_unsigned(),
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

/// Evaluates `slots[0] + ... + slots[count - 1]` with the language's `+`, taking the origin of
/// step `i` from `positions[i - 1]` when `positions` is not 0.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_concat(vm: i32, slots: i32, count: i32, positions: i32) -> i64 {
    value(exs_core::concat(
        &mut memory(),
        address(vm),
        address(slots),
        address(count),
        address(positions),
    ))
}

/// Creates a List of the `count` values in the root slots at `slots`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_list_new(vm: i32, slots: i32, count: i32) -> i64 {
    match exs_core::list_new(&mut memory(), address(vm), address(slots), address(count)) {
        Some(list) => list.cast_signed(),
        None => trap(),
    }
}

/// Returns a shallow copy of one rooted List, trapping for other values.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_list_copy(vm: i32, list: i64) -> i64 {
    match exs_core::list_copy(&mut memory(), address(vm), list.cast_unsigned()) {
        Some(list) => list.cast_signed(),
        None => trap(),
    }
}

/// Evaluates `receiver[index]`; failures are Error values.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_index_get(vm: i32, receiver: i64, index: i64) -> i64 {
    value(exs_core::index_get(
        &mut memory(),
        address(vm),
        receiver.cast_unsigned(),
        index.cast_unsigned(),
    ))
}

/// Evaluates `receiver[index] = value` and returns the value or the Error it produced.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_index_set(vm: i32, receiver: i64, index: i64, stored: i64) -> i64 {
    value(exs_core::index_set(
        &mut memory(),
        address(vm),
        receiver.cast_unsigned(),
        index.cast_unsigned(),
        stored.cast_unsigned(),
    ))
}

/// Reads the property named by the String `name`; failures are Error values.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_property_get(vm: i32, receiver: i64, name: i64) -> i64 {
    value(exs_core::property_get(
        &mut memory(),
        address(vm),
        receiver.cast_unsigned(),
        name.cast_unsigned(),
    ))
}

/// Writes the property named by the String `name` and returns the value or the Error it
/// produced.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_property_set(vm: i32, receiver: i64, name: i64, stored: i64) -> i64 {
    value(exs_core::property_set(
        &mut memory(),
        address(vm),
        receiver.cast_unsigned(),
        name.cast_unsigned(),
        stored.cast_unsigned(),
    ))
}

/// Returns the shape of the `count` distinct static key Strings at `keys`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_shape_for(vm: i32, keys: i32, count: i32) -> i32 {
    match exs_core::shape_for(&mut memory(), address(vm), address(keys), address(count)) {
        Some(shape) => shape.cast_signed(),
        None => trap(),
    }
}

/// Creates an untyped Object of `shape` from `count` values in the root slots at `slots`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_object_with_shape(vm: i32, shape: i32, slots: i32, count: i32) -> i64 {
    match exs_core::object_with_shape(
        &mut memory(),
        address(vm),
        address(shape),
        address(slots),
        address(count),
    ) {
        Some(object) => object.cast_signed(),
        None => trap(),
    }
}

/// Returns `shape << 32 | position` of the key named by `name` in an untyped Object, or -1.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_object_lookup(receiver: i64, name: i64) -> i64 {
    exs_core::object_lookup(&memory(), receiver.cast_unsigned(), name.cast_unsigned())
        .map_or(-1, u64::cast_signed)
}

/// Returns what a `for` loop iterates over: a List copy or a NotIterable Error.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_for_snapshot(vm: i32, iterable: i64) -> i64 {
    value(exs_core::for_snapshot(
        &mut memory(),
        address(vm),
        iterable.cast_unsigned(),
    ))
}

/// Evaluates `!operand` for a value that is no Bool, which is a TypeError value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_not_error(vm: i32, operand: i64) -> i64 {
    value(exs_core::not_error(
        &mut memory(),
        address(vm),
        operand.cast_unsigned(),
    ))
}

/// Creates the TypeError of a condition that is no Bool.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_condition_error(vm: i32, condition: i64) -> i64 {
    value(exs_core::condition_error(
        &mut memory(),
        address(vm),
        condition.cast_unsigned(),
    ))
}

/// Creates the TypeError of a declared-type violation, recoverable when `recoverable` is 1.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_type_mismatch(vm: i32, mismatch: i64, recoverable: i32) -> i64 {
    value(exs_core::type_mismatch(
        &mut memory(),
        address(vm),
        mismatch.cast_unsigned(),
        recoverable != 0,
    ))
}

/// Creates the WireDecodeError of a `let` binding whose value violates its declared type.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_wire_mismatch(vm: i32, mismatch: i64) -> i64 {
    value(exs_core::wire_mismatch(
        &mut memory(),
        address(vm),
        mismatch.cast_unsigned(),
    ))
}

/// Decodes the rooted `decoded` against the static wire schema at `schema`, returning the
/// decoded value or the Error that rejects it.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_wire_decode(vm: i32, decoded: i64, schema: i32) -> i64 {
    value(exs_core::wire_decode(
        &mut memory(),
        address(vm),
        decoded.cast_unsigned(),
        address(schema),
    ))
}

/// Applies the comparison `test` to the Ordering a `Compare` implementation returned.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_ordering_test(vm: i32, ordering: i64, test: i32) -> i64 {
    value(exs_core::ordering_test(
        &mut memory(),
        address(vm),
        ordering.cast_unsigned(),
        test.cast_unsigned(),
    ))
}

/// Creates the ArityError of an implementation method called with the wrong arguments.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_nominal_arity_error(vm: i32, receiver: i64) -> i64 {
    value(exs_core::nominal_arity_error(
        &mut memory(),
        address(vm),
        receiver.cast_unsigned(),
    ))
}

/// Parses `text` as strict RFC 3339 into the raw DateTime fields at the rooted `slots`,
/// returning None, or returns the Error that rejects it.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_datetime_parse(vm: i32, text: i64, slots: i32) -> i64 {
    value(exs_core::datetime_parse(
        &mut memory(),
        address(vm),
        text.cast_unsigned(),
        address(slots),
    ))
}

/// Creates the closure of index `index` capturing the `count` rooted Cells at `slots`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_closure_new(vm: i32, index: i32, slots: i32, count: i32) -> i64 {
    value(exs_core::closure_new(
        &mut memory(),
        address(vm),
        index.cast_unsigned(),
        address(slots),
        count.cast_unsigned(),
    ))
}

/// Creates a Cell holding the rooted `held`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_cell_new(vm: i32, held: i64) -> i64 {
    value(exs_core::cell_new(
        &mut memory(),
        address(vm),
        held.cast_unsigned(),
    ))
}

/// Creates the ArityError of a closure called with the wrong number of arguments.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_closure_arity_error(vm: i32) -> i64 {
    value(exs_core::closure_arity_error(&mut memory(), address(vm)))
}

/// Creates the TypeError of a call of a value that is no closure.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_not_callable(vm: i32, callee: i64) -> i64 {
    value(exs_core::not_callable(
        &mut memory(),
        address(vm),
        callee.cast_unsigned(),
    ))
}

/// Creates the NotIterable Error of a `for` loop over an Object that implements no Iterator.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_not_an_iterator(vm: i32, iterated: i64) -> i64 {
    value(exs_core::not_an_iterator(
        &mut memory(),
        address(vm),
        iterated.cast_unsigned(),
    ))
}

/// Advances the prelude Range `range` of type `range_type` natively, returning the item, 0
/// once exhausted, or -1 when `Range::next` must take the step.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_range_step(range: i64, range_type: i32) -> i64 {
    exs_core::range_step(
        &mut memory(),
        range.cast_unsigned(),
        range_type.cast_unsigned(),
    )
    .cast_signed()
}

/// Starts counting an unobserved Range variable, returning its packed bounds or a fallback.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_range_begin(range: i64, range_type: i32) -> i64 {
    exs_core::range_begin(&memory(), range.cast_unsigned(), range_type.cast_unsigned())
        .cast_signed()
}

/// Writes a counted Range variable's state back once its loop exits.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_range_finish(range: i64, range_type: i32, cursor: i64, state: i64) {
    exs_core::range_finish(
        &mut memory(),
        range.cast_unsigned(),
        range_type.cast_unsigned(),
        cursor,
        state,
    );
}

#[link(wasm_import_module = "exs")]
unsafe extern "C" {
    /// Starts one host call through the runner, which reads the arguments from memory.
    fn __exs_host_call_start_value(
        call_id: i64,
        name_pointer: i32,
        name_length: i32,
        arguments: i64,
        vm: i32,
        source_position: i32,
    ) -> i32;
    /// Returns the byte length of one ready host response.
    fn __exs_host_call_response_len(call_id: i64) -> i32;
    /// Copies one ready host response into linear memory.
    fn __exs_host_call_response_copy(call_id: i64, pointer: i32, length: i32) -> i32;
}

/// The runner's host-call imports.
struct RunnerHost;

impl exs_core::Host for RunnerHost {
    fn start(
        &mut self,
        call_id: u64,
        name: u32,
        name_len: u32,
        arguments: u64,
        vm: u32,
        position: u32,
    ) -> i32 {
        // SAFETY: the runner provides the host-call imports for every module.
        unsafe {
            __exs_host_call_start_value(
                call_id.cast_signed(),
                name.cast_signed(),
                name_len.cast_signed(),
                arguments.cast_signed(),
                vm.cast_signed(),
                position.cast_signed(),
            )
        }
    }

    fn response_len(&mut self, call_id: u64) -> i32 {
        // SAFETY: the runner provides the host-call imports for every module.
        unsafe { __exs_host_call_response_len(call_id.cast_signed()) }
    }

    fn response_copy(&mut self, call_id: u64, address: u32, len: u32) -> i32 {
        // SAFETY: the runner provides the host-call imports for every module.
        unsafe {
            __exs_host_call_response_copy(
                call_id.cast_signed(),
                address.cast_signed(),
                len.cast_signed(),
            )
        }
    }

    fn acquire(&mut self) -> bool {
        // SAFETY: the runner provides this import for every module.
        unsafe { __runner_task_acquire() == 0 }
    }

    fn release(&mut self) {
        // SAFETY: the runner provides this import for every module.
        if unsafe { __runner_task_release() } != 0 {
            trap();
        }
    }
}

/// Starts the task with the root frame of step `step` of function `function_id`, with
/// `slots` empty slots, and returns its address.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_frame_start(vm: i32, step: i32, function_id: i32, slots: i32) -> i32 {
    match exs_core::frame_start(
        &mut memory(),
        address(vm),
        step.cast_unsigned(),
        function_id.cast_unsigned(),
        slots.cast_unsigned(),
    ) {
        Some(frame) => frame.cast_signed(),
        None => trap(),
    }
}

/// Calls step `step` of function `function_id` from the active frame with a new frame of
/// `slots` empty slots that returns into slot `destination`, and returns its address.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_frame_call(
    vm: i32,
    step: i32,
    function_id: i32,
    slots: i32,
    destination: i32,
) -> i32 {
    match exs_core::frame_call(
        &mut memory(),
        address(vm),
        step.cast_unsigned(),
        function_id.cast_unsigned(),
        slots.cast_unsigned(),
        destination.cast_unsigned(),
    ) {
        Some(frame) => frame.cast_signed(),
        None => trap(),
    }
}

/// Completes the active frame at `frame` with `result` and returns the dispatch status.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_frame_return(vm: i32, frame: i32, result: i64) -> i32 {
    match exs_core::frame_return(
        &mut memory(),
        &mut RunnerHost,
        address(vm),
        address(frame),
        result.cast_unsigned(),
    ) {
        Some(status) => status,
        None => trap(),
    }
}

/// Takes the result of the completed root frame.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_take_completed(vm: i32) -> i64 {
    exs_core::take_completed(&mut memory(), address(vm)).cast_signed()
}

/// Starts the host call of the function `name` with the List `arguments`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_host_call_start(vm: i32, name: i64, arguments: i64) -> i32 {
    match exs_core::host_call_start(
        &mut memory(),
        &mut RunnerHost,
        address(vm),
        name.cast_unsigned(),
        arguments.cast_unsigned(),
    ) {
        Some(status) => status,
        None => trap(),
    }
}

/// Takes the result of the current host call.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_host_call_take(vm: i32) -> i64 {
    value(exs_core::host_call_take(
        &mut memory(),
        &mut RunnerHost,
        address(vm),
    ))
}

/// Delivers the runner's response at `pointer` to the waiting host call `call_id`; returns 1
/// when the call was cancelled.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_host_call_resume(
    vm: i32,
    call_id: i64,
    pointer: i32,
    length: i32,
) -> i32 {
    match exs_core::host_call_resume(
        &mut memory(),
        address(vm),
        call_id.cast_unsigned(),
        address(pointer),
        length.cast_unsigned(),
    ) {
        Some(status) => status,
        None => trap(),
    }
}

/// Cancels the execution and releases the permits of its `par` children.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_task_cancel(vm: i32) {
    exs_core::task_cancel(&mut memory(), &mut RunnerHost, address(vm));
}

/// Suspends the running task for its pending host call and returns the dispatch status.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_task_suspend(vm: i32) -> i32 {
    match exs_core::task_suspend(&mut memory(), address(vm)) {
        Some(status) => status,
        None => trap(),
    }
}

/// Handles the running task's used-up quantum; returns 1 when it yielded.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_task_checkpoint(vm: i32) -> i32 {
    match exs_core::task_checkpoint(&mut memory(), address(vm)) {
        Some(yielded) => yielded,
        None => trap(),
    }
}

/// Creates the result List of a `par` expression with `count` tasks.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_parallel_new(vm: i32, count: i32) -> i64 {
    value(exs_core::parallel_new(
        &mut memory(),
        address(vm),
        count.cast_unsigned(),
    ))
}

/// Starts the `par` child `index` of the result List `group` with a root frame of step
/// `step` and `slots` empty slots, and returns the frame's address.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_task_spawn(
    vm: i32,
    group: i64,
    index: i32,
    step: i32,
    slots: i32,
) -> i32 {
    match exs_core::task_spawn(
        &mut memory(),
        &mut RunnerHost,
        address(vm),
        group.cast_unsigned(),
        index.cast_unsigned(),
        step.cast_unsigned(),
        slots.cast_unsigned(),
    ) {
        Some(frame) => frame.cast_signed(),
        None => trap(),
    }
}

/// Waits for the running task's `par` children and returns the dispatch status.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_parallel_wait(vm: i32) -> i32 {
    match exs_core::parallel_wait(&mut memory(), address(vm)) {
        Some(status) => status,
        None => trap(),
    }
}

/// Creates the TypeError for a `par` operand that is no List.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_parallel_list_error(vm: i32, operand: i64) -> i64 {
    value(exs_core::parallel_list_error(
        &mut memory(),
        address(vm),
        operand.cast_unsigned(),
    ))
}

/// Creates the MissingValue Error that `?` produces for None.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_missing_value(vm: i32) -> i64 {
    value(exs_core::missing_value(&mut memory(), address(vm)))
}

/// Evaluates the language's `Error(kind, message, data)` constructor.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_error_new(vm: i32, kind: i64, message: i64, data: i64) -> i64 {
    value(exs_core::error_new(
        &mut memory(),
        address(vm),
        kind.cast_unsigned(),
        message.cast_unsigned(),
        data.cast_unsigned(),
    ))
}

/// Creates the fatal ArityError for an entry that received too many arguments.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_arity_error(vm: i32) -> i64 {
    match exs_core::image_arity_error(&mut memory(), address(vm)) {
        Ok(error) => error.cast_signed(),
        Err(_) => trap(),
    }
}

/// Renders `slots[0..count]` into one String like the formatted strings.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_format(vm: i32, slots: i32, count: i32) -> i64 {
    value(exs_core::format(
        &mut memory(),
        address(vm),
        address(slots),
        address(count),
    ))
}

/// Calls the built-in method with `exs_core::BuiltinMethod` index `method`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_call_builtin(
    vm: i32,
    method: i32,
    receiver: i64,
    first: i64,
    second: i64,
) -> i64 {
    let Some(method) = u32::try_from(method)
        .ok()
        .and_then(exs_core::BuiltinMethod::from_id)
    else {
        trap();
    };
    value(exs_core::call_builtin(
        &mut memory(),
        address(vm),
        method,
        receiver.cast_unsigned(),
        first.cast_unsigned(),
        second.cast_unsigned(),
    ))
}

/// Calls the static function with `exs_core::StaticFunction` index `function`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_call_static(vm: i32, function: i32, first: i64, second: i64) -> i64 {
    let Some(function) = u32::try_from(function)
        .ok()
        .and_then(exs_core::StaticFunction::from_id)
    else {
        trap();
    };
    value(exs_core::call_static(
        &mut memory(),
        address(vm),
        function,
        first.cast_unsigned(),
        second.cast_unsigned(),
    ))
}

/// Creates the MethodNotFound Error for `receiver`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_method_not_found(vm: i32, receiver: i64) -> i64 {
    value(exs_core::method_not_found(
        &mut memory(),
        address(vm),
        receiver.cast_unsigned(),
    ))
}

/// Creates the ArityError of a built-in method that expects `expected` arguments but received
/// the `count` values in the root slots at `slots`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_method_arity_error(
    vm: i32,
    slots: i32,
    count: i32,
    expected: i32,
) -> i64 {
    value(exs_core::method_arity_error(
        &mut memory(),
        address(vm),
        address(slots),
        address(count),
        address(expected),
    ))
}

/// Evaluates `assert(condition, description)`: None, or the fatal Error of a failed assertion.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_assert(vm: i32, condition: i64, description: i64) -> i64 {
    value(exs_core::assert(
        &mut memory(),
        address(vm),
        condition.cast_unsigned(),
        description.cast_unsigned(),
    ))
}

/// Evaluates `assert_eq(actual, expected, description)` given the result `comparison` of
/// `actual == expected`: None, or the fatal Error of a failed assertion.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_core_assert_eq(
    vm: i32,
    comparison: i64,
    actual: i64,
    expected: i64,
    description: i64,
) -> i64 {
    value(exs_core::assert_eq(
        &mut memory(),
        address(vm),
        comparison.cast_unsigned(),
        actual.cast_unsigned(),
        expected.cast_unsigned(),
        description.cast_unsigned(),
    ))
}
