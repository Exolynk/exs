//! Compares the browser runner's results, written by `check.nu`, with the server runner's.

use std::time::Duration;

use exs_browser_check::cases::{CANCELLED_SOURCE, CASES, GUEST_CASES, add, counter, echo, line};
use exs_compiler::{CompileOptions, Source};
use exs_runner::{
    ExecutionCancellation, ExecutionOptions, ExsValue, HostStream, HostStreamFuture,
    HostStreamItem, ProtectionLevel, ServerRunner, ServerRunnerConfig,
};

/// The host stream `counter` on the server.
struct Counter(std::vec::IntoIter<ExsValue>);

impl HostStream for Counter {
    fn next(&mut self) -> HostStreamFuture {
        let item = self
            .0
            .next()
            .map_or(HostStreamItem::End, HostStreamItem::Item);
        Box::pin(std::future::ready(item))
    }
}

/// Polls one runner future to completion.
fn block_on<Output>(future: impl std::future::Future<Output = Output>) -> Output {
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

/// Compiles `source`.
fn compile(source: &str) -> Result<Vec<u8>, String> {
    exs_compiler::compile(Source::new("test.exs", source), CompileOptions::default())
        .map(|module| module.wasm)
        .map_err(|error| error.to_string())
}

/// Returns a server runner with the host functions of the cases.
fn runner() -> Result<ServerRunner, String> {
    let mut runner = ServerRunner::new({
        let mut options = ServerRunnerConfig::default();
        options.max_memory_bytes = 64 << 20;
        options.max_fuel = 1_000_000_000;
        options.timeout = Duration::from_secs(60);
        options.protection = ProtectionLevel::Standard;
        options
    });
    let registry = runner.registry_mut();
    registry
        .fn_sync_raw("add", |arguments: Vec<ExsValue>| add(&arguments))
        .map_err(|error| error.to_string())?;
    registry
        .fn_sync_raw("println", |_: Vec<ExsValue>| ExsValue::None)
        .map_err(|error| error.to_string())?;
    registry
        .fn_async_raw(
            "echo",
            |arguments: Vec<ExsValue>| async move { echo(arguments) },
        )
        .map_err(|error| error.to_string())?;
    registry
        .stream_raw("counter", |arguments: Vec<ExsValue>| {
            Ok(Counter(counter(&arguments).into_iter()))
        })
        .map_err(|error| error.to_string())?;
    Ok(runner)
}

/// Runs the entry `function` of the module `wasm` in the server runner.
fn run(wasm: &[u8], function: &str, inputs: &[ExsValue]) -> Result<ExsValue, String> {
    block_on(runner()?.execute(wasm, function, inputs)).map_err(|error| error.to_string())
}

/// Runs the module `wasm` until its host function `stop` cancels the execution.
fn cancelled(wasm: &[u8]) -> Result<ExsValue, String> {
    let cancellation = ExecutionCancellation::new();
    let stop = cancellation.clone();
    let mut runner = runner()?;
    runner
        .registry_mut()
        .fn_async_raw("stop", move |_: Vec<ExsValue>| {
            stop.cancel();
            async { ExsValue::None }
        })
        .map_err(|error| error.to_string())?;
    block_on(runner.execute_with(
        wasm,
        "main",
        &[],
        &ExecutionOptions::with_cancellation(&cancellation),
    ))
    .map(|report| report.value)
    .map_err(|error| error.to_string())
}

/// Compares every line the browser runner wrote with the server runner's result; the Rust
/// guest is the module at `EXS_RUST_GUEST_WASM`.
#[test]
fn browser_matches_server() {
    let Ok(path) = std::env::var("EXS_BROWSER_RESULTS") else {
        eprintln!("EXS_BROWSER_RESULTS is not set; run check.nu to compare with the browser");
        return;
    };
    let browser = std::fs::read_to_string(path).expect("browser results");
    let guest = std::env::var("EXS_RUST_GUEST_WASM").expect("Rust guest path");
    let guest = std::fs::read(guest).expect("Rust guest module");
    let mut browser = browser.lines();
    for case in CASES {
        let result = compile(case.source).and_then(|wasm| run(&wasm, "main", &(case.inputs)()));
        let expected = line(case.name, "exs", &result);
        assert_eq!(browser.next(), Some(expected.as_str()));
    }
    for case in GUEST_CASES {
        let expected = line(
            case.entry,
            "rust",
            &run(&guest, case.entry, &(case.inputs)()),
        );
        assert_eq!(browser.next(), Some(expected.as_str()));
    }
    let result = compile(CANCELLED_SOURCE).and_then(|wasm| cancelled(&wasm));
    assert_eq!(
        browser.next(),
        Some(line("cancelled", "exs", &result).as_str())
    );
    assert_eq!(browser.next(), None);
}
