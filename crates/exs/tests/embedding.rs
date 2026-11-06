//! Embeds ExS the way an application does, through the public API of the crate alone.

use exs::{
    CompileOptions, ExecutionCancellation, ExecutionOptions, ExsError, ExsValue, ModuleResolver,
    Runner, RunnerConfig, RunnerError, Source, Sources,
};

/// Polls one runner future to completion.
fn block_on<Output>(future: impl std::future::Future<Output = Output>) -> Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        std::thread::yield_now();
    }
}

/// A request a typed host function decodes.
#[derive(serde::Deserialize)]
struct Lookup {
    /// The requested identifier.
    id: i64,
}

/// A response a typed host function encodes.
#[derive(serde::Serialize)]
struct Record {
    /// The name of the record.
    name: String,
}

/// Runs a script calling a typed host function that may fail with a language Error.
#[test]
fn runs_a_script_with_typed_host_functions() {
    let source = r#"
        fn main(id: Int) {
            ret [Host::call("lookup", { id: id }).name, Host::call("lookup", { id: 0 }) is Error];
        }
    "#;
    let module = exs::compile(Source::new("app.exs", source), CompileOptions::default())
        .expect("the script compiles");
    let mut runner = Runner::default();
    runner
        .registry_mut()
        .fn_sync("lookup", |lookup: Lookup| match lookup.id {
            0 => Err(ExsError::recoverable("NotFound", "no such record")),
            id => Ok(Record {
                name: format!("record {id}"),
            }),
        })
        .expect("the name is free");
    let result = block_on(runner.execute(&module.wasm, "main", &[ExsValue::Int(7)]));
    assert_eq!(
        result.expect("the script runs"),
        ExsValue::List(vec![
            ExsValue::String("record 7".to_owned()),
            ExsValue::Bool(true),
        ])
    );
}

/// Compiles a script naming libraries from `Sources` and runs it with their host functions.
#[test]
fn runs_a_project_with_libraries() {
    let mut sources = Sources::new();
    exs::libs::add_to(&mut sources);
    sources.add_library("app", "fn answer() -> Int { ret 42; }");
    let source = r#"fn main() { ret [json::stringify([app::answer()]), json::parse("1")]; }"#;
    let module = exs::compile(
        Source::new("main.exs", source),
        CompileOptions::default().resolver(&sources),
    )
    .expect("the script compiles");
    let mut runner = Runner::default();
    exs::libs::register(runner.registry_mut()).expect("the names are free");
    assert_eq!(
        block_on(runner.execute(&module.wasm, "main", &[])).expect("the script runs"),
        ExsValue::List(vec![ExsValue::String("[42]".to_owned()), ExsValue::Int(1),])
    );
}

/// Loads an import on demand through the application's own resolver.
#[test]
fn compiles_with_an_own_resolver() {
    /// Generates every imported module.
    struct Generated;

    impl ModuleResolver for Generated {
        fn resolve(&self, _importer: &str, path: &str) -> Result<Source<'_>, String> {
            Ok(Source::new(
                path.to_owned(),
                "fn answer() -> Int { ret 42; }".to_owned(),
            ))
        }
    }

    let source = r#"import "./generated.exs" as generated; fn main() { ret generated::answer(); }"#;
    let module = exs::compile(
        Source::new("main.exs", source),
        CompileOptions::default().resolver(&Generated),
    )
    .expect("the script compiles");
    assert_eq!(
        block_on(Runner::default().execute(&module.wasm, "main", &[])).expect("the script runs"),
        ExsValue::Int(42)
    );
}

/// Reports where a script fails to compile.
#[test]
fn reports_compile_errors_with_their_location() {
    let error = exs::compile(
        Source::new("broken.exs", "fn main() { ret missing; }"),
        CompileOptions::default(),
    )
    .expect_err("the script does not compile");
    assert!(error.to_string().contains("broken.exs"), "{error}");
    assert_eq!(error.diagnostics[0].code, "E0205");
    assert!(error.diagnostics[0].start_byte > 0);
}

/// Applies limits from the configuration and stops a cancelled execution.
#[test]
fn applies_limits_and_cancellation() {
    let module = exs::compile(
        Source::new("loop.exs", "fn main() { while true {} }"),
        CompileOptions::default(),
    )
    .expect("the script compiles");
    let mut config = RunnerConfig::default();
    config.max_fuel = 1_000_000;
    let runner = Runner::new(config);
    assert!(matches!(
        block_on(runner.execute(&module.wasm, "main", &[])),
        Err(RunnerError::LimitExceeded(exs::LimitKind::Fuel))
    ));
    let cancellation = ExecutionCancellation::new();
    cancellation.cancel();
    let options = ExecutionOptions::with_cancellation(&cancellation);
    assert!(matches!(
        block_on(runner.execute_with(&module.wasm, "main", &[], &options)),
        Err(RunnerError::Cancelled)
    ));
}

/// Rejects a module built for an ABI outside the range the runner executes.
#[test]
fn reports_an_abi_mismatch() {
    let module = exs::compile(
        Source::new("app.exs", "fn main() { ret 1; }"),
        CompileOptions::default(),
    )
    .expect("the script compiles");
    let mut wasm = module.wasm;
    // The version is the only `i32.const` of the exported `__exs_abi_version` body.
    assert!(patch_abi_version(&mut wasm, 1));
    match block_on(Runner::default().execute(&wasm, "main", &[])) {
        Err(RunnerError::AbiMismatch { module, .. }) => assert_eq!(module, 1),
        other => panic!("unexpected result: {other:?}"),
    }
}

/// Rewrites the version constant of a module to `version`, which must encode in one byte.
fn patch_abi_version(wasm: &mut [u8], version: u8) -> bool {
    let body = [0x41, 29, 0x0b];
    let Some(position) = wasm
        .windows(4)
        .position(|window| window[0] == 4 && window[1] == 0 && window[2..] == body[..2])
    else {
        return false;
    };
    wasm[position + 3] = version;
    true
}
