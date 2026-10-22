# Exolynk Extension System

The Exolynk Extension System is a dynamic addon system, to develop custom code and execute it savely in the browser or server. The Exolynk Script (ExS) is a dynamically typed scripting language that compiles directly to one executable WebAssembly module. It's designed to support easy scripting used in the extension system. For more advanved scripting, additional langugaes are supported to generate .wasm to be executed in exs.

The Exolynk Script and it's architecture definition is located at [SPECIFICATION.md](SPECIFICATION.md).

## Architecture

```text
ExS source -> compiler + embedded exs-core-template.wasm -> final Wasm module
                                                               |
                                                     server or browser runner
                                                               |
                                                              host
```

Values live in the module's linear memory in the layout of `exs-layout`. The compiler generates the program's code and links the stateless functions of the core library (`exs-core`, embedded as `exs-core-template.wasm`) into the final Wasm module. A runner executes outside the module, supplies the Host ABI, and enforces execution limits. It passes inputs and host responses as images the module adopts without parsing, and reads results and host-call arguments directly from the module's memory. Rust guests built with `exs-guest` exchange every value as an image.

## Development Rules

- Stable Rust is required.
- `wasm-encoder`, `wasmparser`, and `wasmtime` are approved dependencies for the initial implementation.
- `crates/exs-core-template/exs-core-template.wasm` is a committed Rust-compiled artifact embedded by the compiler. Compiler users never build it themselves; after changes to `exs-core` or the template, rebuild it with `nu crates/exs-core-template/build-template.nu`.
- Run `cargo fmt`, `cargo test`, `cargo check`, and `cargo clippy` after Rust changes.
- Invoke a program with positional values using `exs run app.exs -- 1 Ada "[3, 'four']"`.

### Compiler limits

`CompileOptions::limits` bounds one compilation of untrusted source: 1 MiB per source file, 8 MiB per project, 512 modules and 64 nesting levels of expressions, blocks and type annotations by default. Adjust them with struct update syntax:

```rust
let mut options = CompileOptions::default();
options.limits.max_nesting = 32;
let module = compile(source, options)?;
```

### Server runner

`ServerRunner::new` takes a `ServerRunnerConfig` with the memory, fuel and wall-clock limits, the protection level for host-side resources, the Wasmtime compiler strategy and the module cache size. Every field has a default suited to untrusted source:

```rust
let mut runner = ServerRunner::new(ServerRunnerConfig {
    max_fuel: 50_000_000,
    timeout: Duration::from_secs(30),
    ..ServerRunnerConfig::default()
});
runner.registry_mut().fn_sync_raw("log", |arguments| ExsValue::None)?;
let result = runner.execute(&module.wasm, "main", &inputs, &ExecutionCancellation::new()).await?;
```

The runner executes modules compiled for the current ABI and Rust guests built with the same `exs-guest` version; modules compiled for an earlier ABI version are rejected and must be recompiled.

### Browser runner

`exs-runner` keeps the Wasmtime-backed `server` feature enabled by default. A Rust-Wasm browser application uses the browser-only backend without compiling Wasmtime:

```toml
exs-runner = { version = "0.2.0", default-features = false, features = ["browser"] }
```

```rust
let mut config = BrowserRunnerConfig::new();
config.registry_mut().fn_sync_raw("log", |arguments| {
    // Call application-owned Rust-Wasm state here.
    ExsValue::None
})?;
let runner = BrowserRunner::new(&compiled_wasm, config).await?;
let result = runner.execute("main", &inputs).await?;
```

The browser feature uses the application's `wasm-bindgen` JavaScript glue to instantiate the separate ExS module with native `WebAssembly` APIs. Each `execute` call receives a fresh ExS instance from one browser-compiled module, preserving execution isolation. Synchronous host callbacks use the ready path; asynchronous Rust futures resolve through browser Promises and resume the ExS task through the existing Host ABI.

Both runners also support runner-registered pull streams through `Host::stream(name, arguments...)`. Native registrations implement `HostStream`; browser registrations implement `BrowserHostStream`. Stream state is isolated to each execution and released on completion, cancellation, or `IteratorStep::Done`.

`BrowserRunner` executes Wasm on the calling JavaScript thread and does not enforce fuel, wall-clock, memory, task, or host-call limits. It must therefore run untrusted ExS source inside a dedicated Web Worker. The embedding application owns the Worker lifecycle and should terminate it when its resource policy is exceeded; executing untrusted code on the main thread can freeze the user interface.
