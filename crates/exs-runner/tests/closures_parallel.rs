//! Closure-capture, dynamic-function, and parallel-task integration tests.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Poll;

use exs_abi::{ErrorSeverity, ExsValue};
use exs_runner::{ExecutionCancellation, ExecutionLimits, ServerRunner};
use support::{block_on, compile_source, execute_source, execute_source_with_inputs};

/// Executes a closure passed through an unparameterized Fn contract.
#[test]
fn executes_a_closure_argument_with_a_captured_binding() {
    let result = execute_source(
        r#"
        fn apply(function: Fn, value: Int) -> Int {
            ret function(value);
        }
        fn main(input: Int) -> Int {
            let offset = 2;
            let add = (value) => { ret value + offset; };
            ret apply(add, input);
        }
        "#,
        ExsValue::Int(40),
    );
    assert_eq!(result, ExsValue::Int(42));
}

/// Executes static `par` task closures and retains their source-order results.
#[test]
fn executes_static_parallel_tasks_in_source_order() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            fn main() -> List {
                ret par {
                    6 * 7;
                    20 + 1;
                };
            }
            "#,
            &[],
        ),
        ExsValue::List(vec![ExsValue::Int(42), ExsValue::Int(21)])
    );
}

/// Reuses completed child frames across sequential parallel blocks.
#[test]
fn reuses_parallel_child_frames_within_a_constrained_memory_budget() {
    let compiled = compile_source(
        r#"
        fn main() -> Int {
            let count = 0;
            while count < 20_000 {
                par { 1; };
                count = count + 1;
            }
            ret count;
        }
        "#,
    );
    let runner = ServerRunner::new(ExecutionLimits {
        max_memory_bytes: 2 * 1024 * 1024,
        max_fuel: u64::MAX,
        ..ExecutionLimits::default()
    });
    let result =
        block_on(runner.execute(&compiled.wasm, "main", &[], &ExecutionCancellation::new()));
    match result {
        Ok(result) => assert_eq!(result, ExsValue::Int(20_000)),
        Err(error) => panic!("execution failed: {error}"),
    }
}

/// Preserves raw literal bytes and removes shared indentation from dedented multiline literals.
#[test]
fn executes_hash_delimited_multiline_strings() {
    assert_eq!(
        execute_source_with_inputs(
            r###"
            fn main() -> List {
                let raw = r##"first
  "# remains raw
last"##;
                let dedented = d#"
                    first
                      second
                "#;
                ret [raw, dedented];
            }
            "###,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::String("first\n  \"# remains raw\nlast".to_owned()),
            ExsValue::String("first\n  second".to_owned()),
        ])
    );
}

/// Preserves a recoverable task Error while allowing static `par` siblings to finish.
#[test]
fn continues_parallel_siblings_after_a_recoverable_error() {
    let result = execute_source_with_inputs(
        r#"
        fn main() -> List {
            ret par {
                1 + "invalid";
                6 * 7;
            };
        }
        "#,
        &[],
    );
    let ExsValue::List(values) = result else {
        panic!("parallel execution did not return a List");
    };
    assert!(matches!(values.first(), Some(ExsValue::Error(_))));
    assert_eq!(values.get(1), Some(&ExsValue::Int(42)));
}

/// Terminates the root execution when one parallel child returns a fatal Error.
#[test]
fn terminates_parallel_execution_after_a_fatal_contract_failure() {
    let result = execute_source_with_inputs(
        r#"
        fn wrong() -> Int { ret "invalid"; }
        fn main() -> List {
            ret par {
                wrong();
                6 * 7;
            };
        }
        "#,
        &[],
    );
    let ExsValue::Error(error) = result else {
        panic!("fatal parallel contract failure did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Fatal);
    assert_eq!(error.kind, "TypeError");
}

/// Executes each closure supplied by dynamic `par(list)` and retains source order.
#[test]
fn executes_dynamic_parallel_closure_lists() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            fn main() -> List {
                let tasks = [() => { ret 40 + 2; }, () => { ret 20 + 1; }];
                ret par(tasks);
            }
            "#,
            &[],
        ),
        ExsValue::List(vec![ExsValue::Int(42), ExsValue::Int(21)])
    );
}

/// Polls every pending parallel host future so one task cannot block its siblings.
#[test]
fn polls_parallel_host_calls_concurrently() {
    let compiled = compile_source(
        r#"
        fn main() -> List {
            ret par {
                Host::call("wait");
                Host::call("wait");
            };
        }
        "#,
    );
    let polls = Arc::new(AtomicUsize::new(0));
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .register_async("wait", {
                let polls = Arc::clone(&polls);
                move |_arguments: Vec<ExsValue>| {
                    let polls = Arc::clone(&polls);
                    std::future::poll_fn(move |_| {
                        if polls.fetch_add(1, Ordering::SeqCst) + 1 >= 2 {
                            Poll::Ready(ExsValue::Int(7))
                        } else {
                            Poll::Pending
                        }
                    })
                }
            })
            .is_ok()
    );
    let result = match block_on(runner.execute(
        &compiled.wasm,
        "main",
        &[],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(polls.load(Ordering::SeqCst), 3);
    assert_eq!(
        result,
        ExsValue::List(vec![ExsValue::Int(7), ExsValue::Int(7)])
    );
}

/// Preserves a captured binding's shared Cell across repeated closure assignments.
#[test]
fn preserves_mutation_shared_by_a_closure() {
    let result = execute_source_with_inputs(
        r#"
        fn main() -> Int {
            let count = 0;
            let increment = () => {
                count = count + 1;
                ret count;
            };
            increment();
            ret increment();
        }
        "#,
        &[],
    );
    assert_eq!(result, ExsValue::Int(2));
}

/// Returns a recoverable Error when a closure is called with too few arguments.
#[test]
fn rejects_too_few_dynamic_closure_arguments() {
    let result = execute_source_with_inputs(
        r#"
        fn main() -> Error {
            let identity = (value) => { ret value; };
            ret identity();
        }
        "#,
        &[],
    );
    let ExsValue::Error(error) = result else {
        panic!("missing closure argument did not return an Error");
    };
    assert_eq!(error.kind, "ArityError");
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
}

/// Returns a recoverable Error when a closure is called with too many arguments.
#[test]
fn rejects_too_many_dynamic_closure_arguments() {
    let result = execute_source_with_inputs(
        r#"
        fn main() -> Error {
            let identity = (value) => { ret value; };
            ret identity(1, 2);
        }
        "#,
        &[],
    );
    let ExsValue::Error(error) = result else {
        panic!("excess closure arguments did not return an Error");
    };
    assert_eq!(error.kind, "ArityError");
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
}

/// Rejects a non-callable source value passed through an Fn function contract.
#[test]
fn rejects_non_function_fn_contract_arguments() {
    let result = execute_source_with_inputs(
        r#"
        fn accept(callback: Fn) -> None | Error {
            ret None;
        }
        fn main() -> Error {
            ret accept(1);
        }
        "#,
        &[],
    );
    let ExsValue::Error(error) = result else {
        panic!("non-function Fn argument did not return an Error");
    };
    assert_eq!(error.kind, "TypeError");
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
}

/// Returns a recoverable Error instead of trapping when a local non-closure is called.
#[test]
fn rejects_a_non_callable_dynamic_binding() {
    let compiled = compile_source(
        r#"
        fn main() -> Error {
            let callback = Host::call("callback");
            ret callback();
        }
        "#,
    );
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .register_sync("callback", |_| ExsValue::Int(1))
            .is_ok()
    );
    let result = match block_on(runner.execute(
        &compiled.wasm,
        "main",
        &[],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    let ExsValue::Error(error) = result else {
        panic!("non-callable dynamic binding did not return an Error");
    };
    assert_eq!(error.kind, "TypeError");
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
}

/// Returns a recoverable Error instead of trapping when dynamic `par` receives a non-List.
#[test]
fn rejects_a_non_list_dynamic_parallel_operand() {
    let result = execute_source_with_inputs("fn main() -> Error { ret par(1); }", &[]);
    let ExsValue::Error(error) = result else {
        panic!("non-List dynamic par operand did not return an Error");
    };
    assert_eq!(error.kind, "TypeError");
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
}

/// Validates dynamic `par` elements before scheduling any child tasks.
#[test]
fn rejects_non_callable_and_non_zero_arity_dynamic_parallel_tasks() {
    for (source, kind) in [
        ("fn main() -> Error { ret par([1]); }", "TypeError"),
        (
            "fn main() -> Error { ret par([(value) => { ret value; }]); }",
            "ArityError",
        ),
    ] {
        let result = execute_source_with_inputs(source, &[]);
        let ExsValue::Error(error) = result else {
            panic!("invalid dynamic par task did not return an Error");
        };
        assert_eq!(error.severity, ErrorSeverity::Recoverable);
        assert_eq!(error.kind, kind);
    }
}

/// Returns nested closures while retaining captures through the enclosing closure environment.
#[test]
fn executes_returned_nested_closures() {
    let result = execute_source(
        r#"
        fn make(value: Int) -> Fn {
            let offset = value;
            ret () => {
                ret () => { ret offset + 2; };
            };
        }
        fn main(input: Int) -> Int {
            let first = make(input);
            let second = first();
            ret second();
        }
        "#,
        ExsValue::Int(40),
    );
    assert_eq!(result, ExsValue::Int(42));
}

/// Resumes a closure body after an asynchronous Host ABI call.
#[test]
fn resumes_host_calls_inside_closures() {
    let compiled = compile_source(
        r#"
        fn invoke(function: Fn, value: Int) -> Int | Error {
            ret function(value);
        }
        fn main(input: Int) -> Int | Error {
            let offset = 2;
            let add = (value) => { ret Host::call("echo", value + offset); };
            ret invoke(add, input);
        }
        "#,
    );
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
        "main",
        &[ExsValue::Int(40)],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(42));
}
