# ExS

Exolynk Script (ExS) is a dynamically typed scripting language that compiles directly to one executable WebAssembly module. It has no bytecode, interpreter, or language-level virtual machine.

The authoritative language and architecture definition is [SPECIFICATION.md](SPECIFICATION.md).

## Architecture

```text
ExS source -> compiler + embedded exs-runtime.wasm -> final Wasm module
                                                               |
                                                     server or browser runner
                                                               |
                                                              host
```

The `exs-runtime` executes inside the final Wasm module. A runner executes outside the module, supplies the Host ABI, and enforces execution limits. The compiler and runtime communicate only through stable named ABI exports; neither relies on fixed Wasm function indices.

## Planned Workspace

```text
.
├── crates/
│   ├── exs-value/       # shared opaque ValueRef(NonZeroU32) carrier
│   ├── exs-abi/         # ABI versions, stable names, and ExsValue CBOR transport
│   ├── exs-runtime/     # Rust runtime source and committed Wasm template
│   ├── exs-compiler/    # source to linked Wasm compiler library
│   ├── exs-runner/      # Wasmtime-based server runner
│   └── exs-cli/         # thin compiler/runner command-line interface
├── tests/               # conformance and end-to-end tests
├── SPECIFICATION.md
└── Cargo.toml
```

## Implementation Roadmap

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

### Phase 4: Garbage collection

- [x] Implement stop-the-world mark-and-sweep collection with reusable value-table slots.
- [x] Add compiler-generated root frames, temporary runtime roots, and List/Object heap scanning.
- [x] Test aliasing, cycles, and allocation-triggered collection, including allocation-heavy loops.

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

### Phase 13: Deep clone

- [ ] Implement built-in `ToString`, `PropertyKey`, `Equality`, and `Clone` traits.
- [ ] Add Clone contexts that preserve cycles and aliases.
- [ ] Support user-defined, potentially suspendable Clone implementations.

### Phase 14: Limits

- [ ] Add runner-enforced limits for memory, fuel, timeouts, task and host-call counts, stack depth, CBOR payloads, and results.

### Phase 15: Browser runner

- [ ] Implement the equivalent TypeScript runner and synchronous/asynchronous host registry.

### Phase 16: Optimizations

- [ ] Add host-name caching, direct-operation specialization, root-frame reduction, and safe inlining.

## Development Rules

- Stable Rust is required.
- `wasm-encoder`, `wasmparser`, and `wasmtime` are approved dependencies for the initial implementation.
- `crates/exs-runtime/exs-runtime.wasm` is a committed Rust-compiled artifact embedded by the `exs-runtime` crate. Compiler users never build it themselves.
- Run `cargo fmt`, `cargo test`, `cargo check`, and `cargo clippy` after Rust changes.
- Invoke a program with positional values using `exs run app.exs -- 1 Ada "[3, 'four']"`.
