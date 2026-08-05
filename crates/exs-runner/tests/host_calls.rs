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
    let result = match block_on(runner.execute(&compiled.wasm, &[ExsValue::Int(42)], &cancellation))
    {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(42));
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
        &[ExsValue::String("Ada".to_owned())],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::String("Ada".to_owned()));
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
