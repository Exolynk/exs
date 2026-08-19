//! Basic host-call, cancellation, and runner-deadlock integration tests.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Poll;

use exs_abi::{ErrorSeverity, ExsValue};
use exs_runner::{ExecutionCancellation, ExecutionLimits, RunnerError, ServerRunner};
use support::{block_on, compile_source, execute_source, execute_source_with_inputs};

/// Executes an arithmetic result through the linked runtime.
#[test]
fn executes_compiled_integer_program() {
    assert_eq!(
        execute_source(
            "fn main(input) { let value = 40 + 2; ret value; }",
            ExsValue::None
        ),
        ExsValue::Int(42)
    );
}

/// Packs root and direct-call variadic arguments into Lists visible to ExS bodies.
#[test]
fn executes_variadic_functions() {
    let source = "fn total(values: Int...) -> Int { let sum = 0; for value in values { sum = sum + value; } ret sum; } fn main(inputs: Int...) -> Int { ret total(inputs[0], inputs[1], inputs[2]); }";
    assert_eq!(
        execute_source_with_inputs(
            source,
            &[ExsValue::Int(10), ExsValue::Int(20), ExsValue::Int(12)],
        ),
        ExsValue::Int(42)
    );
}

/// Invokes a variadic closure through dynamic callable dispatch.
#[test]
fn executes_variadic_closures() {
    let source = "fn main() -> Int { let total = (values...) => { let sum = 0; for value in values { sum = sum + value; } ret sum; }; ret total(10, 20, 12); }";
    assert_eq!(execute_source_with_inputs(source, &[]), ExsValue::Int(42));
}

/// Dispatches a variadic trait method through its nominal implementation.
#[test]
fn executes_variadic_instance_methods() {
    let source = "type Total {} trait Sum { fn sum(self, values: Int...) -> Int; } impl Sum for Total { fn sum(self, values: Int...) -> Int { let total = 0; for value in values { total = total + value; } ret total; } } fn main() -> Int { ret Total {}.sum(10, 20, 12); }";
    assert_eq!(execute_source_with_inputs(source, &[]), ExsValue::Int(42));
}

/// Dispatches a variadic static method through its nominal implementation.
#[test]
fn executes_variadic_static_methods() {
    let source = "type Total {} impl Total { fn sum(values: Int...) -> Int { let total = 0; for value in values { total = total + value; } ret total; } } fn main() -> Int { ret Total::sum(10, 20, 12); }";
    assert_eq!(execute_source_with_inputs(source, &[]), ExsValue::Int(42));
}

/// Resumes a host call evaluated inside a formatted string interpolation.
#[test]
fn executes_host_calls_inside_formatted_strings() {
    let compiled = compile_source(
        r#"fn main(input: Int) -> String { ret f"value: {host.call("echo", input)}"; }"#,
    );
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .register_sync("echo", |arguments: Vec<ExsValue>| arguments
                .into_iter()
                .next()
                .unwrap_or(ExsValue::None))
            .is_ok()
    );
    let result = match block_on(runner.execute(
        &compiled.wasm,
        &[ExsValue::Int(42)],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::String("value: 42".to_owned()));
}

/// Resumes a `ToString` implementation selected by formatted string interpolation.
#[test]
fn executes_host_calls_inside_to_string_implementations() {
    let compiled = compile_source(
        r#"
        type Remote {}

        impl ToString for Remote {
            fn to_string(self) -> String {
                ret host.call("echo", "custom");
            }
        }

        fn main() -> String {
            ret f"value: {Remote {}}";
        }
        "#,
    );
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .register_sync("echo", |arguments: Vec<ExsValue>| arguments
                .into_iter()
                .next()
                .unwrap_or(ExsValue::None))
            .is_ok()
    );
    let result = match block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()))
    {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::String("value: custom".to_owned()));
}

/// Preserves packed variadic arguments across resumable function, closure, and method calls.
#[test]
fn executes_resumable_variadic_calls() {
    let compiled = compile_source(
        r#"
        type Total {}
        trait Sum {
            fn sum(self, values: Int...) -> Int;
        }
        impl Sum for Total {
            fn sum(self, values: Int...) -> Int {
                ret host.call("echo", values[0]) + values[1];
            }
        }
        fn echo_first(values: Int...) -> Int {
            ret host.call("echo", values[0]);
        }
        fn main() -> Int {
            let from_function = echo_first(40, 0);
            let echo = (values...) => { ret host.call("echo", values[0]); };
            let from_closure = echo(from_function);
            ret Total {}.sum(from_closure, 2);
        }
        "#,
    );
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .register_sync("echo", |arguments: Vec<ExsValue>| arguments
                .into_iter()
                .next()
                .unwrap_or(ExsValue::None))
            .is_ok()
    );
    let result = match block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()))
    {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(42));
}

/// Starts variadic closures without arguments through dynamic parallel execution.
#[test]
fn executes_zero_argument_variadic_parallel_closures() {
    let source = "fn main() -> Int { let closure = (values...) => { ret values.length(); }; let results = par([closure]); ret results[0]; }";
    assert_eq!(execute_source_with_inputs(source, &[]), ExsValue::Int(0));
}

/// Delivers a synchronous dynamic Host ABI lookup through the resumable main frame.
#[test]
fn returns_a_language_error_for_an_unregistered_dynamic_host_function() {
    let result = execute_source_with_inputs(
        "fn main(input) { ret host.call(\"missing\", input); }",
        &[ExsValue::Int(7)],
    );
    let ExsValue::Error(error) = result else {
        panic!("expected a language Error result");
    };
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
    assert_eq!(error.kind, "HostFunctionNotFound");
    assert!(error.origin.is_some());
}

/// Uses the host fast path without suspending the generated resumable frame.
#[test]
fn executes_a_synchronous_dynamic_host_function() {
    let compiled = compile_source("fn main(input) { ret host.call(\"echo\", input); }");
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .register_sync("echo", |arguments: Vec<ExsValue>| arguments
                .into_iter()
                .next()
                .unwrap_or(ExsValue::None))
            .is_ok()
    );
    let cancellation = ExecutionCancellation::new();
    let result =
        match block_on(runner.execute(&compiled.wasm, &[ExsValue::Int(i64::MAX)], &cancellation)) {
            Ok(result) => result,
            Err(error) => panic!("execution failed: {error}"),
        };
    assert_eq!(result, ExsValue::Int(i64::MAX));
}

/// Rejects non-serializable host arguments without invoking the registered host function.
#[test]
fn rejects_unserializable_host_call_arguments() {
    for source in [
        r#"
        fn main() -> Error {
            let callback = () => { ret 1; };
            ret host.call("save", callback);
        }
        "#,
        r#"
        fn main() -> Error {
            let cycle = [];
            cycle.push(cycle);
            ret host.call("save", cycle);
        }
        "#,
    ] {
        let compiled = compile_source(source);
        let calls = Arc::new(AtomicUsize::new(0));
        let mut runner = ServerRunner::new(ExecutionLimits::default());
        assert!(
            runner
                .registry_mut()
                .register_sync("save", {
                    let calls = Arc::clone(&calls);
                    move |_| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        ExsValue::None
                    }
                })
                .is_ok()
        );
        let result =
            match block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new())) {
                Ok(result) => result,
                Err(error) => panic!("execution failed: {error}"),
            };
        let ExsValue::Error(error) = result else {
            panic!("unserializable host arguments did not return an Error");
        };
        assert_eq!(error.severity, ErrorSeverity::Recoverable);
        assert_eq!(error.kind, "SerializationError");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

/// Suspends and resumes the same generated frame for an asynchronous host function.
#[test]
fn executes_an_asynchronous_dynamic_host_function() {
    let compiled = compile_source("fn main(input) { ret host.call(\"echo\", input); }");
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .register_async("echo", |arguments: Vec<ExsValue>| async move {
                arguments.into_iter().next().unwrap_or(ExsValue::None)
            })
            .is_ok()
    );
    let result = match block_on(runner.execute(
        &compiled.wasm,
        &[ExsValue::Int(i64::MIN)],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(i64::MIN));
}

/// Cancels a suspended execution and invalidates its pending host-call continuation.
#[test]
fn cancels_a_pending_host_execution() {
    let compiled = compile_source("fn main() { ret host.call(\"wait\"); }");
    let cancellation = ExecutionCancellation::new();
    let cancellation_for_host = cancellation.clone();
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .register_async("wait", move |_arguments: Vec<ExsValue>| {
                let cancellation = cancellation_for_host.clone();
                std::future::poll_fn(move |_| {
                    cancellation.cancel();
                    Poll::Pending
                })
            })
            .is_ok()
    );
    let result = block_on(runner.execute(&compiled.wasm, &[], &cancellation));
    assert!(matches!(result, Err(RunnerError::Cancelled)));
}

/// Cancels a host call that is suspended inside a dynamically invoked closure frame.
#[test]
fn cancels_a_pending_host_call_inside_a_closure() {
    let compiled = compile_source(
        r#"
        fn main() {
            let wait = () => { ret host.call("wait"); };
            ret wait();
        }
        "#,
    );
    let cancellation = ExecutionCancellation::new();
    let cancellation_for_host = cancellation.clone();
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .register_async("wait", move |_arguments: Vec<ExsValue>| {
                let cancellation = cancellation_for_host.clone();
                std::future::poll_fn(move |_| {
                    cancellation.cancel();
                    Poll::Pending
                })
            })
            .is_ok()
    );
    let result = block_on(runner.execute(&compiled.wasm, &[], &cancellation));
    assert!(matches!(result, Err(RunnerError::Cancelled)));
}

/// Reports a scheduler deadlock when Wasm suspends without a runner host future.
#[test]
fn reports_pending_execution_without_host_future_as_deadlock() {
    let wasm = format!(
        r#"
        (module
            (import "runner" "__runner_task_acquire" (func (result i32)))
            (import "runner" "__runner_task_release" (func (result i32)))
            (memory (export "memory") 1)
            (func (export "__exs_abi_version") (result i32)
                i32.const {})
            (func (export "__exs_input_alloc") (param i32) (result i32)
                i32.const 0)
            (func (export "__exs_start") (param i32 i32) (result i32)
                i32.const 1)
        )
        "#,
        exs_abi::ABI_VERSION
    );
    let cancellation = ExecutionCancellation::new();
    let runner = ServerRunner::new(ExecutionLimits::default());
    let result = block_on(runner.execute(wasm.as_bytes(), &[], &cancellation));
    assert!(
        matches!(result, Err(RunnerError::Deadlock(message)) if message.contains("without a runner host future"))
    );
}
