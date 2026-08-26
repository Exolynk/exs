//! End-to-end runner benchmark for a basic resumable ExS program.

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::Instant;

use exs_abi::ExsValue;
use exs_compiler::{CompileOptions, SourceInput, compile};
use exs_runner::{ExecutionCancellation, ExecutionLimits, ServerRunner};

const ITERATIONS: u64 = 100;
const LIMIT: i64 = 10_000;

/// Runs the basic script benchmark and reports aggregate runner throughput.
fn main() {
    let compiled = compile_source(
        r#"
        fn main(limit: Int) -> Int {
            let total = 0;
            let index = 0;
            while index < limit {
                total = total + index * 3 + 1;
                index = index + 1;
            }
            Host::call("print", total);
            ret total;
        }
        "#,
    );
    let expected = expected_total(LIMIT);
    let checksum = Arc::new(AtomicU64::new(0));
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    register_print(&mut runner, Arc::clone(&checksum));

    let warmup = execute(&runner, &compiled.wasm);
    assert_eq!(warmup, ExsValue::Int(expected));
    checksum.store(0, Ordering::Relaxed);

    let started = Instant::now();
    for _ in 0..ITERATIONS {
        assert_eq!(execute(&runner, &compiled.wasm), ExsValue::Int(expected));
    }
    let elapsed = started.elapsed();
    let checksum = checksum.load(Ordering::Relaxed);
    assert_eq!(checksum, print_mix(expected).wrapping_mul(ITERATIONS));

    let executions_per_second = f64::from(ITERATIONS as u32) / elapsed.as_secs_f64();
    println!(
        "basic_script: {ITERATIONS} executions in {elapsed:?} ({executions_per_second:.2} executions/s)"
    );
}

/// Compiles the benchmark program once so the measurement excludes source compilation.
fn compile_source(source: &str) -> exs_compiler::CompiledModule {
    match compile(
        SourceInput {
            source_id: "basic_script.exs",
            text: source,
        },
        CompileOptions::default(),
    ) {
        Ok(module) => module,
        Err(error) => panic!("benchmark compilation failed: {error}"),
    }
}

/// Registers a synchronous print host call that performs deterministic checksum work.
fn register_print(runner: &mut ServerRunner, checksum: Arc<AtomicU64>) {
    let registration =
        runner
            .registry_mut()
            .register_sync("print", move |arguments: Vec<ExsValue>| {
                let value = match arguments.as_slice() {
                    [ExsValue::Int(value)] => *value,
                    _ => 0,
                };
                checksum.fetch_add(print_mix(value), Ordering::Relaxed);
                ExsValue::None
            });
    assert!(registration.is_ok());
}

/// Executes one already-compiled benchmark module through the public runner path.
fn execute(runner: &ServerRunner, wasm: &[u8]) -> ExsValue {
    let cancellation = ExecutionCancellation::new();
    match block_on(runner.execute(wasm, "main", &[ExsValue::Int(LIMIT)], &cancellation)) {
        Ok(value) => value,
        Err(error) => panic!("benchmark execution failed: {error}"),
    }
}

/// Computes the source program's arithmetic result for the configured loop bound.
fn expected_total(limit: i64) -> i64 {
    3 * limit * (limit - 1) / 2 + limit
}

/// Mixes the host argument into a nontrivial checksum contribution.
fn print_mix(value: i64) -> u64 {
    (value as u64)
        .wrapping_mul(0x9E37_79B1_85EB_CA87)
        .rotate_left(17)
}

/// Polls one immediately-ready runner future without adding an executor dependency.
fn block_on<Output>(future: impl Future<Output = Output>) -> Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}
