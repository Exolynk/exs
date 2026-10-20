# Runtime v2: Compiler-Owned Values and a Stateless Runtime Library

Status: **approved design**. Implementation starts with phase 0; nothing else is implemented yet.

## 1. Goal

ExS is still several times more expensive than equivalent Rust for value-heavy code. Recent
fuel work removed most of the overhead inside generated code; what remains is dominated by the
boundary between generated code and `exs-runtime`. Runtime v2 removes that boundary for values:

- **All values and all mutable state belong to the compiled module.** Heap objects, the
  allocator, async frames, the scheduler state and the error-trace stack live in linear memory
  with a layout defined once and shared by compiler, runtime library and runner.
- **The runtime becomes a stateless library.** It keeps only functions that would be hard to
  re-implement as generated Wasm (garbage collection, scheduling decisions, UTF-8 and number
  formatting, sorting, wire encoding, DateTime parsing). It owns no `static` state and no
  allocator; every function receives pointers into memory owned by the module.
- **Generated code reads and writes values directly.** Field loads, List indexing, String
  lengths, allocation fast paths and frame switches become inline Wasm instead of runtime calls.

Decisions already taken:

| Question | Decision |
|---|---|
| Values in the runtime? | No. The runtime holds no values and no state. |
| Host boundary (`ExsValue`, CBOR, runner API) | May change where that helps. |
| Migration | Parallel rebuild; the current backend stays until v2 passes every test. |
| Metrics | Fuel **and** wall-clock time. |

## 2. Where the cost is today

Approximate fuel per loop iteration from isolated micro benchmarks (loop overhead included),
measured during the optimization series up to `212ba60`:

| Operation | Fuel | Dominant cost |
|---|---:|---|
| Property read `o.c` | ~230 | value-table lookups and checks in `get_literal` (~145) |
| `text + "-"` | ~430 | `malloc` + value-table slot (~185), copy, dispatch |
| `{ a, b, c }` literal | ~926 | one runtime call per field (~117 each) plus allocation |
| `x.to_string()` | ~630 | allocation, table slot, dispatch |
| Resumable call | ~925 | frame allocation and completion in the runtime (~600) |
| Sync call, two arguments | ~227 | root frame, trace entry, contract checks |

Every heap access today goes `ValueRef` → table index → `Vec<Option<HeapSlot>>` → `RtValue`
enum → `Box` → payload, with bounds, option and variant checks on each step. Every heap value
needs a `dlmalloc` allocation plus a table slot. Those costs are structural; they disappear
only when generated code can address heap objects directly.

Fuel counts executed Wasm operators wherever they run, so moving code from the runtime into
generated code saves nothing by itself. The savings come from the **representation**: direct
addresses, fixed layouts, bump allocation and no per-call conversions.

## 3. Core principle: one layout, many readers

```
            +---------------------------------------------+
            |        exs-layout (shared Rust crate)       |
            |  #[repr(C)] headers, offsets, kind codes    |
            +-----------+------------------+--------------+
                        |                  |
       compiler emits   |   runtime lib    |   runner reads/writes
       inline loads and |   (stateless)    |   guest memory through
       stores by offset |   reads/writes   |   validated accessors
                        v   via pointers   v
            +---------------------------------------------+
            |        linear memory of the module          |
            |  VM state block | heap pages | root stack   |
            +---------------------------------------------+
```

- `exs-layout` replaces the value-table parts of `exs-value` and the register offsets in
  `exs-abi`. It defines every header and struct as `#[repr(C)]` with compile-time asserted
  offsets, so the compiler, the runtime library and the runner cannot disagree.
- The **VM state block** sits at a linker-assigned address (like `__EXS_RT_REGISTERS` today)
  and contains: heap allocator state, root-stack registers, trace-stack registers, scheduler
  state (task table, run queue, current task and frame), host-call state and literal tables.
- **Stateless** means: no `static mut`, no global allocator state inside the runtime crate.
  A function such as `gc_collect(vm: *mut VmState)` may freely mutate the memory it is given;
  it just never keeps anything for itself. Temporary Rust allocations (formatting buffers,
  CBOR) go through a `#[global_allocator]` that allocates from the module heap via the VM
  state block, so there is exactly one heap owner.

## 4. Value representation

`ValueRef` stays a 64-bit tagged `i64` with the current kind / extra / payload split, so all
existing inline immediate code keeps working:

- Immediates unchanged: None, Bool, Int in `i32` range.
- For heap kinds the 32-bit payload is the **linear-memory address** of the object header
  instead of a table index. The kind and extra bits keep their meaning (type flags, inline
  nominal type id), so most type tests stay pure bit tests.

### 4.1 Common header (8 bytes, 8-byte aligned objects)

| Bits | Field |
|---|---|
| `u8` | object kind (String, Bytes, BigInt, Float, List, Buffer, Object, TypedObject, Enum, Error, Closure, Cell, Iterator, Frame, ...) |
| `u8` | GC bits (mark, static, pinned) |
| `u16` | size class or flags |
| `u32` | kind-specific word: nominal type id, shape id, variant index or length |

### 4.2 Object kinds (sketch)

| Kind | Layout after the header | Notes |
|---|---|---|
| String / Bytes | `len: u32`, `hash: u32`, inline bytes | immutable; one allocation; literal Strings are **static objects in a data segment** (no first-use allocation, never collected) |
| BigInt / Float | `i64` / `f64` | Ints outside `i32` and all Floats |
| List | `len`, `capacity`, `items: *Buffer` | elements are raw `ValueRef`s in a separate growable Buffer object |
| TypedObject | fixed `ValueRef` slots in declaration order, then `extra: *Dict` | field offset known at compile time from the nominal type; extra keys only through the overflow dictionary |
| Object (untyped) | `shape: *Shape`, `values: *Buffer` | hidden-class shapes (ordered key list + transition table) make property access cacheable |
| Enum | variant index in the header word, payload slots | `match` becomes a header load and a `br_table` |
| Error | kind, message, data, severity, origin, trace | still built by runtime helpers |
| Closure / Cell | function index, captured Cells / one `ValueRef` | |
| Frame | state, function id, caller frame, destination slot, slots | async frames are ordinary GC objects |

## 5. Memory management

- **Allocator:** segregated size classes with bump allocation inside pages, plus a large-object
  space. The fast path is inline Wasm (`load top; add size; compare limit; store top`), the
  slow path calls the stateless runtime (`heap_refill(vm, size, kind)`), which may sweep a page
  or start a collection.
- **Collector:** stop-the-world, **non-moving** mark-and-sweep, implemented in Rust over the
  shared layout. Non-moving is deliberate: generated code keeps raw addresses in Wasm locals,
  and a moving collector would force every allocation site to reload all locals from root
  slots. A generational sticky-mark-bit scheme can follow later without changing the layout.
- **Roots:** the existing shadow root frames (already precise and inline), the scheduler's task
  table and frames, pending host results, and the VM state block. Static literal objects are
  never collected.
- **Accounting:** collection work stays inside the module and therefore inside fuel, as today.

## 6. What generated code can then do directly

These are the improvements this design exists for; each one is a later, measurable step:

1. Typed field read/write: one tag test plus one `i64.load`/`i64.store` at a constant offset.
2. Untyped property access with inline caches keyed by shape id.
3. String length, byte access, equality fast path (length + hash) and **single-allocation
   concatenation chains** (`a + "-" + b + ":"` computes the total length, allocates once).
4. List `len`, `get`, `set`, `push` with inline bounds checks; growth through the runtime.
5. Inline allocation for all small objects (literals, enum values, boxed Ints/Floats).
6. `match` on enums through the variant index without runtime calls.
7. Async calls: frame allocation is an inline bump allocation, the frame switch is a few
   stores into the VM state block, and the direct child step call from `212ba60` stays.
8. Later: type specialization from annotations, escape analysis for short-lived objects.

## 7. Execution model and async

- Scheduler state (tasks, run queue, quantum, parallel groups, host-call ids) moves into the
  VM state block. Generated code handles the hot paths inline (checkpoint countdown, frame
  switch, direct child calls); task selection, parallel joins and cancellation stay as
  stateless runtime functions operating on that state.
- The error-trace stack and the root stack stay register-driven as today, now stored in the
  VM state block.

## 8. Host boundary

Because the boundary may change, the guest-side CBOR encoding can go away:

- **Wasmtime runner:** reads results and writes inputs directly in guest memory through
  validated `exs-layout` accessors (bounds, alignment, kind, cycle and depth checks; guest
  memory is always treated as untrusted). Inputs are allocated through an exported
  `__exs_alloc` so the guest heap stays the single owner.
- **Host calls:** the same direct exchange for arguments and results.
- **Browser runner and `exs-guest`:** both implement the same boundary today. Either they get a
  JS / Rust implementation of the direct layout, or a stateless runtime pair
  `encode_cbor` / `decode_cbor` remains as a portable fallback. To be decided in phase 6.

## 9. Compiler structure

- Shared front end unchanged: parser, HIR, suspension analysis, type registry, diagnostics.
- New value-operation layer in codegen that emits v2 sequences; selected by
  `CompileOptions::backend` (`V1` default until switch-over) so both backends coexist.
- New runtime template crate (`exs-core`, built like `exs-runtime.wasm`) linked the same way.

## 10. Verification and migration

- The whole `exs-runner` integration suite runs against both backends (backend selected by the
  test support helper), plus differential tests that compare v1 and v2 results on the same
  programs, including errors and traces.
- Fuel scenarios and the Rust baseline comparison run for both backends.
- **New:** wall-clock benchmarks for the same scenarios, under Winch and Cranelift, since wall
  time now counts. (A benchmark harness dependency would need approval; a plain
  `Instant`-based bench binary needs none.)

## 11. Phases

Each phase ends with measurement and an explicit go / no-go.

| Phase | Scope | Exit criterion |
|---|---|---|
| 0 | Baseline: wall-clock benchmarks; measure the share of CBOR boundary work in `record_roundtrip_20` | numbers recorded |
| 1 | `exs-layout`, VM state block, allocator, non-moving GC in the stateless library, unit tests with synthetic heaps | GC and allocator tests green |
| 2 | v2 backend for Int/Float/Bool/None, Strings, Lists, untyped and typed Objects, sync functions, `?`, Errors | prototype beats v1 clearly on property, String and literal micro benchmarks; go / no-go |
| 3 | All built-in methods as stateless helpers over the layout; enums and `match`; closures and cells; iterators; formatted strings | all non-async runtime-value tests green on v2 |
| 4 | Async: frames as heap objects, scheduler state in memory, host calls, parallel groups, cancellation | full continuation and host-call suites green on v2 |
| 5 | Host boundary v2 for the Wasmtime runner | runner tests green, boundary cost measured |
| 6 | Browser runner and `exs-guest` on the new boundary (or CBOR fallback) | all crates green |
| 7 | Switch the default to v2, delete v1 code and `exs-runtime` | one backend left |

## 12. Risks and open questions

- **GC correctness:** every value that survives an allocation must be reachable from a root.
  The precise root frames exist, but every inline allocation site and every helper that
  allocates must keep that discipline. Mitigation: a debug mode that collects on every
  allocation, run against the full test suite.
- **Fragmentation** of the non-moving heap for long-running tasks; measured in phase 2.
- **Wall-clock vs fuel:** Winch generates slower machine code than Cranelift; whether
  production should switch compilers is a separate decision informed by phase 0 numbers.
- **Browser runner** may need its own implementation of the layout readers.
- **Scope:** this replaces most of `exs-runtime` (≈9,300 lines) and large parts of the
  value-related codegen; it spans many sessions.

Decisions taken for phase 1:

1. The garbage collector stays in the stateless Rust library and operates on the shared layout.
2. Untyped Objects use hidden-class shapes.
3. `ValueRef` stays 64 bits wide, keeping every immediate fast path.
