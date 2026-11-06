//! ExS is an extension language for applications: scripts compile to WebAssembly modules that
//! run sandboxed on a server or in the browser and reach the application only through the host
//! functions it registers.
//!
//! ```
//! use exs::{CompileOptions, ExsValue, Runner, Source};
//!
//! # fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
//! #     let mut future = std::pin::pin!(future);
//! #     let mut context = std::task::Context::from_waker(std::task::Waker::noop());
//! #     loop {
//! #         if let std::task::Poll::Ready(output) = future.as_mut().poll(&mut context) {
//! #             return output;
//! #         }
//! #     }
//! # }
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let source = r#"
//!     fn main(name: String) -> String {
//!         ret Host::call("greet", name);
//!     }
//! "#;
//! let module = exs::compile(Source::new("app.exs", source), CompileOptions::default())?;
//!
//! let mut runner = Runner::default();
//! runner
//!     .registry_mut()
//!     .fn_sync("greet", |name: String| Ok(format!("Hello, {name}!")))?;
//!
//! let inputs = [ExsValue::String("Ada".to_owned())];
//! let result = block_on(runner.execute(&module.wasm, "main", &inputs))?;
//! assert_eq!(result, ExsValue::String("Hello, Ada!".to_owned()));
//! # Ok(())
//! # }
//! ```
//!
//! # Compatibility
//!
//! A runner executes every module built since 0.2.0, whichever language it comes from, and scripts
//! that compile keep compiling. Only this crate and `exs-guest` carry this promise, not the
//! `exs-*` crates behind them. `COMPATIBILITY.md` in the repository has the details.
//!
//! # Features
//!
//! - `compiler` (default): compiles source to modules.
//! - `server` (default): [`Runner`] with Wasmtime, enforcing memory, fuel and time limits.
//! - `browser`: [`Runner`] through wasm-bindgen on `wasm32`. It enforces no limits, so run
//!   untrusted scripts in a dedicated Web Worker.
//! - `serde` (default): typed host functions and value conversions.
//! - `tooling`: the `tooling` module for editors and documentation generators.
//! - `json`, `url`, `rand`, `http`: the library modules in `libs`.

pub use exs_abi::value::ExsStackFrame;
pub use exs_abi::{Bytes, ErrorSeverity, ExsError, ExsValue, SourcePositionId};
#[cfg(feature = "compiler")]
pub use exs_compiler::{
    CompileError, CompileLimits, CompileOptions, CompiledModule, CompiledTest, CompiledTests,
    ModuleResolver, Source, SourceDiagnostic, Sources, check_syntax, compile, compile_tests,
    format, test_descriptions,
};

#[cfg(feature = "serde")]
pub use exs_abi::SerdeError;

#[cfg(any(
    all(feature = "server", not(target_arch = "wasm32")),
    all(feature = "browser", target_arch = "wasm32")
))]
pub use exs_runner::{
    ExecutionCancellation, ExecutionOptions, ExecutionReport, LimitKind, RunnerError,
};

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use exs_runner::{
    AsyncHostFunction, CompilerStrategy, HostFunctionRegistry as Registry, HostFuture, HostStream,
    HostStreamFunction, HostStreamFuture, HostStreamItem, ProtectionLevel, RegistryError,
    RunnerTimer, ServerRunner as Runner, ServerRunnerConfig as RunnerConfig, SharedModuleCache,
    SyncHostFunction,
};

#[cfg(all(feature = "browser", target_arch = "wasm32"))]
pub use exs_runner::{
    BrowserHostFunctionRegistry as Registry, BrowserHostStream as HostStream,
    BrowserHostStreamFunction as HostStreamFunction, BrowserHostStreamFuture as HostStreamFuture,
    BrowserHostStreamItem as HostStreamItem, BrowserRegistryError as RegistryError,
    BrowserRunner as Runner, BrowserRunnerConfig as RunnerConfig,
};

/// Syntax highlighting, editor completions and API documentation.
#[cfg(feature = "tooling")]
pub mod tooling {
    pub use exs_autocomplete::{
        CompletionEngine, CompletionItem, CompletionKind, CompletionRequest, CompletionResponse,
    };
    pub use exs_compiler::{
        ApiDocumentation, DebugInfoError, Documentation, DocumentationPage, EmbeddedSource,
        FunctionDebugInfo, HighlightKind, HighlightSpan, ItemDocs, ItemKind, ItemRef,
        MarkdownOptions, MemberDocs, ModuleDebugInfo, ModuleDocs, ModuleOrigin, PageInfo, PageKind,
        PageLayout, SectionDocs, SourceComment, SourceLex, SourcePosition, SourceToken,
        SourceTokenKind, document, document_libraries, highlight, read_debug_info, source_lex,
        tokens,
    };
}

/// The optional library modules scripts can name without imports, such as `json::parse(text)`.
///
/// An application makes the enabled ones available in two places: `add_to` adds them to its
/// `Sources` (its own [`ModuleResolver`] can return `library(path)` instead), and its runner
/// registers their host functions with `register`.
#[cfg(any(feature = "json", feature = "url", feature = "rand", feature = "http"))]
pub mod libs {
    #[cfg(feature = "http")]
    pub use exs_libs::http;
    #[cfg(any(
        all(feature = "server", not(target_arch = "wasm32")),
        all(feature = "browser", target_arch = "wasm32")
    ))]
    pub use exs_libs::register;
    #[cfg(all(
        feature = "http",
        any(
            all(feature = "server", not(target_arch = "wasm32")),
            all(feature = "browser", target_arch = "wasm32")
        )
    ))]
    pub use exs_libs::register_with_http;
    #[cfg(feature = "compiler")]
    pub use exs_libs::{add_to, library};
    pub use exs_libs::{libraries, source};
}
