## Implementation Roadmap

### Documentation Boundary

The [language reference](SPECIFICATION.md) defines only the ExS source language and its observable behavior. It is the document to give an authoring tool or LLM that needs to write ExS code. Compiler, runtime, runner, WebAssembly, CBOR, and ABI implementation details belong in crate documentation and this roadmap.

### Phase 1: Minimal compiler

- [x] Create the final Rust workspace and shared `exs-value` and `exs-abi` crates.
- [x] Define `#[repr(transparent)] ValueRef(NonZeroU32)` for runtime-allocated values.
- [x] Compile and commit `crates/exs-runtime/exs-runtime.wasm` for compiler linking.
- [x] Implement source loading, lexer, parser, `Module` AST, source spans, and diagnostics.
- [x] Support a function-only module root with `fn main(...)`, including zero or multiple parameters.
- [x] Support local `let`, reassignment, `ret`, semicolon validation, integers, floats, booleans, mixed numeric arithmetic/comparisons, logical expressions, calls, conditionals, `while`, and `break`/`continue`.
- [x] Lower direct functions to Wasm and link them with the runtime template into one `.wasm` output.
- [x] Implement a minimal Wasmtime runner that passes an ordered `ExsValue` CBOR input array to `fn main(...)` and returns an `ExsValue` result without exposing internal references.
- [x] Add lexer, parser, compiler, and end-to-end Wasm execution tests.

### Phase 2: Dynamic values and runtime ABI

- [x] Add runtime-allocated primitive values and checked operations through named `__exs_rt_*` exports.
- [x] Add shared `ExsValue` CBOR input/output buffers and runner ABI validation.
- [x] Resolve named runtime exports from the committed Wasm template without depending on heap layouts or fixed Wasm function indices.

### Phase 3: Heap values

- [x] Add immutable UTF-8 Strings as boxed `RtValue` variants with literal, CBOR, and content-equality support.
- [x] Add mutable boxed Lists with literals, dynamic indexing, index assignment, generic member dispatch for `list.push(value)`, and recursive CBOR input/output.
- [x] Add insertion-ordered boxed Objects with literals, generic bracket/dot access, mutation, member dispatch, and recursive CBOR input/output.
- [x] Complete generic List operations: `push`, `pop`, `insert`, `remove`, `clear`, and shallow `List + value` / `List + List`.
- [x] Add `for item in iterable` with runtime-owned shallow List snapshots and Unicode-scalar String snapshots.
- [x] Limit the current host boundary to acyclic nested Lists and Objects; graph-reference CBOR is not implemented.

### Phase 4: Garbage collection

- [x] Implement stop-the-world mark-and-sweep collection with reusable value-table slots.
- [x] Add compiler-generated root frames, temporary runtime roots, and List/Object heap scanning.
- [x] Test aliasing, cycles, and allocation-triggered collection, including allocation-heavy loops.
- [x] Use stop-the-world mark-and-sweep before language-value allocation, with reusable value-table slots and compiler/runtime root tracking.

### Phase 5: Errors, Options, and source maps

- [x] Replace Null with direct None and structured Error transport values.
- [x] Add source-level None, `is Error`, and ? propagation with MissingValue conversion.
- [x] Convert recoverable numeric, condition, List, Object, iterable, and method validation failures from traps to Error values.
- [x] Emit compact source-position IDs, `exs.source.map`, and optional embedded `exs.sources` text.
- [x] Add direct-function language stack traces; async-frame traces remain part of suspension work.

### Phase 6: Traits & Types

- [x] Add optional parameter and return union annotations for non-entry functions, with dynamic runtime contracts for `Any`, `None`, `Error`, `Bool`, `Int`, `Float`, `String`, `List`, and `Object`.
- [x] Return recoverable `TypeError` for a contract mismatch when `Error` is allowed and a source-mapped fatal `TypeError` for strict return contracts. `?` requires `Error` or `Any` in the return annotation.
- [x] Support zero or more typed `main` inputs, substitute `None` for missing values, and return fatal `ArityError` for excess values.
- [x] Add nominal Object types with declared fields, named type contracts, explicit `None` for omitted optional fields, instance `impl` methods, and static `Type::method()` methods.
- [x] Add nominal trait declarations, required signatures, default methods, `impl Trait for Type` implementations, trait contracts, and instance/static dispatch. Trait and type names share one namespace; duplicate exposed method names are rejected.

### Phase 7: Runner and host boundary

- [x] Add canonical CBOR List argument decoding and `ExsValue` result encoding for host calls.
- [x] Add a server host registry with synchronous and asynchronous `Vec<ExsValue> -> ExsValue` adapters.
- [x] Reject invalid argument payloads and duplicate static host names while preserving language Errors as normal Values.

### Phase 8: Suspension

- [x] Build a conservative call graph and transitive host-call suspendability analysis.
- [x] Add runtime async frames, GC roots, Host ABI ready/pending transport, and a generated root dispatcher.
- [x] Add the synchronous host fast path and asynchronous `ServerRunner` resume delivery for the initial `ret host.call(name, arguments...);` main-function path.
- [x] Replace the initial path with a full continuation IR that assigns durable slots for lexical bindings and expression temporaries.
- [x] Lower sequential statements, nested expressions, assignments, `ret`, and `?` into continuation states.
- [x] Lower `if`, `while`, `for`, `break`, and `continue` into graph branch and loop states.
- [x] Lower transitive suspendable direct, instance, static, and trait calls through child frames and caller continuations.
- [x] Preserve function contracts, source positions, language stack traces, and recoverable Error propagation in resumable frames.
- [x] Add end-to-end coverage for synchronous and asynchronous calls at every supported source position.

### Phase 9: Scheduler

- [x] Add the execution context, root-task lifecycle, task states, deterministic runnable queue, and scheduler-owned GC roots.
- [x] Add scheduler checkpoints at function entry, loop backedges, and host resumes, with quantum-based dispatcher yields.
- [x] Route host completions to tasks, add cancellation and invalidated call IDs, and report scheduler deadlocks.

### Phase 10: Closures

- [x] Add `(parameters) => { ... }` closure expressions, unparameterized `Fn` contracts, and HIR callable-binding resolution.
- [x] Discover nested closures before linking, assign stable lifted function identities, and compute their lexical captures.
- [x] Add GC-traced runtime Cell and Closure values; capture shared Cells so outer and nested assignments retain one binding identity.
- [x] Lower closure construction and dynamic `Fn` invocation through generated continuation frames, including `host.call` suspension and cancellation.
- [x] Add end-to-end coverage for closure arguments, returned closures, nested captures, shared mutation, type contracts, and async closure bodies.

### Phase 11: `par`

- [x] Add static `par { ... }` and dynamic `par(list)` lowering using callable Values.
- [x] Preserve source-order results and continue sibling tasks after recoverable Errors.
- [x] Have a test which checks, that futures are executed in parrallel

### Phase 12: Files, Formatter & Docs

- [x] Support relative `.exs` file imports with canonical resolver identities, namespaces, merged namespaces, `as`, and `use` aliases.
- [x] Create a formatter usable as a library API and through `exs fmt <file.exs>`.
- [x] Generate Markdown language and module API documentation through `exs docs <file.exs> -o <directory>`.
- [x] Support every global built-in type with an optional `std::` qualifier, such as `std::Int` and `std::None`.

### Phase 13: Enums & Pattern Matching

- [x] Add nominal `enum` declarations with zero-payload variants and zero or more named, optionally typed payload fields.
- [x] Add qualified variant constructors such as `Color::Rgb(red, green, blue)` and `Color::Transparent`.
- [x] Support enum names in type annotations, imports, `use`, contracts, and generated documentation; allow inherent and trait `impl` blocks for enums.
- [x] Extend `ExsValue` and canonical CBOR with tagged enum values carrying an opaque resolver-derived type identity, variant name, and ordered fields.
- [x] Add `match` expressions with qualified variants, comma-separated payload bindings, and `_` fallback arms.
- [x] Require exhaustive enum matching unless a `_` arm is present.
- [x] Lower `match` through continuation graphs so arms may suspend.

### Phase 14: Standard Library & Deep Clone

- [x] Define a deliberately small direct method API for `Int`, `Float`, `String`, `List`, `Object`, and `Error`.
- [x] Render built-in method signatures and behavior on the relevant `std` type pages.
- [x] Implement automatic deep `value.clone()` for mutable graphs.
- [x] Preserve aliases and cycles while cloning Lists, Objects, Errors, nominal Objects, enums, Cells, and Closures.
- [x] Reuse immutable values where safe and reserve future host-owned runtime values as non-cloneable.
- [x] Keep clone behavior uniform; defer user-defined clone overrides.

### Phase 15: Extensible Protocols & Operators

- [x] Add contextual `Self` annotations to existing trait declarations and `impl Trait for Type` blocks.
- [x] Define the standard `Add` protocol with `fn add(self, other: Any) -> Any` for nominal types and enums.
- [x] Dispatch `+` through matching `Add` implementations, preserving built-in numeric, String, and List behavior as its fallback.
- [x] Allow `Add` implementations to suspend through the existing continuation child-frame path.
- [x] Define `Sub`, `Mul`, and `Div` as standard protocols with `Any` results, built-in numeric implementations, matching methods, and `-`, `*`, and `/` dispatch.
- [x] Define `Compare` with `fn compare(self, other: Any) -> Ordering`, the global `std::Ordering` enum, and `==`, `!=`, `<`, `<=`, `>`, and `>=` dispatch.
- [x] Add generated `std` trait pages and link every built-in and user implementation from the owning type page.
- [x] Retain non-overridable runtime-owned deep cloning for every value and document `clone()` on built-in, type, and enum pages.

### Phase 16: Runner Limits & Browser

- [x] Add an `exs-runner` browser feature that executes compiled `.wasm` modules through the browser engine and routes host calls into Rust-Wasm callbacks.
- [x] Add runner-enforced limits for memory, fuel, timeouts, tasks, host calls, stack depth, CBOR payloads, and results.
  - [x] Define `ExecutionLimits` and typed runner limit errors; bound main, host, and result CBOR payloads by byte size, nesting, and collection entries.
  - [x] Enforce native Wasmtime memory, fuel, wall-clock timeout, and Wasm stack limits.
  - [x] Enforce generic runner task permits plus total and concurrent pending host-call limits in the native runner.
  - [x] Document browser main-thread execution as application-owned isolation; applications that execute untrusted source must place their runner in a Worker.
- [x] Execute each native root in a fresh Wasmtime store and instance; instance pooling is not implemented.

### Phase 17: Fixed Proposed by Sol

- [] **High: `Host::sleep` can permanently exhaust native threads.**  
   [host_sleep.rs](/Users/roba/Code/exs/crates/exs-runner/src/host_sleep.rs:25) creates a detached OS thread for every sleep. Timeout or cancellation drops the future but cannot stop that thread. Since [Duration accepts up to `i64::MAX` seconds](/Users/roba/Code/exs/crates/exs-compiler/src/prelude/duration.exs:59), repeated executions can leave hundreds of threads sleeping indefinitely. Replace this with a shared cancellable timer mechanism and cap sleeps to the execution deadline.
- [] **High: cancellation does not interrupt executing Wasm.**  
   [ServerRunner::execute](/Users/roba/Code/exs/crates/exs-runner/src/lib.rs:147) runs guest code synchronously and checks cancellation only before execution or while awaiting host futures. A CPU-bound loop ignores `ExecutionCancellation` until fuel or timeout interrupts it. It also blocks a single-threaded async executor. Cancellation should trigger Wasmtime interruption, with a test cancelling an active infinite loop.
- [] **High under an untrusted-Wasm threat model: native runner memory is not fully bounded.**  
   Synchronous responses accumulate in [ready_responses](/Users/roba/Code/exs/crates/exs-runner/src/host_abi.rs:27), with only a per-response size check at [host_abi.rs](/Users/roba/Code/exs/crates/exs-runner/src/host_abi.rs:545). A custom Wasm module can issue unique calls without copying their responses, potentially retaining roughly `10,000 × 2 MiB` using default limits. Additionally, module compilation occurs before the timeout starts at [lib.rs](/Users/roba/Code/exs/crates/exs-runner/src/lib.rs:117). Add a total host-owned byte budget, outstanding-ready-response limit, module-size limit, and compilation isolation/caching.
- [] **Medium: the documented `exs-guest` no-std build is broken.**  
   [host.rs](/Users/roba/Code/exs/crates/exs-guest/src/host.rs:114) uses `vec!` without importing `alloc::vec`. The advertised configuration fails for `wasm32-unknown-unknown`. CI should explicitly build this feature combination.
- [] **Medium: the “strict RFC 3339” parser accepts invalid offsets.**  
   [datetime.exs](/Users/roba/Code/exs/crates/exs-compiler/src/prelude/datetime.exs:310) parses offset minutes but only validates the combined offset seconds later. Values such as `+01:60` are accepted as `+02:00`. Validate hours and minutes independently and add invalid-offset tests.
- [] **Medium: CI does not verify the committed runtime matches its source.**  
   Compilers embed the committed artifact at [exs-runtime/src/lib.rs](/Users/roba/Code/exs/crates/exs-runtime/src/lib.rs:30). CI rebuilds the runtime at [ci.yml](/Users/roba/Code/exs/.github/workflows/ci.yml:40), but never compares that output with the committed file. Runtime source changes could therefore pass tests while the old runtime still ships. The artifact currently matches a deterministic fresh build, but CI should enforce this.
- [ ] Lower priority: the CLI’s [custom `block_on`](/Users/roba/Code/exs/crates/exs-cli/src/main.rs:365) repeatedly polls with `yield_now`, consuming significant CPU during sleeps or other pending host calls.

### Backlog: Further Ideas

- [ ] Optimizations: Add host-name caching, direct-operation specialization, root-frame reduction, and safe inlining.
- [ ] Add CBOR graph-reference encoding for cyclic and aliased Lists and Objects.
- [ ] Add host-resource capability values and lifecycle management.
- [ ] Add reusable runner instance pooling when a reset strategy is available.
