# Runtime v2: Compiler-Owned Values and a Stateless Runtime Library

Status: **approved design**. Phases 0, 0.5, 1 and 2 (2a to 2d, plus lazy class-page
linking) are done and phase 2 got its go; phase 3 is done except programs with more than
65,534 nominal types (section 12r); phase 4 is done except one test under a 2 MiB memory
limit (section 12w). See sections 12 to 12w.

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
| TypedObject | fixed `ValueRef` slots in declaration order, `extra: *Dict` and `order: *[u32]` (entry order, 0 = declaration order) | field offset known at compile time from the nominal type; extra keys only through the overflow dictionary |
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
| 0 | Baseline: wall-clock benchmarks; measure the share of CBOR boundary work in `record_roundtrip_20` | done, see section 12 |
| 0.5 | Link only reachable runtime and prelude functions; cache compiled modules in the runner | done, see section 12 |
| 1 | `exs-layout`, VM state block, allocator, non-moving GC in the stateless library, unit tests with synthetic heaps | done, see section 12a |
| 2 | v2 backend for Int/Float/Bool/None, Strings, Lists, untyped and typed Objects, sync functions, `?`, Errors | prototype beats v1 clearly on property, String and literal micro benchmarks; go / no-go |
| 3 | All built-in methods as stateless helpers over the layout; enums and `match`; closures and cells; iterators; formatted strings | all non-async runtime-value tests green on v2 |
| 4 | Async: frames as heap objects, scheduler state in memory, host calls, parallel groups, cancellation | done, see sections 12t to 12w |
| 5 | Host boundary v2 for the Wasmtime runner | runner tests green, boundary cost measured |
| 6 | Browser runner and `exs-guest` on the new boundary (or CBOR fallback) | all crates green |
| 7 | Switch the default to v2, delete v1 code and `exs-runtime` | one backend left |

### Later work outside the phases

- **Template diet.** The template's initialized data (about 15.4 KB after phase 3) leaves a
  program less than 1 KB of static data before its span starts a second 16 KiB block, which
  costs 16,384 fuel on every execution (sections 12j, 12q and 12r; `for_range` and
  `format_loop` already pay it). The template's data is mostly about 400 small panic
  locations and panic or formatting texts, plus `core::fmt`'s float table `CACHED_POW10`
  (1.3 KB), a libm square-root table and the Unicode whitespace table. Levers: panic-free
  indexing (`get` instead of `[]`, no `unwrap`), an own float formatter like the table-free
  `Float::parse`, replacing the small tables, and moving the uninitialized VM state block out
  of the initialized span (a memory layout change). Goal: typical programs stay in one block
  with margin. To be planned and approved separately.
- **More than 65,534 nominal types.** Inline type-tag tests need a heap type-id fallback
  (section 12r).
- **Initial memory.** v2 needs more than 2 MiB before a program runs: the template's 1 MiB
  Rust stack, a 1 MiB root stack and the trace and heap pages (section 12v). Smaller stacks
  or growing them on demand are a memory layout change.
- **Resumable loop overhead.** A loop in a resumable function loads and stores frame slots
  for every operation, so `async_int_while_1000` is only 3 % below v1 (section 12w); keeping
  values of one state machine in locals between suspension points is the lever.

## 12. Phase 0 results

Measured with `examples/fuel-baseline/compare.nu` (median of 7 runs) and a per-function fuel
profiler on the state after `12fbfb4`.

### Wall-clock time is dominated by module compilation

| Scenario | ExS exec Winch | Rust exec Winch | ExS exec Cranelift | Rust exec Cranelift |
|---|---:|---:|---:|---:|
| `empty_main` | 86 µs | 59 µs | 67 µs | 47 µs |
| `int_loop_1000` | 134 µs | 71 µs | 75 µs | 50 µs |
| `list_churn_6000` | 2,793 µs | 523 µs | 898 µs | 195 µs |
| `datetime_roundtrip_100` | 2,104 µs | 355 µs | 731 µs | 151 µs |
| `record_roundtrip_20` | 1,932 µs | 811 µs | 773 µs | 410 µs |

| Module compilation + instantiation | Winch | Cranelift |
|---|---:|---:|
| ExS module (any script) | ~19 ms | ~110 ms |
| Rust guest (all 18 scenarios) | ~4.4 ms | ~17 ms |

Every execution compiles its module, so compilation is 10–200× the execution time of these
scenarios. An empty ExS script already links a 668 KB module with 684 functions (509 KB of
code): the whole runtime template (369 KB) plus every compiled prelude function, whether used
or not. The Rust guest with all 18 scenarios is 101 KB.

Consequences for the plan:

- **Link only reachable functions.** Dead-function elimination over the runtime library and
  the prelude cuts compile time for every script; the stateless library of v2 makes this
  easier, but it pays off in v1 already.
- **Cache compiled modules in the runner.** Compiling once per script and instantiating per
  execution (Wasmtime `Module` reuse or serialized artifacts) removes most of the remaining
  wall-clock cost and makes Cranelift's faster code affordable.
- Both items are independent of the value model and are scheduled before phase 1.

### Fuel: memory management and the host boundary dominate value-heavy scripts

Share of executed Wasm operators by category:

| Category | `record_roundtrip_20` | `datetime_roundtrip_100` |
|---|---:|---:|
| Allocation and freeing (`dlmalloc`, value table, `Vec` growth) | 21.8 % | 34.7 % |
| Garbage collection | 0 % | 10.5 % |
| CBOR decode/encode and `ExsValue` conversion | 21.1 % | 1.8 % |
| Typed decode schema parsing (`wire`) | 8.9 % | 1.4 % |
| Object and property operations | 13.2 % | 8.0 % |
| Value-table lookups | 7.3 % | 4.3 % |
| UTF-8 validation | 4.7 % | 0.7 % |
| Strings and formatting | 3.9 % | 9.0 % |
| Generated code | 7.5 % | 14.2 % |
| Other runtime | 11.5 % | 15.5 % |

- Memory management alone is 22–45 %: the bump allocator and the non-moving collector of
  phase 1 target exactly this.
- In record-style scripts, about 30 % is boundary work (CBOR, `ExsValue` conversion, schema
  parsing). The direct-memory boundary of phase 5 removes it; the typed decode schema is
  re-parsed on every conversion today and could be cached even in v1.

### Phase 0.5 results

- **Pruning:** the compiler now drops the runtime template's `__exs_rt_*` link exports and
  every function unreachable from runner-visible exports, the start function, element segments
  and `ref.func`. The resumable dispatcher's calls of step functions are weak; suspendable
  entry points and lifted closures are roots, because only their frames are created by
  function id, and every child call calls its step directly.
- **Module cache:** `ServerRunner` keeps the compiled artifacts of the 16 most recently
  executed modules (`with_module_cache_capacity`, 0 disables it). Each execution still gets its
  own engine, so deadline and cancellation epochs stay isolated; a cache hit only loads the
  artifact. Fuel is unchanged.

| Module | Size before → after | First compile Winch | First compile Cranelift |
|---|---:|---:|---:|
| empty script | 668 KB → 226 KB | 21 ms → 6 ms | 110 ms → 20 ms |
| `datetime_roundtrip_100` | 553 KB → 417 KB | 13.5 ms → 9.3 ms | 92 ms → 50 ms |
| `record_roundtrip_20` | 601 KB → 466 KB | → 10.9 ms | → 59 ms |

With the cache, start-up per execution (artifact load and instantiation) drops to 0.2–0.5 ms
for every scenario under both compilers, so Cranelift's faster code no longer costs start-up
time after the first execution of a module.

## 12a. Phase 1 results

- **`exs-layout`** defines the 8-byte object header, the fixed part of every object kind
  (String/Bytes, BigInt, Float, Buffer, List, TypedObject, Object, Shape, Enum, Cell, Closure,
  Error, Frame, free cells), 64 KiB pages with a 16-byte page header, 15 size classes from 16 to
  2,048 bytes, the VM state block (heap state and root ranges) and, per kind, which fields are
  `ValueRef` slots and which are `u32` object addresses. Offsets and alignment are checked at
  compile time.
- **`exs-core`** is `no_std`, has no state of its own and is generic over a `Memory` trait, with
  a direct `LinearMemory` implementation for Wasm. It provides `init_heap`, `allocate` (free list
  per size class, first-fit page runs, memory growth only when nothing is free) and `collect`
  (non-moving mark and sweep from the root ranges; empty class pages and dead large objects
  return to merged free page runs; a mark-stack overflow falls back to rescanning marked
  objects; an option collects before every allocation to test root discipline).
- Native tests run the same code on a byte-vector memory: object headers and zeroing, large
  objects, exhaustion, reachability through Lists, Buffers and Cells, cycles, immediates and
  static objects, page and run reuse, random graphs with and without collection at every
  allocation, mark-stack growth and overflow.

## 12b. Phase 2a results

- **`exs-core-template`** (17 KB) exports the stateless core functions as `__exs_core_*`. A
  global allocator that refuses every request makes any accidental Rust allocation trap.
- **VM placement:** the v2 linker puts the VM state block right after the template's
  `__heap_base` and the heap after it; `__exs_input_alloc` initializes the heap and a 16-page
  root stack on first use.
- **Backend selection:** `compile_with_backend(source, options, Backend::V2)`. `CompileOptions`
  stays unchanged, because Exolynk constructs it field by field.
- **`codegen/v2`** compiles the functions reachable from public entries: None, Bool, Int (with
  heap Ints outside `i32`), Float, `+ - * /`, comparisons, `== !=`, `! && ||`, `let`,
  assignment, `ret`, blocks, `if`, `while`, `for` over literal Ranges of immediate Ints,
  `break`/`continue`, direct calls and built-in type contracts. Every other construct is
  compile error E0600. Language Errors do not exist yet, so type errors, overflow, contract
  violations and excess arguments trap.
- `backend_v2` tests run programs with both backends and require identical results, including
  a run that collects repeatedly.

| 1000 iterations | v1 fuel | v2 fuel | change |
|---|---:|---:|---:|
| `while` Int loop | 176,620 | 147,598 | −16 % |
| `for` over a Range | 143,658 | 90,603 | −37 % |
| sync calls | 292,614 | 195,530 | −33 % |
| Float loop | 579,772 | 610,040 | +5 % |

The Float loop still calls the Rust allocator for every Float; the inline allocation fast path
is part of the next steps.

## 12c. Phase 2b results

- **Static literals:** every String literal is a finished `GC_STATIC` String object in a data
  segment between the VM state block and the heap; a literal is one `i64.const`.
- **String layout:** the header word holds the character count, so a future `length()` is one
  load.
- **`+` chains and formatted strings:** a chain whose leftmost operand is a String is joined
  in one `__exs_core_concat` call with exactly one allocation; other chains add pairwise with
  inline Int and Float paths. Scalars render like v1 (Ints with a dedicated digit writer,
  Floats with `core::fmt`). Formatted strings with String and scalar parts use the same call.
- **Equality and ordering** of Strings (text, byte-wise order) and **CBOR** text strings with
  UTF-8 validation.
- **Inline allocation:** boxed Ints and Floats take a 16-byte cell straight from the free list;
  two Floats add, subtract, multiply and divide inline.

| 1000 iterations | v1 fuel | v2 fuel | change |
|---|---:|---:|---:|
| `while` Int loop | 176,620 | 147,633 | −16 % |
| `for` over a Range | 143,658 | 90,638 | −37 % |
| sync calls | 292,614 | 195,569 | −33 % |
| Float loop | 579,772 | 318,108 | −45 % |
| formatted string per row | 2,006,248 | 1,751,916 | −13 % |
| `+` chain per row | 2,439,203 | 1,788,916 | −27 % |

String building still copies byte by byte through the core library; writing whole words and
rendering into the destination directly are possible follow-ups.

## 12d. Phase 2c-1 results

- **Lists** are a List object over one Buffer whose slot count is the capacity. Literals are
  one `__exs_core_list_new` call over consecutive root slots. `list[i]` and `list[i] = v` run
  inline when the receiver is a List and the index an immediate Int inside the bounds (one
  unsigned comparison also rejects negative indices); every other case goes through
  `__exs_core_index_get`/`_set`. `List + value` builds a new List like v1 (a List operand is
  appended element by element) through the `+` path of `__exs_core_concat`. `for x in list`
  iterates over a copy taken at loop start, matching v1's snapshot.
- **Typed Objects** keep their declared fields in slots in declaration order and carry
  `type_id + 1` in the reference's extra bits. A literal evaluates its properties in source
  order, checks each field contract and stores None for fields without a property.
  `obj.field` and `obj.field = v` test the receiver's tag against every type declaring the
  field and access that type's slot inline; other receivers and undeclared fields go through
  `__exs_core_property_get`/`_set` (a missing field reads as None). Nominal type contracts are
  inline tag tests.
- **Type table:** the linker places a static table of field-name Strings per type id after the
  String literals; `VmState::program` points to it, so the core can name fields.
- **Temporary roots:** core functions that allocate twice root intermediate objects on the
  root stack and pop them before returning.
- **CBOR** decodes nested arrays into Lists and encodes Lists and typed Objects (maps of their
  declared fields in declaration order), rejecting nesting deeper than 128 and therefore
  cycles.
- Still trapping instead of returning Errors until 2d: out-of-bounds and non-Int indices,
  undeclared or repeated properties in literals, contract violations, `for` over non-Lists,
  and writes of undeclared keys.

| 1000 iterations | v1 fuel | v2 fuel | change |
|---|---:|---:|---:|
| property read and write | 731,746 | 295,451 | −60 % |
| typed Object and List literal per row | 1,952,885 | 927,798 | −52 % |
| List index read and write | 692,015 | 428,600 | −38 % |
| `for` over a 100-element List | 449,855 | 371,225 | −17 % |

**Difference from v1:** v1 stores a typed Object's fields in literal order followed by the
missing fields, so its CBOR map uses that order; v2 always uses declaration order. Both agree
whenever a literal names its fields in declaration order. (Resolved in section 12n.)

## 12e. Phase 2c-2 results

- **Shapes** form a tree from the VM's empty root shape (`VmState::program.root_shape`); each
  child extends its parent by one key, and the children of a shape are linked through
  `first_child`/`next_sibling`. Every shape holds its full key Buffer. The collector treats the
  root shape as a root, so shapes and their keys are never collected.
- **Untyped Objects** are a shape plus a value Buffer that doubles when a key is added.
  Literals evaluate their values left to right into consecutive root slots; a repeated key
  keeps the position of its first occurrence and the value of its last, like v1. Each literal
  site computes its shape once from a static key array and caches it in a Wasm global.
- **Inline caches:** every property access site owns two mutable `i32` globals, the last seen
  shape and the key's value position. After the typed-Object tag chain, an untyped receiver of
  the cached shape reads or writes its slot directly; otherwise `__exs_core_object_lookup`
  refills the cache and the core does the access. Writes only use the cache for existing keys;
  adding a key always goes through the core.
- **Typed Objects** keep keys their type does not declare in an untyped extra Object; they
  read after the declared fields and encode after them, like v1.
- **CBOR** decodes maps into untyped Objects (text keys only, repeated keys rejected) and
  encodes untyped Objects as maps in key order.

| | v1 fuel | v2 fuel | change |
|---|---:|---:|---:|
| property read and write, 1000 iterations | 731,720 | 478,150 | −35 % |
| Object literal per row, 1000 iterations | 1,167,940 | 887,774 | −24 % |
| `concat_10_objects` | 136,378 | 352,451 | +158 % |
| `concat_100_objects` | 664,705 | 860,076 | +29 % |

The Object scenarios with runner input lost because of a fixed start-up cost, not per Object:
decoding an empty List argument cost about 147,000 fuel in v2 against 70,000 in v1, while every
further Object cost less in v2 (about 3,800 against 4,600). The allocator linked all cells of a
fresh class page into the free list at once, e.g. 4,080 cells for the 16-byte class. Section
12f removes this cost.

## 12f. Lazy class-page linking

- A refill links at most 64 cells of the class's current page into its free list. Each size
  class keeps a cursor (`HeapState::class_cursors`) at its first never-linked cell, and the next
  refill continues there until the page is used up.
- Cells from the cursor on never held an object and may hold stale bytes of a reused page, so
  sweeping and the rescan after a mark-stack overflow only look at the cells before it. A
  cursor page that sweeps empty returns to the free runs and its cursor is cleared.
- The inline allocation fast path of boxed numbers is unchanged; it still pops the free list.

| | v1 fuel | v2 before | v2 after | v2 vs v1 |
|---|---:|---:|---:|---:|
| `concat_10_objects` | 136,378 | 352,451 | 81,864 | −40 % |
| `concat_100_objects` | 664,705 | 860,076 | 578,633 | −13 % |
| untyped property read and write, 1000 iterations | 731,720 | 478,150 | 290,696 | −60 % |
| untyped Object literal per row, 1000 iterations | 1,167,940 | 887,774 | 705,493 | −40 % |
| typed property read and write, 1000 iterations | 731,746 | 295,526 | 249,095 | −66 % |
| typed Object and List literal per row, 1000 iterations | 1,952,885 | 927,873 | 833,879 | −57 % |
| List index read and write, 1000 iterations | 692,015 | 428,675 | 319,757 | −54 % |
| `for` over a 100-element List | 449,855 | 371,718 | 186,953 | −58 % |
| Float loop, 1000 iterations | 579,772 | 318,437 | 280,825 | −52 % |

## 12g. Phase 2d results

- **Errors are values,** as in v1. Every core operation that can fail returns v1's Error
  instead of trapping: TypeError, IntOverflowError, IndexError, MissingValue, NotIterable,
  WireDecodeError and the fatal ArityError, with v1's kinds, messages and data. Generated
  code returns early exactly where v1 does: for `?`, for a condition that is no Bool, for a
  declared-type violation (recoverable when the return type admits Errors, fatal otherwise)
  and after a direct call that produced a fatal Error. `Error(kind, message, data)` is
  supported.
- **Origins:** before every core call that may create an Error, generated code stores the
  source position v1 uses (`VmState::errors.source_position`), only on the slow paths. A `+`
  chain passes a static array with one position per step, so a failing step deep inside a
  fused chain reports its own `+`, and later steps add to that Error like v1.
- **Traces:** each function pushes `function_id | call_site << 32` on a fixed two-page trace
  stack on entry and restores it before returning; a call stores its call site first. Function
  ids and source positions come from v1's numbering, and v2 modules carry the same
  `exs.source.map` section.
- **Layout:** `ErrorObject` gained a `cause` slot (None until Error methods exist); Errors
  encode as the codec's tagged seven-field map.
- **Let annotations** follow v1's wire decoding for built-in types: an Error returns early and
  a value of another type returns a WireDecodeError. Nominal, enum and List element types in
  `let` are compile error E0600 until wire decoding is ported.
- `backend_v2` compares complete Error values, including origin and trace, for 36 programs
  and runs a loop that keeps Errors alive across collections.

Still trapping, because v1 needs features of later phases: ordering comparisons without an
order (v1's TypeError carries an `std::Ordering` enum value), formatted-string parts that are
not Strings or scalars (v1 renders them with the default `ToString`), and `for` over Strings,
Bytes and Objects (Iterators). Errors as runner inputs remain unsupported until phase 5.

The trace costs about 40 fuel per direct call: sync calls rose from 196,001 to 236,177 per
1000 calls, still 19 % below v1.

### Phase 2 go / no-go summary

| Workload | v1 fuel | v2 fuel | change |
|---|---:|---:|---:|
| `while` Int loop, 1000 iterations | 176,620 | 147,254 | −17 % |
| `for` over a Range, 1000 iterations | 143,658 | 91,260 | −36 % |
| sync calls, 1000 calls | 292,614 | 236,177 | −19 % |
| Float loop, 1000 iterations | 579,772 | 280,001 | −52 % |
| formatted string per row, 1000 rows | 2,006,248 | 1,747,893 | −13 % |
| `+` chain per row, 1000 rows | 2,439,203 | 1,792,893 | −26 % |
| typed property read and write, 1000 iterations | 731,746 | 248,271 | −66 % |
| typed Object and List literal per row, 1000 rows | 1,952,885 | 833,055 | −57 % |
| untyped property read and write, 1000 iterations | 731,720 | 289,947 | −60 % |
| untyped Object literal per row, 1000 rows | 1,167,940 | 704,693 | −40 % |
| List index read and write, 1000 iterations | 692,015 | 317,933 | −54 % |
| `for` over a 100-element List | 449,855 | 188,159 | −58 % |
| `concat_10_objects` | 136,378 | 82,213 | −40 % |
| `concat_100_objects` | 664,705 | 580,242 | −13 % |

Every measured workload is cheaper in v2, with the largest gains on property access, literals
and Lists, which meets phase 2's exit criterion. String building gains least, because the
core still copies byte by byte.

## 12h. Phase 3a results

Phase 3 is split into 3a (measurement and formatted strings), 3b (built-in methods and Bytes),
3c (enums, `match`, `is`, `std::Ordering` and wire decoding of `let` annotations), 3d
(implementation methods, traits and operator traits, `ToString`, `Range` values), 3e (closures,
Cells and variadic parameters) and 3f (Iterators).

- **Test switch:** `EXS_TEST_BACKEND=v2` makes the runner's shared test support compile with
  the v2 backend, so every existing suite measures v2 unchanged. E0600 now names the
  unsupported expression kind.
- **Formatted strings** render like v1: Strings and scalars as with `+`, every other value
  with v1's default text (`None`, `[]`, `{}`, `Error`), through one `__exs_core_format` call
  that never fails.
- **Protocol guards:** v1 dispatches operators to nominal `Add`, `Sub`, `Mul`, `Div` and
  `Compare` implementations and formatted-string parts to `ToString` implementations. v2 does
  not compile implementation methods yet and silently applied the built-in behavior; it now
  traps on exactly those receivers until 3d.

Suites on v2 (v1 passes all of them):

| Suite | v2 passed | v2 failed |
|---|---:|---:|
| `runtime_values` | 26 | 70 |
| `protocols_and_nominal` | 3 | 24 |
| `contracts_and_errors` | 8 | 7 |
| `closures_parallel` | 1 | 16 |
| `limits` | 9 | 2 |
| `fuel` | 8 | 10 |
| `continuations` (phase 4) | 0 | 18 |
| `host_calls` (phases 4 and 5) | 2 | 29 |

No failing test produces a different value than v1; each one is a compile error or a trap.
Compile errors across the non-async suites, by cause:

| Unsupported | Tests | Phase |
|---|---:|---|
| method calls | 41 | 3b, 3d |
| enum and List element type contracts | 18 | 3c |
| static method calls | 15 | 3b, 3d |
| closures | 10 | 3e |
| Host operations | 9 | 4, 5 |
| parallel expressions | 7 | 4 |
| `is` type tests | 5 | 3c |
| enum values, `match` | 7 | 3c |
| calls of prelude functions | 4 | 3d |
| calls of local bindings, variadic parameters | 3 | 3e |
| Bytes literals | 1 | 3b |
| more than 65,534 nominal types | 1 | open |

The traps are Bytes and Error runner inputs (phase 5 and 3b), the guarded trait
implementations (3d) and literal Range bounds outside `i32` (3f).

## 12i. Phase 3b-1 results

- **Method calls** on built-in values evaluate receiver and arguments and call one core
  dispatcher, `__exs_core_call_builtin`, with the `exs_abi::BuiltinMethod` index. Number
  methods, `add`/`sub`/`mul`/`div`, `div_euclid`/`rem_euclid`, `to_string`/`debug`,
  `length`/`is_empty` and the Error methods `kind`/`message`/`data`/`cause` return v1's
  results and Errors. Like v1, an unknown method name is a MethodNotFound Error and a built-in
  called with another number of arguments an ArityError; the compiler decides both from the
  static name.
- **Static functions** of built-in types (`Int::parse`, `Float::parse`, every `Math`
  function) go through `__exs_core_call_static` with an `exs_layout::StaticFunction` index.
  Floating-point functions use `libm`, as v1 does, for bit-identical results.
- A receiver whose type implements a method of the same name traps until implementations are
  compiled (phase 3d), like the operator guards of 3a.
- `runtime_values` on v2: 31 of 96 (was 26). The leading blockers are `push` and the other
  List and Object methods (3b-2) and `List<T>` and enum contracts (3c).

## 12j. Fuel accounting of data and bulk memory

Measured with Wasmtime 47 directly: `memory.fill` and `memory.copy` consume one fuel per byte,
and initializing data segments at instantiation consumes one fuel per byte in blocks of 16 KiB
(16,384 fuel per started block). The core template's data segment is initialized on every
execution, and pruning removes unused functions but not their data, so every byte of template
data costs every script.

- 3b-1 added `f64::from_str`, whose `POWER_OF_FIVE_128` table alone is 10,416 bytes. It pushed
  the template's data from 5,494 to 22,487 bytes, a second block, and every execution
  16,384 fuel more. `Float::parse` now uses a table-free exact parser: Clinger's fast path and
  otherwise one big-integer division with round-half-to-even on at most 800 significant digits.
  Native tests compare it bit for bit with `f64::from_str` on edge cases and 1.6 million
  random inputs, including rounding midpoints, subnormals, overflow and invalid syntax.
- **Data budget:** the template's data must stay below 16 KiB; it is 10,054 bytes now. Every
  new table needs a size check, and large tables (Unicode case mapping in 3b-2) need a compact
  form or must come from compiler-emitted data that only programs using them carry.
- 3a's formatted strings had lost the inlined rendering of `+` chains (a function pointer per
  part); joining is generic again and formatting takes the `+` rendering path when every part
  is a String or scalar.
- Allocation zeroes each object and every call zeroes its root frame with `memory.fill`, which
  also costs per byte; skipping zeroing for objects that are written completely is a later
  optimization.

| | 2d | 3b-1 before the fix | now |
|---|---:|---:|---:|
| `while` Int loop, 1000 iterations | 147,254 | 163,659 | 147,275 |
| formatted string per row, 1000 rows | 1,747,893 | 2,376,298 | 1,633,914 |
| `+` chain per row, 1000 rows | 1,792,893 | 2,041,298 | 1,792,914 |
| `concat_10_objects` | 82,213 | 99,998 | 82,254 |

## 12k. Phase 3b-2a results

- **List methods** (`push`, `pop`, `get`, `first`, `last`, `slice`, `extend`, `reverse`,
  `reversed`, `contains`, `index_of`, `last_index_of`, `join`, `insert`, `remove`, `clear`),
  **Object methods** (`has`, `get`, `get_or`, `delete`, `keys`, `values`, `entries`, `merge`,
  `clear`, `Object::from_entries`), **String methods** (`slice` by characters, `contains`,
  `starts_with`, `ends_with`, `repeat`, `trim`, `trim_start`, `trim_end`, `replace`, `split`)
  and deep **`clone`** return v1's results and Errors, with v1's order of checks (index
  arguments before receivers). Lists grow by doubling their Buffer; `delete` on an untyped
  Object rebuilds its shape from the remaining keys. `clone` keeps aliases and cycles through
  a hash table from source to copy in a rooted List.
- v1 lets `delete` and `clear` remove declared fields of typed Objects. v2 marks such a field
  with an empty slot: it reads as None and is no entry for `keys`, `length`, `entries` or CBOR.
  Setting it again traps, because v1 appends it after every other key, an order fixed slots
  cannot express. The inline typed-property paths check for the empty slot, which costs
  about 17 fuel per property access in the property benchmark (248,271 → 265,271, still −64 %
  against v1).
- **Data budget:** `str::find`'s searcher and String slicing reach `core`'s panic formatting,
  which adds Unicode grapheme tables and panic texts. The core searches bytes itself and
  slices with `get`; the template's data is 12,798 bytes, 3.6 KiB below the 16 KiB block.
- `runtime_values` on v2: 41 of 96 (was 31), still without a single differing result. The
  leading blockers are now `List<T>` and enum contracts (3c), static methods of nominal types
  and closures.

## 12l. Phase 3b-2b results

- **Bytes** are static objects for literals and String-layout heap objects otherwise. They
  compare by octets, render as `Bytes(n)`, index to Ints, travel as CBOR byte strings in both
  directions and support v1's methods (`slice`, `concat`, `contains`, `starts_with`,
  `ends_with`, `repeat`, `to_hex`, `to_base64`, `decode_utf8`, `length`, `is_empty`) and
  constructors (`Bytes::from_list`, `from_utf8`, `from_hex`, `from_base64`, and
  `encode_utf8` on Strings). `to_list` is a prelude Iterator helper in v1 and follows with
  phase 3f.
- **Data accounting, refined:** Wasmtime charges initialized memory as the span from the
  lowest to the highest initialized byte, in 16 KiB blocks, not the bytes of the segments. The
  template's data, the VM state block and a program's static data (String literals, type
  table, Object keys, positions) lie back to back and share one span. The template's data is
  14,422 bytes, so programs with more than about 2 KiB of static data start a second block.
- **Case mapping** (`to_lowercase`, `to_uppercase`) uses compact varint tables of 3,412 bytes
  in `exs-layout`, generated from the toolchain's standard library (Unicode 17.0) by
  `cargo test -p exs-layout -- --ignored generates_case_tables`. They map every code point and
  apply the final-sigma rule exactly like `str::to_lowercase`; tests compare all code points,
  the derived Case_Ignorable and Cased properties and random sigma texts against `std`, and
  fail when the toolchain's Unicode version changes. The compiler adds the tables to a
  program's static data only when a reachable function calls `to_lowercase` or `to_uppercase`;
  such programs pay about one more data block (a minimal script: 20,085 → 40,275 fuel).
- `runtime_values` on v2: 43 of 96, still without a single differing result.

## 12m. Phase 3c-1 results

- **Enums** are `EnumObject`s (type id, variant index, payload slots) whose references carry
  the enum's type tag like typed Objects. The type table's `TypeEntry` gained the enum's
  identity String; for enums `field_count` and `field_names` hold the variants in declaration
  order. Unit variants, payload constructors (as calls and static calls) check their payload
  contracts in source order like v1, then call one core function.
- **`match`** validates its arms with v1's `MatchArms` in source order. A variant arm tests
  the reference's tag inline and, only on a hit, the variant index; bindings read payload
  slots after a field-count guard, which binds v1's MatchError when the value lacks the field.
  Unmatched values return the MatchError at the `match`'s position.
- **Contracts and `is`** test built-in kinds by mask and nominal and enum types by tag. Like
  v1, `List<T>` only checks the List, not its elements; this alone made most runner tests
  compile on v2.
- **Ordering:** `compare` returns `std::Ordering` values (type 0), and `<`, `<=`, `>`, `>=`
  on values without an order return v1's TypeError with `Ordering::Unordered` as its data
  instead of trapping. Enums render as `identity::Variant`, clone with their payloads, and
  travel as CBOR tag 60005 in both directions. Inputs of identities or variants the program
  does not declare trap as unsupported until host enums arrive (phase 5).
- **Fuel:** unchanged within ±21 for scalar loops. The extra enum case of rendering costs about
  4 fuel per formatted String (format_loop 1,629,943 → 1,633,922) and about 30 per Object
  operand of `+` (concat_100_objects 581,404 → 584,585). The template's data is 14,542 bytes.
- `runtime_values` on v2: 57 of 96 (was 43). One newly compiling test exposes a difference
  from phase 2c: v1 lists the fields of a typed Object in assignment order
  (`P { z: 3, x: 1 }` has keys `z, x`), while v2 always uses declaration order, in `keys`,
  `entries` and CBOR. The other new failures are known traps: user `impl` and trait dispatch
  (3d), iterating non-Lists, Error inputs, and Range bounds outside `i32`.

## 12n. Field order of typed Objects

- A `TypedObject` carries `order`: 0 for declaration order, or the address of a static array
  of slot indices in entry order. Like v1, a literal orders its properties in source order,
  then the fields it omits in declaration order. The compiler stores an array only for
  literals whose order differs from the declaration, deduplicated among the static `u32`
  arrays after the `+` positions. Slots stay in declaration order, so inline property access
  is unchanged.
- `keys`, `values`, `entries`, `length`, `merge`, `get`, CBOR and `clone` follow the entry
  order. Assigning a field keeps its position, as in v1. Arrays are static only: Objects
  decoded from inputs (3c-2) will need a heap order, and setting a removed field again still
  traps until then.
- **Fuel:** the header grows by 8 bytes (`order` plus padding for 8-byte slots), which the
  allocator's zeroing charges per typed Object: the typed literal benchmark rises by 2,896
  (839,084 → 841,980). A reordered literal adds one `i32` store.
- `runtime_values` on v2: 58 of 96, with no differing results.
- Nothing needs a heap order after all: v1 never converts runner inputs to typed Objects
  (a typed parameter rejects an Object input), and wire decoding (12o) builds declaration
  order.

## 12o. Phase 3c-2 results

- **Wire decoding** of annotated `let` and `for` bindings follows v1's decoder exactly:
  Errors pass and return, `List<T>` builds a new List of decoded elements, an Object of the
  first nominal Object alternative passes while any other Object becomes one of its declared
  fields (missing ones None, extra keys dropped), an Object tagged `$variant` becomes that
  enum with its payload by name or from `$values`, and every other value is checked by mask,
  with v1's WireDecodeError carrying the innermost rejected value.
- **Schemas** are static record trees (`exs_layout::WireContract`, `WireObject`, `WireEnum`,
  `WireVariant`, `WireField`) built during code generation and shared by content among the
  static `u32` arrays, which are now 8-byte aligned. The core's `wire_decode` walks them.
  Contracts of built-in types keep the inline mask test, so plain annotated bindings cost
  nothing more. Annotations of recursive types are reported as unsupported (v1 overflows its
  stack building their schema string).
- **v1 fix (approved):** v1 rejected enum values for their own enum contract
  (`let c: Color = Color::Gray(1)`, `let o: Ordering = a.compare(b)`), because a real enum
  has no `$variant` key. Both backends now accept an enum of one of the contract's enum types
  unchanged. It costs v1's `record_roundtrip_20` 240 fuel (ceiling 4,255,458 → 4,255,698).
- Payload names of enums join the static Strings. The template's data is 14,662 bytes.
- `runtime_values` on v2: 58 of 96, unchanged; the remaining failures need phases 3d to 3f.

## 12p. Phase 3d results

- **Implementation methods** compile as v2 functions keyed `Type::method`, with v1's trace
  ids (functions first, then methods in declaration order) and v1's E0201 and E0224 checks.
  Reachability follows calls, static calls and, like v1's dispatch, every implementation of
  a called method name, of a used operator and, for formatted strings, of `ToString`.
- **Dispatch** is v1's static chain: the receiver's (or left operand's) type tag is tested
  against every implementing type in declaration order before the built-in behavior. Method
  calls, `.to_string()`, `.debug()` and `.compare()` use it, as do `+ - * /`, `==`, `!=`,
  `<`, `<=`, `>`, `>=` (a `Compare` result is tested by the core's `ordering_test` with v1's
  Errors) and formatted-string parts (`ToString`). Static methods use v1's diagnostics. A
  wrong argument count returns v1's "method received an incorrect number of arguments". The
  protocol guards of 3a and 3b are gone; a fallback the backend cannot compile yet traps only
  when implementations exist, else it stays a compile error.
- **Prelude:** `Range`, `Duration`, `DateTime` and other prelude code compile like user code.
  Its RFC 3339 parser intrinsic is lowered like v1: the core parses into four rooted slots and
  the compiler builds the raw-field Object.
- **Fuel:** `datetime_roundtrip_100` 5,597,206 → 3,457,701 (−38 %), `record_roundtrip_20`
  4,255,698 → 2,672,439 (−37 %). `format_loop` rises 1,633,922 → 1,660,306: like v1, every
  part now tests the prelude's `ToString` types, and the program's data starts a new 16 KiB
  block. Pruning dispatch targets to types a program can construct is a later optimization.
  The template's data is 15,110 bytes, 1.2 KiB below the block; the template diet is due.
- Suites on v2: `runtime_values` 62 of 96, `protocols_and_nominal` 20 of 27, `fuel` 14 of 18,
  all without differing results. The rest needs closures and variadics (3e), iterators (3f),
  host operations (4, 5) and Error inputs (5).

## 12q. Phase 3e results

- **Closures** are lifted like v1 (`$closure:N`, trace ids after the methods) into functions
  whose leading parameters are the captured Cells. A closure value is a `ClosureObject`
  (kind `Fn`) holding its dense index and Cells. Calls of a binding go through one generated
  dispatcher, a `br_table` over the reached closures, which checks the argument count (v1's
  ArityError), packs variadic arguments and calls the lifted function; a value that is no
  closure returns v1's "value is not callable". Closures run synchronously; phase 4 replaces
  the call mechanism with frames and keeps the object layout.
- **Cells:** like v1, every binding whose name some closure captures lives in a Cell
  (`let`, parameters, `for` and `match` bindings), so closures share mutations and each loop
  iteration binds a new Cell. Reads and writes are inline loads and stores; `clone` copies
  closures with independent Cells, keeping aliases.
- **Variadic parameters** pack the remaining arguments into a List at call sites (functions,
  methods, static methods, closures and entries) and check each element against the
  parameter's contract. v1 reports parameter checks of functions it lowers as resumable at the
  declaration instead of the parameter, so v2 reuses v1's suspendability analysis for that
  origin. The prelude's iterator helpers (`map`, `filter`, `find`, `any`, `all`, `each`,
  `reduce`) dispatch like v1 after implementations and before built-in methods.
- **Runner boundary:** a result with a closure or a cycle becomes v1's fatal
  SerializationError (cycle versus unsupported value, found by a depth-first walk).
- **Data budget:** the template's data grew to 15,388 bytes, and template data, VM block and
  a program's statics crossed into a second 16 KiB block for every program (+16,384 fuel).
  The type table now names the prelude's types only when reached code uses them (all user
  types stay), which keeps small programs in one block (`int_loop` 147,289). The template diet
  is overdue: about 400 small panic and formatting texts make up most of the template's data.
- Suites on v2: `runtime_values` 77 of 96, `contracts_and_errors` 11 of 15, `host_calls` 10 of
  31, `closures_parallel` 7 of 17, still without a differing result.

## 12r. Phase 3f results and the end of phase 3

- **Iteration:** the core's `for` snapshot copies Lists, turns Strings into a List of their
  Unicode scalars and Bytes into a List of Ints (both never change, so this yields what v1's
  lazy iterators yield), keeps Objects and returns v1's NotIterable for other values. Every
  loop has one inline fast path, the List index or the Range counter, and only once that is
  exhausted advances an Object through the `Iterator::next` dispatch and its `IteratorStep`
  with v1's order of Errors (an Error returns, `Done` ends, `Item` binds, anything else is a
  MatchError, an Object without an Iterator its "next requires an Iterator receiver"). The
  body is compiled once.
- **Ranges:** literal Ranges of immediate Ints count in locals; other bounds call the
  prelude's `Range::until` or `Range::through` with their contract checks and iterate the
  Range. Range objects step natively in the core when their fields are immediate (v1's fast
  path), else through `Range::next`. Like v1, loops over Range variables that nothing observes
  (v1's `unobserved_range_loops`) count in locals and write `current` and `done` back on exit
  (`for_range_variable_1000`: v1 108,074, v2 58,160). The prelude's eager helpers (`collect`,
  `count`, `to_list`, `map` and the others) run on v2. Reachability adds the `Iterator`
  implementations of the types a program can construct, so `HostStream` (phase 4) stays out.
- **Data budget:** `for` loops now reach `Range::next` and `IteratorStep`, so a program with a
  literal Range loop crosses into a second 16 KiB data block (`for_range` 91,295 → 108,685,
  +16,384 of it from the block). The template diet is now the main fuel lever.

### Phase 3 summary

| Suite on v2 | after 3a | after 3f | remaining failures |
|---|---:|---:|---|
| `runtime_values` | 26 / 96 | 87 / 96 | host operations (8), more than 65,534 nominal types (1) |
| `protocols_and_nominal` | 3 / 27 | 20 / 27 | host operations |
| `contracts_and_errors` | 8 / 15 | 12 / 15 | Error runner inputs (phase 5) |
| `fuel` | 8 / 18 | 16 / 18 | host operations |
| `limits` | 9 / 11 | 9 / 11 | host operations |
| `closures_parallel` | 1 / 17 | 7 / 17 | `par`, host operations |
| `host_calls` | 2 / 31 | 11 / 31 | phases 4 and 5 |
| `continuations` | 0 / 18 | 0 / 18 | phase 4 |

No test produces a value that differs from v1. Phase 3's exit criterion, every non-async
runtime-value test green, holds except for programs with more than 65,534 nominal types,
which need a heap type-id fallback for every inline tag test.

## 12s. One semantics for v1's lowering modes (before phase 4)

v1 lowers a function as resumable when it may suspend, has a `for` loop or calls a binding,
and every function once the program has any closure. Its resumable lowering differed from the
synchronous one: an Error of a static call (`Int::parse`, `Float::parse`, `Object::from_entries`,
`Math::*`) or of a built-in comparison returned from the function instead of being a value; a
`Compare` result was tested before its Error check; and parameter, condition, enum payload and
`let` decoding checks reported other spans. Synchronous semantics is now canonical (approved):
v1's resumable lowering follows it, and every v2 equivalence test also runs v1 with all
functions resumable and requires the same result. v2 therefore keeps precise suspendability,
and its workaround for resumable parameter spans is gone.

## 12t. Phase 4a results: frames, resumable functions and host calls

- **Which units are resumable:** only those that can reach a suspend point, a host operation
  directly or through calls, dispatch, binding calls of a resumable closure or `for` loops
  over a resumable iterator (`resumable/analysis.rs`). All other code stays synchronous, so
  `Host::call` in `main` makes `main` resumable but none of the functions it calls.
- **Frames:** a frame is a heap `Frame` object: the step index in its header word, the
  continuation state, the caller's address (traced by the collector), the caller's
  destination slot and the durable slots. The VM state block's `TaskState` holds the active
  frame, a ready host result and the completed value as a second GC root range, plus the
  current, next and cancelled host-call ids. The core library creates, links and completes
  frames (`frame_start`, `frame_call`, `frame_return`) and keeps their trace entries, so
  Errors carry v1's traces through resumable calls.
- **Step functions:** v2 reuses v1's continuation graph unchanged and emits one step function
  per resumable unit, `(frame) -> status`, with v1's state machine (loop, blocks, `br_table`).
  Most value operations rebuild their source expression over temporary bindings of the
  operand slots and compile it with the synchronous lowering, so resumable code has exactly
  the synchronous semantics (section 12s); calls of synchronous functions, methods and
  operators run inline, calls of resumable units link a child frame and return to the
  dispatcher. Temporaries are released after each operation and, since the frame keeps the
  operands alive, are not rooted again.
- **Run loop and protocol:** the generated run loop dispatches the active frame to its step
  function until the task waits or completes, then encodes the result. A resumable entry
  decodes its arguments like a synchronous one (including variadic entries and the ArityError
  of excess arguments) into its root frame. The runner protocol is v1's: `__exs_start_*`,
  `__exs_resume_host(call_id, address, length)` and `__exs_cancel`, with the host-call
  imports of v1. A host call encodes its arguments as CBOR, a ready result continues at once,
  a pending one suspends the task; v1's Errors for an invalid name, arguments or an argument
  that cannot cross the boundary are values.
- **Host time and streams:** `Host::now`, `Host::elapsed`, the DateTime time-zone operations
  and `Host::stream` build their prelude values like v1; reachability now treats `HostStream`
  as constructible by `Host::stream`, so `for` loops over streams reach `HostStream::next`.

| Suite on v2 | after 3f | after 4a | remaining failures |
|---|---:|---:|---|
| `continuations` | 0 / 18 | 18 / 18 | |
| `host_calls` | 11 / 31 | 27 / 31 | closures that suspend (3, phase 4b), `par` (1, phase 4c) |
| `closures_parallel` | 7 / 17 | 8 / 17 | `par` (7), closures that suspend (2) |
| `limits` | 9 / 11 | 11 / 11 | |
| `fuel` | 16 / 18 | 18 / 18 | |
| `runtime_values` | 87 / 96 | 95 / 96 | more than 65,534 nominal types |
| `protocols_and_nominal` | 20 / 27 | 26 / 27 | enum input of an undeclared variant (section 12o) |
| `contracts_and_errors` | 15 / 15 | 15 / 15 | |

| Fuel | v1 | v2 |
|---|---:|---:|
| `async_int_loop_100` | 88,929 | 54,804 |
| `async_empty_while_1000` | 204,743 | 163,506 |
| `async_int_while_1000` | 367,762 | 356,600 |
| `async_call_loop_100` | 93,547 | 41,197 |
| `resumable_call_loop_100` | 164,417 | 40,097 |

`backend_v2.rs` compares host calls that finish at once and that suspend, resumable child
calls, methods, operators, `Compare` and `Iterator` implementations, variadic and entry
parameters, `?`, formatted strings, `match`, and the Errors and traces of failed host calls
and contract checks. One difference stays outside the tests: v1 does not accept an
`Ordering` that a host returns as the result of a `Compare` implementation, v2 does, since it
decodes host enums into the program's enum types (the host-enum gap of section 12o).
Resumable loops still spend most of their fuel on slot traffic (`async_int_while_1000` is
only 3 % below v1); phase 4d measures and reduces it.

## 12u. Phase 4b results: closures that suspend

- A lifted closure that may suspend compiles to a step function like any resumable unit; its
  frame holds the captured Cells first, then the fixed parameters and the List of a variadic
  tail, like v1's closure frames. The synchronous dispatcher keeps a case for it that is
  never taken.
- A binding call in resumable code selects the closure's dense index with one `br_table`: a
  closure that may suspend links a child frame with its Cells and arguments (or stores v1's
  ArityError), every other value takes the synchronous closure call with its Errors for
  values that are no closures.
- Resumable methods take variadic arguments as a packed List and report v1's ArityError
  for other argument counts. When a closure may suspend, the prelude's eager helpers
  (`map`, `filter` and the others) are resumable and run as child frames.

| Suite on v2 | after 4a | after 4b | remaining failures |
|---|---:|---:|---|
| `host_calls` | 27 / 31 | 29 / 31 | `par` (phase 4c) |
| `closures_parallel` | 8 / 17 | 9 / 17 | `par` (phase 4c) |

`backend_v2.rs` compares closures that suspend through bindings, Lists, nested closures and
the eager helpers, Cells shared across suspensions, variadic closures, ArityErrors and
contract failures inside closures.

## 12v. Phase 4c results: `par`, the scheduler and cancellation

- **Tasks:** every task is a heap `Task` object linked into the live tasks; runnable tasks
  wait in a first-in, first-out run queue. The running task keeps its frame, host call, ready
  host result and quantum in the VM's `TaskState`, whose references (frame, results, running
  task, queue, live tasks) are one GC root range, and saves them into its object when it
  stops. The core library schedules (`task_suspend`, `task_checkpoint`, `task_spawn`,
  `parallel_wait`); the runner's task permits are taken for every `par` child and released
  when it completes or the execution ends.
- **`par`:** the parent creates its result List, starts one child task per closure with the
  closure's Cells in the root frame and waits; each child stores its result at its index and
  the last one wakes the parent. A dynamic `par` first returns v1's Errors for an operand
  that is no List, an element that is no closure and a closure that takes arguments. Every
  closure `par` starts runs as frames: the closures a static `par` lists, and every closure
  of a program with a dynamic `par`. A fatal Error in any task ends every task and completes
  the execution with it.
- **Host calls:** a pending host call suspends only its task; other runnable tasks run
  meanwhile and the runner polls every pending call, as with v1. A response for a call no
  task waits for any more reports the cancellation.
- **Traces:** frames record their trace entries, and switching tasks rebuilds the trace stack
  from the running task's frames and those of the tasks it was started from. A child's root
  frame has no entry, like v1's.
- **Interleaving (approved semantic change):** a task switches at suspension points and at
  scheduler checkpoints of resumable code, the back edges of loops and child calls, after
  v1's quantum of 64. Synchronous code runs without interruption, while v1 makes every
  function of a program with a closure resumable. Results of `par` stay in source order;
  side effects of parallel tasks on shared state may interleave differently from v1.

| Suite on v2 | after 4b | after 4c | remaining failures |
|---|---:|---:|---|
| `host_calls` | 29 / 31 | 31 / 31 | |
| `closures_parallel` | 9 / 17 | 16 / 17 | a 2 MiB memory limit, which v2's initial memory exceeds (see below) |
| `continuations`, `limits`, `fuel`, `contracts_and_errors` | all | all | |

`reuses_parallel_child_frames_within_a_constrained_memory_budget` runs 20,000 `par` blocks
under a 2 MiB memory limit. v2 fails before the program starts: the template's 1 MiB Rust
stack, the 1 MiB root stack and the trace and heap pages exceed the limit. This is a memory
layout question outside phase 4.

`backend_v2.rs` compares static and dynamic `par` with host calls that suspend single tasks,
nested `par`, `par` inside closures and loops of 300 `par` blocks, recoverable Errors of
single tasks, their traces, v1's Errors for invalid dynamic operands, and fatal Errors that
end every task.

## 12w. Phase 4d: measurements and the end of phase 4

Fuel (Winch) and median execution time of 21 runs, without module compilation, from
`reports_resumable_fuel_and_time`; `echo` suspends the task, `add` finishes at once:

| Scenario | v1 fuel | v2 fuel | v1 time | v2 time |
|---|---:|---:|---:|---:|
| host call, then a loop of 1,000 | 362,144 | 351,360 | 179 µs | 145 µs |
| 100 pending host calls | 493,464 | 310,837 | 370 µs | 275 µs |
| 100 ready host calls | 481,284 | 302,299 | 308 µs | 207 µs |
| 100 calls of a resumable function | 214,164 | 154,946 | 174 µs | 119 µs |
| 100 calls of a suspending closure | 278,647 | 156,410 | 206 µs | 111 µs |
| `par` over 100 tasks with host calls | 1,039,865 | 526,461 | 592 µs | 372 µs |

The fuel suite's resumable scenarios: `async_int_loop_100` 88,929 → 54,804,
`async_empty_while_1000` 204,743 → 165,680, `async_int_while_1000` 367,762 → 358,774,
`async_call_loop_100` 93,547 → 41,197, `resumable_call_loop_100` 164,417 → 40,097. Precise
suspendability keeps every synchronous function at its synchronous cost; what remains
expensive is a loop inside a resumable function, whose operations go through frame slots.

### Phase 4 summary

| Suite on v2 | after 3f | after 4c | remaining failures |
|---|---:|---:|---|
| `continuations` | 0 / 18 | 18 / 18 | |
| `host_calls` | 11 / 31 | 31 / 31 | |
| `closures_parallel` | 7 / 17 | 16 / 17 | 2 MiB memory limit (section 12v) |
| `limits` | 9 / 11 | 11 / 11 | |
| `fuel` | 16 / 18 | 18 / 18 | |
| `runtime_values` | 87 / 96 | 95 / 96 | more than 65,534 nominal types |
| `protocols_and_nominal` | 20 / 27 | 26 / 27 | enum input of an undeclared variant (section 12o) |
| `contracts_and_errors` | 15 / 15 | 15 / 15 | |

Phase 4's exit criterion, the continuation, host-call, closure and parallel and limit suites
green on v2, holds except for the memory-limit test. No compared program differs from v1,
except the documented ones: the interleaving of parallel tasks on shared state (approved) and
a host-decoded `Ordering` as a `Compare` result (section 12t).

## 13. Risks and open questions

- **GC correctness:** every value that survives an allocation must be reachable from a root.
  The precise root frames exist, but every inline allocation site and every helper that
  allocates must keep that discipline. Mitigation: a debug mode that collects on every
  allocation, run against the full test suite.
- **Shape growth:** shapes are never collected, so keys computed from data create shapes for
  the rest of the execution. This is bounded by the execution, but a script that builds Objects
  with many distinct dynamic keys keeps all of their shapes.
- **Template data budget:** fuel grows by 16,384 per started 16 KiB of initialized memory span
  on every execution, and template and program data share that span (sections 12j and 12l).
  A planned template diet (panic-free indexing, fewer panic locations) would give programs
  more room for literals.
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
