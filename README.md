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

- **Values:** a `ValueRef` is 64 bits wide; None, Bool, and Ints in the `i32` range are immediates, every other value lives on a non-moving heap in linear memory. Untyped Objects share hidden-class shapes; nominal types use fixed field layouts.
- **Memory:** a stop-the-world mark-and-sweep collector in `exs-core` finds live values from precise root frames the compiler emits. When memory growth reaches the runner's limit, it collects and retries before the execution fails.
- **Tasks:** functions that may suspend compile to resumable step functions with frames in linear memory. The scheduler state of `par` tasks lives in the module's memory and its logic in `exs-core`; a task switches only when suspending or at checkpoints in loops and calls of resumable code.
- **Boundary:** inputs, host responses, results, and host-call arguments are acyclic by-value images (`exs_abi::image`), bounded by `BoundaryLimits`.

## Development Rules

- Stable Rust is required.
- `wasm-encoder`, `wasmparser`, and `wasmtime` are approved dependencies for the initial implementation.
- `crates/exs-core-template/exs-core-template.wasm` is a committed Rust-compiled artifact embedded by the compiler. Compiler users never build it themselves; after changes to `exs-core` or the template, rebuild it with `nu crates/exs-core-template/build-template.nu`.
- Run `cargo fmt`, `cargo test`, `cargo check`, and `cargo clippy` after Rust changes.
- Invoke a program with positional values using `exs run app.exs -- 1 Ada "[3, 'four']"`.

### Compiler limits

`CompileOptions::limits` bounds one compilation of untrusted source: 1 MiB per source file, 8 MiB per project, 512 modules and 32 nesting levels of expressions, blocks and type annotations by default. Adjust them with struct update syntax:

```rust
let mut options = CompileOptions::default();
options.limits.max_nesting = 32;
let module = compile(source, options)?;
```

### Libraries

An application provides library modules, such as `log` or `exo::db`, through its `ModuleResolver`: `library(path)` returns the source of the library module at `path`, and `libraries()` lists every path. Scripts name library items without importing them (`log::info("ready")`) or bring them into scope with `use exo::db::load;`. The compiler loads only the libraries a script names. Declarations are public by default; `private` keeps a declaration inside its library or project.

```rust
impl ModuleResolver for AppResolver {
    fn resolve(&mut self, importer: &str, path: &str) -> Result<ResolvedSource, String> {
        Err(format!("{importer} cannot import {path}"))
    }

    fn library(&mut self, path: &str) -> Option<ResolvedSource> {
        let text = match path {
            "log" => LOG_SOURCE,
            "exo::db" => DB_SOURCE,
            _ => return None,
        };
        Some(ResolvedSource { source_id: path.to_owned(), text: text.to_owned() })
    }

    fn libraries(&mut self) -> Vec<String> {
        vec!["log".to_owned(), "exo::db".to_owned()]
    }
}
```

`CompletionEngine::with_library(path, source)` offers the public declarations of a library module to editor completions.

### Documentation

`document_with_resolver` documents the standard library, a project, and the libraries it names; `document_libraries` documents the standard library and every library `libraries()` lists. Both return an `ApiDocumentation` model. `markdown(&MarkdownOptions)` renders it as pages, one per module by default or one per type, enum, trait, and namespace with `PageLayout::PerItem`; the options also set the index file name, an absolute link prefix, and a front-matter callback for static-site generators such as Zola. `llm()` renders the language specification and a compact API reference as one Markdown document. `exs docs app.exs -o docs` writes the pages and `exs docs app.exs -o api.md --llm` the compact reference.

```rust
let front_matter = |page: &PageInfo<'_>| format!("+++\ntitle = \"{}\"\n+++\n\n", page.title);
let pages = document_libraries(&mut resolver)?.markdown(&MarkdownOptions {
    layout: PageLayout::PerItem,
    index_file: "_index.md",
    link_prefix: Some("@/exs/"),
    title_heading: false,
    front_matter: Some(&front_matter),
});
```

### Server runner

`ServerRunner::new` takes a `ServerRunnerConfig` with the memory, fuel and wall-clock limits, the protection level for host-side resources, the Wasmtime compiler strategy and the module cache size. Every field has a default suited to untrusted source.

`execute` is an ordinary future that works with any executor. Running guest code yields to the executor every 100,000 fuel, so a busy script does not block the executor thread, and it stops at its wall-clock limit or cancellation. One process-wide timer thread serves the deadlines of all executions and `Host::sleep`; an execution starts no thread of its own.

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

## Open Points

- CI does not verify that the committed `exs-core-template.wasm` matches its source; builds on macOS and Linux currently differ.
- The Exolynk embedding still has to move to library modules, `private`, and the documentation options.
- Optimizations: host-name caching, direct-operation specialization, root-frame reduction, and inlining.
- Graph references at the boundary for cyclic and aliased Lists and Objects; host-resource capability values; runner instance pooling.

Risks to keep in mind:

- Every value that survives an allocation must be reachable from a root; each allocating helper must keep that discipline.
- Shapes are never collected, so Objects with many distinct dynamic keys keep their shapes for the rest of the execution.
- The non-moving heap can fragment in long-running tasks.
- Winch generates slower machine code than Cranelift; which compiler production uses is a separate decision.
