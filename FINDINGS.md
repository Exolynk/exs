**Findings**

1. **High: strict contract failures are not terminal.** A fatal return-contract error is represented as a normal value, and callers can discard it and continue. A strict callee returning the wrong type can therefore be ignored by its caller, contradicting the root-termination guarantee. See [Wasm contract handling](/Users/roba/Code/exs/crates/exs-runtime/src/wasm.rs:108), [direct-call lowering](/Users/roba/Code/exs/crates/exs-compiler/src/codegen/function/lowering.rs:227), and [statement discard](/Users/roba/Code/exs/crates/exs-compiler/src/codegen/function/control.rs:122).  
   Action: make fatal errors terminal execution state, or propagate them across every call boundary before any subsequent evaluation.

2. **High: ordinary invalid dynamic calls can trap the Wasm module.** Calling a local non-closure and invalid `par(...)` operands use unchecked runtime assertions. For example, `let f = 1; f()` or `par([1])` can trap instead of producing a recoverable `TypeError` or `ArityError` as the language reference requires. See [closure arity access](/Users/roba/Code/exs/crates/exs-runtime/src/wasm.rs:531), [closure-call lowering](/Users/roba/Code/exs/crates/exs-compiler/src/codegen/continuation/step.rs:718), [parallel lowering](/Users/roba/Code/exs/crates/exs-compiler/src/codegen/continuation/step.rs:891), and [parallel list access](/Users/roba/Code/exs/crates/exs-runtime/src/runtime.rs:479).  
   Action: perform callable/list/arity checks before accessor calls and return language errors through the task result path.

3. **Medium: valid ExS values can trap during runner or host serialization.** Cyclic lists and cells, closures, and placeholder values reach `trap()` when converted for host calls or final results. A source program can create a cyclic list without violating source semantics. See [serialization conversion](/Users/roba/Code/exs/crates/exs-runtime/src/runtime.rs:859) and [unsupported runtime values](/Users/roba/Code/exs/crates/exs-runtime/src/runtime.rs:906).  
   Action: define the boundary policy: support graph encoding, or reject unsupported/cyclic values with a defined serialization error. Document and test it.

4. **Medium: runner and host inputs accept integers outside ExS’s 56-bit range.** Generic CBOR encoding accepts any `i64`; the runtime later traps when it converts such a value. This affects runner input and host responses, even though source literals are correctly checked. See [runner input encoding](/Users/roba/Code/exs/crates/exs-runner/src/lib.rs:403) and [runtime integer conversion](/Users/roba/Code/exs/crates/exs-runtime/src/runtime.rs:925).  
   Action: validate the complete external `ExsValue` tree at runner/host boundaries and return a typed ABI or runner error before entering Wasm.

5. **Low: the reference and implementation disagree on an error kind.** The specification lists `UnknownFunction`, while the runner emits `HostFunctionNotFound`. See [SPECIFICATION.md](/Users/roba/Code/exs/SPECIFICATION.md:439), [host ABI](/Users/roba/Code/exs/crates/exs-runner/src/host_abi.rs:335), and [existing test](/Users/roba/Code/exs/crates/exs-runner/tests/host_calls.rs:34).  
   Action: update the specification to `HostFunctionNotFound`, or rename the implementation consistently.

**Test gaps**

Missing integration coverage for non-callable local invocation, invalid `par` values and arities, discarded fatal contract errors, cyclic/non-serializable boundary values, and out-of-range runner or host integers.
