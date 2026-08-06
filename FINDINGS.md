**Findings**

7. **Low: The specification contradicts the implementation and itself.** It still limits literals to 55 bits although `Int` is otherwise defined and implemented as signed 64-bit; it also lists `NotSerializable` while the documented and emitted error is `SerializationError`. [number rule](/Users/roba/Code/exs/SPECIFICATION.md:66), [error list](/Users/roba/Code/exs/SPECIFICATION.md:439), [boundary behavior](/Users/roba/Code/exs/SPECIFICATION.md:521).

8. **Low: Target-wide Wasm linting/testing is broken.** `cargo clippy --workspace --all-targets --target wasm32-unknown-unknown -- -D warnings` fails with duplicate `panic_impl`, because the runtime’s panic handler is included with Cargo’s std test harness. [panic handler](/Users/roba/Code/exs/crates/exs-runtime/src/wasm.rs:833). Gate it outside tests or disable the unsupported runtime test target, then add the target-wide command to CI.
