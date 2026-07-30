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
- [x] Support a function-only module root with exactly one-parameter `fn main(input)`.
- [x] Support local `let`, reassignment, `ret`, semicolon validation, integers, floats, booleans, mixed numeric arithmetic/comparisons, logical expressions, calls, conditionals, `while`, and `break`/`continue`.
- [x] Lower direct functions to Wasm and link them with the runtime template into one `.wasm` output.
- [x] Implement a minimal Wasmtime runner that passes an `ExsValue` CBOR input to `fn main(input)` and returns an `ExsValue` result without exposing internal references.
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
- [ ] Extend the fixed one-value `main` entry ABI to support multiple inputs.
- [ ] Add type - struct like objects with defined keys and functions with impl.
- [ ] Add trait declarations, implementations, resolution, and dispatch.
- [ ] Implement built-in `ToString`, `PropertyKey`, `Equality`, and `Clone` traits.

### Phase 7: Closures

- [ ] Add closure discovery, capture analysis, Cells, and closure runtime objects.
- [ ] Preserve shared mutable binding identity across nested closures.

### Phase 8: Deep clone

- [ ] Add Clone contexts that preserve cycles and aliases.
- [ ] Support user-defined, potentially suspendable Clone implementations.

### Phase 9: Runner and host boundary

- [ ] Add canonical CBOR host input/output handling.
- [ ] Add a server host registry with typed synchronous and asynchronous adapters.
- [ ] Validate host schemas and preserve language Errors as normal Values.

### Phase 10: Suspension

- [ ] Build the call graph and transitive suspendability analysis.
- [ ] Lower suspendable functions into state machines with async frames.
- [ ] Add hostcall resume and the synchronous fast path.

### Phase 11: Scheduler

- [ ] Add the execution context, task states, deterministic runnable queue, cancellation, and limits.

### Phase 12: `par`

- [ ] Add static `par { ... }` and dynamic `par(list)` lowering.
- [ ] Preserve source-order results and continue sibling tasks after recoverable Errors.

### Phase 13: Browser runner

- [ ] Implement the equivalent TypeScript runner and synchronous/asynchronous host registry.

### Phase 14: Optimizations

- [ ] Add host-name caching, direct-operation specialization, root-frame reduction, and safe inlining.

## Development Rules

- Stable Rust is required.
- `wasm-encoder`, `wasmparser`, and `wasmtime` are approved dependencies for the initial implementation.
- `crates/exs-runtime/exs-runtime.wasm` is a committed Rust-compiled artifact embedded by the `exs-runtime` crate. Compiler users never build it themselves.
- Run `cargo fmt`, `cargo test`, `cargo check`, and `cargo clippy` after Rust changes.
