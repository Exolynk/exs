# Exolynk Extension System

The Exolynk Extension System is a dynamic addon system, to develop custom code and execute it savely in the browser or server. The Exolynk Script (ExS) is a dynamically typed scripting language that compiles directly to one executable WebAssembly module. It's designed to support easy scripting used in the extension system. For more advanved scripting, additional langugaes are supported to generate .wasm to be executed in exs.

The Exolynk Script and it's architecture definition is located at [SPECIFICATION.md](SPECIFICATION.md).

## Architecture

```text
ExS source -> compiler + embedded exs-runtime.wasm -> final Wasm module
                                                               |
                                                     server or browser runner
                                                               |
                                                              host
```

The `exs-runtime` executes inside the final Wasm module. A runner executes outside the module, supplies the Host ABI, and enforces execution limits. The compiler and runtime communicate only through stable named ABI exports; neither relies on fixed Wasm function indices.

## Development Rules

- Stable Rust is required.
- `wasm-encoder`, `wasmparser`, and `wasmtime` are approved dependencies for the initial implementation.
- `crates/exs-runtime/exs-runtime.wasm` is a committed Rust-compiled artifact embedded by the `exs-runtime` crate. Compiler users never build it themselves.
- Run `cargo fmt`, `cargo test`, `cargo check`, and `cargo clippy` after Rust changes.
- Invoke a program with positional values using `exs run app.exs -- 1 Ada "[3, 'four']"`.

### Browser runner

`exs-runner` keeps the Wasmtime-backed `server` feature enabled by default. A Rust-Wasm browser application uses the browser-only backend without compiling Wasmtime:

```toml
exs-runner = { version = "0.1.0", default-features = false, features = ["browser"] }
```

```rust
let mut config = BrowserRunnerConfig::new();
config.registry_mut().register_sync("log", |arguments| {
    // Call application-owned Rust-Wasm state here.
    ExsValue::None
})?;
let runner = BrowserRunner::new(&compiled_wasm, config).await?;
let result = runner.execute(&inputs).await?;
```

The browser feature uses the application's `wasm-bindgen` JavaScript glue to instantiate the separate ExS module with native `WebAssembly` APIs. Each `execute` call receives a fresh ExS instance from one browser-compiled module, preserving execution isolation. Synchronous host callbacks use the ready path; asynchronous Rust futures resolve through browser Promises and resume the ExS task through the existing Host ABI.

Both runners also support runner-registered pull streams through `Host::stream(name, arguments...)`. Native registrations implement `HostStream`; browser registrations implement `BrowserHostStream`. Stream state is isolated to each execution and released on completion, cancellation, or `IteratorStep::Done`.

`BrowserRunner` executes Wasm on the calling JavaScript thread and does not enforce fuel, wall-clock, memory, task, host-call, or CBOR limits. It must therefore run untrusted ExS source inside a dedicated Web Worker. The embedding application owns the Worker lifecycle and should terminate it when its resource policy is exceeded; executing untrusted code on the main thread can freeze the user interface.
