# ExS

ExS is an extension language for applications. Your users write scripts, your application compiles them to WebAssembly and runs them in a sandbox, on a server or in the browser. A script reaches your application only through the host functions you register, within the memory, fuel, and time limits you set.

Modules from other languages run the same way, such as Rust with `exs-guest`. A compiled module keeps running on later versions of ExS; see [COMPATIBILITY.md](COMPATIBILITY.md). [ABI.md](ABI.md) describes the contract between a module and a runner.

## Quickstart

```toml
[dependencies]
exs = "0.2"
tokio = { version = "1", features = ["macros", "rt"] }
```

```rust
use exs::{CompileOptions, ExsValue, Runner, Source};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let script = r#"
        fn main(name: String) -> String {
            ret Host::call("greet", name);
        }
    "#;
    let module = exs::compile(Source::new("hello.exs", script), CompileOptions::default())?;

    let mut runner = Runner::default();
    runner
        .registry_mut()
        .fn_sync("greet", |name: String| Ok(format!("Hello, {name}!")))?;

    let inputs = [ExsValue::String("Ada".to_owned())];
    let result = runner.execute(&module.wasm, "main", &inputs).await?;
    assert_eq!(result, ExsValue::String("Hello, Ada!".to_owned()));
    Ok(())
}
```

`module.wasm` can be stored and executed later without the compiler. Any executor works, as `execute` is an ordinary future.

## The language

ExS is dynamically typed, with optional type contracts that are checked at run time. Failures are ordinary `Error` values rather than exceptions, and `par` runs concurrent tasks.

```exs
type Order {
    customer: String,
    lines: List,
}

enum Status {
    Open,
    Shipped(tracking: String),
}

impl Order {
    fn total(self) -> Int {
        let total = 0;
        for line in self.lines {
            total = total + line.price * line.quantity;
        }
        ret total;
    }
}

fn describe(status: Status) -> String {
    ret match status {
        Status::Open => "open",
        Status::Shipped(tracking) => f"shipped as {tracking}",
    };
}

fn main(customer: String) {
    let order = Order {
        customer: customer,
        lines: [{ price: 3, quantity: 2 }, { price: 5, quantity: 4 }],
    };
    ret f"{order.customer}: {order.total()}, {describe(Status::Shipped("A-1"))}";
}
```

[SPECIFICATION.md](SPECIFICATION.md) defines the language and its architecture.

## Command line

```bash
cargo install exs-cli
```

```bash
exs run app.exs -- 1 Ada "[3, 'four']"
```

`exs` also compiles (`compile app.exs -o app.wasm`), calls one function (`call app.wasm add -- 1`), checks, formats, tests, and documents scripts. `exs --version` prints its version, and any other invalid command prints the usage.

## Crate features

| Feature | Default | Purpose |
| --- | --- | --- |
| `compiler` | yes | Compiles source to modules. |
| `server` | yes | `Runner` with Wasmtime, enforcing memory, fuel, and time limits. |
| `serde` | yes | Typed host functions and value conversions. |
| `browser` | | `Runner` through wasm-bindgen on `wasm32`. It enforces no limits. |
| `tooling` | | Highlighting, editor completions, and API documentation. |
| `json`, `url`, `rand`, `http` | | Library modules for scripts. |

## Guide

### Compiling

`compile` turns a `Source` into a module. A self-contained script needs nothing else:

```rust
let module = compile(Source::new("app.exs", text), CompileOptions::default())?;
```

A script that imports files or names libraries needs a resolver, which tells the compiler where that code comes from; the compiler never reads files itself, so a script reaches only what the resolver provides. `Sources` is a resolver for sources known up front. Implement `ModuleResolver` to load them on demand, for example from a database.

```rust
let mut sources = Sources::new();
sources.add("./math.exs", MATH);
sources.add_library("log", LOG);
let options = CompileOptions::default().resolver(&sources);
let module = compile(Source::new("app.exs", text), options)?;
```

The resolver only supplies ExS source. The functions a library calls at run time are registered with the runner.

A failure is a `CompileError`: `report` is the rendered text with source excerpts and `diagnostics` holds each error's stable code, message, source identity and byte range, also for errors in imported files and libraries.

### Compiler limits

`CompileOptions::limits` bounds one compilation of untrusted source: 1 MiB per source file, 8 MiB per project, 512 modules and 32 nesting levels of expressions, blocks and type annotations by default. Adjust them on the default value:

```rust
let mut options = CompileOptions::default();
options.limits.max_nesting = 32;
let module = compile(source, options)?;
```

### Libraries

An application provides library modules, such as `log` or `app::db`, through its resolver: `library(path)` returns the source of the library module at `path`, and `libraries()` lists every path. Scripts name library items without importing them (`log::info("ready")`) or bring them into scope with `use app::db::load;`. The compiler loads only the libraries a script names. Declarations are public by default; `private` keeps a declaration inside its library or project.

```rust
impl ModuleResolver for AppResolver {
    fn resolve(&self, importer: &str, path: &str) -> Result<Source<'_>, String> {
        Err(format!("{importer} cannot import {path}"))
    }

    fn library(&self, path: &str) -> Option<Source<'_>> {
        let text = match path {
            "log" => LOG_SOURCE,
            "app::db" => DB_SOURCE,
            _ => return None,
        };
        Some(Source::new(path.to_owned(), text))
    }

    fn libraries(&self) -> Vec<String> {
        vec!["log".to_owned(), "app::db".to_owned()]
    }
}
```

`exs::tooling::CompletionEngine::with_library(path, source)` (feature `tooling`) offers the public declarations of a library module to editor completions.

### Optional libraries

The `exs` crate ships general library modules with their host functions, one Cargo feature per library:

- `json`: `json::parse(text)`, `json::stringify(value)`, and `json::pretty(value)` convert between values and JSON text; Objects keep their key order.
- `url`: `url::encode_query(values)` and `url::decode_query(text)` handle `application/x-www-form-urlencoded` query strings; `url::encode(text)` and `url::decode(text)` percent-encode single URL parts.
- `rand`: `rand::int(min, max)`, `rand::float(min, max)`, `rand::text(length)`, and `rand::bytes(length)` return values from the platform's secure random source (the operating system, or `crypto.getRandomValues` in browsers).
- `http`: `http::get`, `post`, `put`, `patch`, `delete`, and `request(method, url)` take an optional `{ headers, body, timeout }` Object and return an `http::Response` with `status`, `headers`, `body`, `ok()`, `text()`, and `json()`. Responses are limited to 10 MiB and 5 redirects. Requests go through an `exs::libs::http::HttpTransport`: `reqwest` on native targets, which needs a tokio runtime, and `fetch` in browsers. `exs::libs::register_with_http` replaces it, for example to send browser requests through the application's server, which can send them with `exs::libs::http::send`.

An application enables them in two lines: `exs::libs::add_to(&mut sources)` adds them to its `Sources`, or its own resolver returns `exs::libs::library(path)` from `library` and `exs::libs::libraries()` from `libraries`; and its runner calls `exs::libs::register(runner.registry_mut())`. The `exs` CLI enables `json`, `url`, and `rand`.

### Documentation

`exs::tooling` (feature `tooling`) provides this. `document` documents the standard library, a project, and the libraries it names; `document_libraries` documents the standard library and every library `libraries()` lists. Both return an `ApiDocumentation` model. `markdown(&MarkdownOptions)` renders it as pages, one per module by default or one per type, enum, trait, and namespace with `PageLayout::PerItem`; the options also set the index file name, an absolute link prefix, and a front-matter callback for static-site generators such as Zola. `llm()` renders the language specification and a compact API reference as one Markdown document. `exs docs app.exs -o docs` writes the pages and `exs docs app.exs -o api.md --llm` the compact reference.

```rust
let front_matter = |page: &PageInfo<'_>| format!("+++\ntitle = \"{}\"\n+++\n\n", page.title);
let mut options = MarkdownOptions::default();
options.layout = PageLayout::PerItem;
options.index_file = "_index.md";
options.link_prefix = Some("@/exs/");
options.title_heading = false;
options.front_matter = Some(&front_matter);
let pages = document_libraries(CompileOptions::default().resolver(&resolver))?.markdown(&options);
```

### Server runner

`Runner::new` takes a `RunnerConfig` (`ServerRunner` and `ServerRunnerConfig` in `exs-runner`) with the memory, fuel and wall-clock limits, the protection level for host-side resources, the Wasmtime compiler strategy and the module cache size. Every field has a default suited to untrusted source, which an application overrides on the value `RunnerConfig::default()` returns. Applications that create a runner per execution share compiled modules through one `SharedModuleCache`, created with the same configuration and passed as `module_cache`; each execution still gets its own store, limits and host functions.

`execute` is an ordinary future that works with any executor; `execute_with` also takes `ExecutionOptions`, such as an `ExecutionCancellation`, and returns an `ExecutionReport` with the consumed fuel and times. Running guest code yields to the executor every 100,000 fuel, so a busy script does not block the executor thread, and it stops at its wall-clock limit or cancellation. `Host::sleep` and the deadline of an execution that waits for a host operation are woken by the runner's timer. An application passes the timer of its async runtime through `ServerRunnerConfig::timer`, implementing `RunnerTimer`:

```rust
struct TokioTimer;

impl RunnerTimer for TokioTimer {
    fn sleep_until(&self, deadline: Instant) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(tokio::time::sleep_until(deadline.into()))
    }
}

let mut config = RunnerConfig::default();
config.timer = Some(Arc::new(TokioTimer));
let runner = Runner::new(config);
```

Without a timer, the runner starts one process-wide timer thread on first use, which serves every execution; an execution never starts a thread of its own.

```rust
let mut config = RunnerConfig::default();
config.max_fuel = 50_000_000;
config.timeout = Duration::from_secs(30);
let mut runner = Runner::new(config);
runner.registry_mut().fn_sync_raw("log", |arguments| ExsValue::None)?;
let result = runner.execute(&module.wasm, "main", &inputs).await?;
```

The runner executes every module whose ABI version lies between `MIN_SUPPORTED_ABI_VERSION` and `ABI_VERSION`, compiled from ExS or built with `exs-guest`; another version fails with `RunnerError::AbiMismatch`.

### Rust guests

`exs-guest` builds extensions in Rust: `async` entry points named with `exs_guest::export!`, compiled as a `cdylib` for `wasm32-unknown-unknown`, and run by any runner like a compiled script. An entry point returns `ExsValue` or `Result<ExsValue, ExsError>`, and `exs_guest::host` calls host functions, reads time, sleeps and consumes streams. The `serde` feature adds `decode` and `encode` for Rust types with the Serde traits. Without the default `std` feature a guest is `no_std`, and the crate supplies its allocator and panic handler.

```rust
async fn log(inputs: Vec<ExsValue>) -> Result<ExsValue, ExsError> {
    let now = host::now().await?;
    host::call("log", inputs).await;
    Ok(ExsValue::Int(now.unix_seconds))
}

exs_guest::export!(log);
```

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
exs-guest = "0.2"
```

```bash
cargo build --release --target wasm32-unknown-unknown
```

### Browser runner

The `exs` crate enables the Wasmtime-backed `server` feature by default. A Rust-Wasm browser application uses the browser-only backend without compiling Wasmtime; `exs::Runner` has the same methods on both:

```toml
exs = { version = "0.2", default-features = false, features = ["browser", "compiler"] }
```

```rust
let mut runner = Runner::default();
runner.registry_mut().fn_sync_raw("log", |arguments| {
    // Call application-owned Rust-Wasm state here.
    ExsValue::None
})?;
let result = runner.execute(&compiled_wasm, "main", &inputs).await?;
```

The browser feature uses the application's `wasm-bindgen` JavaScript glue to instantiate the separate ExS module with native `WebAssembly` APIs. Each `execute` call receives a fresh ExS instance, and the runner keeps the module it compiled last. A cancellation takes effect before the function starts and between host calls. Synchronous host callbacks use the ready path; asynchronous Rust futures resolve through browser Promises and resume the ExS task through the existing Host ABI.

Both runners also support runner-registered pull streams through `Host::stream(name, arguments...)`. Native registrations implement `HostStream`; browser registrations implement `BrowserHostStream`. Stream state is isolated to each execution and released on completion, cancellation, or `IteratorStep::Done`.

`BrowserRunner` executes Wasm on the calling JavaScript thread and does not enforce fuel, wall-clock, memory, task, or host-call limits. It must therefore run untrusted ExS source inside a dedicated Web Worker. The embedding application owns the Worker lifecycle and should terminate it when its resource policy is exceeded; executing untrusted code on the main thread can freeze the user interface.

## Architecture

```text
ExS source -> compiler + embedded exs-core-template.wasm -> final Wasm module
                                                               |
                                                     server or browser runner
                                                               |
                                                              host
```

Values live in the module's linear memory in the layout of `exs-layout`. The compiler generates the program's code and links the stateless functions of the core library (`exs-core`, embedded as `exs-core-template.wasm`) into the final Wasm module. A runner executes outside the module, supplies the Host ABI, and enforces execution limits. Every value crosses the boundary as an image, for compiled modules and Rust guests built with `exs-guest` alike: the module adopts inputs and host responses without parsing and writes results and host-call arguments itself, so the runner never reads the module's heap.

- **Values:** a `ValueRef` is 64 bits wide; None, Bool, and Ints in the `i32` range are immediates, every other value lives on a non-moving heap in linear memory. Untyped Objects share hidden-class shapes; nominal types use fixed field layouts.
- **Memory:** a stop-the-world mark-and-sweep collector in `exs-core` finds live values from precise root frames the compiler emits. When memory growth reaches the runner's limit, it collects and retries before the execution fails.
- **Tasks:** functions that may suspend compile to resumable step functions with frames in linear memory. The scheduler state of `par` tasks lives in the module's memory and its logic in `exs-core`; a task switches only when suspending or at checkpoints in loops and calls of resumable code.
- **Boundary:** inputs, host responses, results, and host-call arguments are acyclic by-value images (`exs_abi::image`), bounded by `BoundaryLimits`. Values of nominal Object types cross as `ExsValue::Typed` with their type's identity, such as `app::dom::Div`, and plain Objects as `ExsValue::Object`; a Typed host value becomes a value of its type when the program uses the type, and otherwise an Object that keeps the identity.

## Development

- Stable Rust is required.
- `wasm-encoder`, `wasmparser`, and `wasmtime` are approved dependencies for the initial implementation.
- `crates/exs-core-template/exs-core-template.wasm` is a committed Rust-compiled artifact embedded by the compiler. Compiler users never build it themselves; after changes to `exs-core` or the template, rebuild it with `nu crates/exs-core-template/build-template.nu`. Builds can differ in their bytes between systems, so a test in `tests/compatibility` compares a hash of the template's sources instead, and fails when the template is out of date.
- `tests/compatibility` runs a corpus of scripts through the current compiler and through the modules frozen by every release, including a Rust guest. A failure there means a change broke existing code or stored modules. `nu tests/compatibility/freeze.nu` freezes the modules of a new version.
- Run `cargo fmt`, `cargo test`, `cargo check`, and `cargo clippy` after Rust changes.

### Open points

- Optimizations: host-name caching, direct-operation specialization, root-frame reduction, and inlining.
- Graph references at the boundary for cyclic and aliased Lists and Objects; host-resource capability values; runner instance pooling.

### Risks to keep in mind

- Every value that survives an allocation must be reachable from a root; each allocating helper must keep that discipline.
- Shapes are never collected, so Objects with many distinct dynamic keys keep their shapes for the rest of the execution.
- The non-moving heap can fragment in long-running tasks.
- Winch generates slower machine code than Cranelift; which compiler production uses is a separate decision.
