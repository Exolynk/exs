//! External public-function execution integration tests.

use exs_compiler::{CompileOptions, SourceInput, compile};
use exs_runner::{ExecutionCancellation, ExsValue, ServerRunner};

/// Executes one future to completion without adding a test-only async dependency.
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

/// Executes a public non-main function without evaluating the root main function.
#[test]
fn executes_named_public_function() {
    let source = r#"
        fn main() -> Int { ret 0; }
        fn add(left: Int, right: Int) -> Int { ret left + right; }
        fn delayed() -> Int { Host::sleep(Duration::milliseconds(0)); ret 9; }
        fn value() -> Int { ret 7; }
    "#;
    let compiled = match compile(
        SourceInput {
            source_id: "public-functions.exs",
            text: source,
        },
        CompileOptions::default(),
    ) {
        Ok(compiled) => compiled,
        Err(error) => panic!("could not compile public function fixture: {error}"),
    };
    let runner = ServerRunner::default();
    let cancellation = ExecutionCancellation::new();
    let result = block_on(runner.execute(
        &compiled.wasm,
        "add",
        &[ExsValue::Int(20), ExsValue::Int(22)],
        &cancellation,
    ));
    assert_eq!(
        result.unwrap_or_else(|error| panic!("add execution failed: {error}")),
        ExsValue::Int(42)
    );

    let delayed = block_on(runner.execute(&compiled.wasm, "delayed", &[], &cancellation));
    assert!(matches!(delayed, Ok(ExsValue::Int(9))));

    let value = block_on(runner.execute(&compiled.wasm, "value", &[], &cancellation));
    assert!(matches!(value, Ok(ExsValue::Int(7))));
}

/// Executes a root main function without requiring an explicit visibility modifier.
#[test]
fn executes_root_main_without_visibility_modifier() {
    let source = "fn main() -> Int { ret 0; } fn value() -> Int { ret 42; }";
    let compiled = match compile(
        SourceInput {
            source_id: "private-main.exs",
            text: source,
        },
        CompileOptions::default(),
    ) {
        Ok(compiled) => compiled,
        Err(error) => panic!("could not compile private main fixture: {error}"),
    };
    let runner = ServerRunner::default();
    let cancellation = ExecutionCancellation::new();
    let result = block_on(runner.execute(&compiled.wasm, "main", &[], &cancellation));
    assert!(matches!(result, Ok(ExsValue::Int(0))));
}

/// Returns identical results and fuel whether a module is compiled or taken from the cache.
#[test]
fn reuses_cached_modules_with_identical_results() {
    let compile_text = |text: &str| match compile(
        SourceInput {
            source_id: "test.exs",
            text,
        },
        CompileOptions::default(),
    ) {
        Ok(module) => module.wasm,
        Err(error) => panic!("compilation failed: {error}"),
    };
    let doubled = compile_text("fn main(input: Int) -> Int { ret input * 2; }");
    let greeted = compile_text(r#"fn main(input: String) -> String { ret "hi " + input; }"#);
    let cases = [
        (&doubled, ExsValue::Int(21), ExsValue::Int(42)),
        (
            &greeted,
            ExsValue::String("ExS".to_owned()),
            ExsValue::String("hi ExS".to_owned()),
        ),
    ];
    let uncached = ServerRunner::default().with_module_cache_capacity(0);
    // A single cache slot makes the alternating modules evict each other every time.
    for runner in [
        ServerRunner::default(),
        ServerRunner::default().with_module_cache_capacity(1),
    ] {
        for _ in 0..3 {
            for (wasm, input, expected) in &cases {
                let execute = |runner: &ServerRunner| match block_on(runner.execute_with_report(
                    wasm,
                    "main",
                    std::slice::from_ref(input),
                    &ExecutionCancellation::new(),
                )) {
                    Ok(report) => report,
                    Err(error) => panic!("execution failed: {error}"),
                };
                let cached = execute(&runner);
                let reference = execute(&uncached);
                assert_eq!(&cached.value, expected);
                assert_eq!(cached.value, reference.value);
                assert_eq!(cached.fuel_consumed, reference.fuel_consumed);
            }
        }
    }
}
