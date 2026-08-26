//! External public-function execution integration tests.

use exs_compiler::{CompileOptions, SourceInput, compile};
use exs_runner::{ExecutionCancellation, ExecutionLimits, ExsValue, ServerRunner};

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
    let runner = ServerRunner::new(ExecutionLimits::default());
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
    let runner = ServerRunner::new(ExecutionLimits::default());
    let cancellation = ExecutionCancellation::new();
    let result = block_on(runner.execute(&compiled.wasm, "main", &[], &cancellation));
    assert!(matches!(result, Ok(ExsValue::Int(0))));
}
