//! Integration tests for executing linked Phase-1 ExS modules.

use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use exs_abi::{ErrorSeverity, ExsError, ExsValue};
use exs_compiler::{CompileOptions, SourceInput, compile};
use exs_runner::{ExecutionCancellation, RunnerError, ServerRunner, execute};

/// Compiles source text for runner tests.
fn compile_source(source: &str) -> exs_compiler::CompiledModule {
    match compile(
        SourceInput {
            source_id: "test.exs",
            text: source,
        },
        CompileOptions::default(),
    ) {
        Ok(module) => module,
        Err(error) => panic!("compilation failed: {error}"),
    }
}

/// Executes source text and unwraps a successful runner result for assertions.
fn execute_source(source: &str, input: ExsValue) -> ExsValue {
    execute_source_with_inputs(source, &[input])
}

/// Executes source text with all supplied main arguments for assertions.
fn execute_source_with_inputs(source: &str, inputs: &[ExsValue]) -> ExsValue {
    let compiled = compile_source(source);
    match execute(&compiled.wasm, inputs) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    }
}

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
                host.call("wait");
                host.call("wait");
            };
        }
        "#,
    );
    let polls = Arc::new(AtomicUsize::new(0));
    let mut runner = ServerRunner::new();
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
    let result = match block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()))
    {
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
            let add = (value) => { ret host.call("echo", value + offset); };
            ret invoke(add, input);
        }
        "#,
    );
    let mut runner = ServerRunner::new();
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
        &[ExsValue::Int(40)],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(42));
}

/// Polls one immediately-ready test future without adding an executor dependency.
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

/// Registers the host functions used by the continuation source-position matrix.
fn register_continuation_matrix_hosts(
    runner: &mut ServerRunner,
    asynchronous: bool,
    calls: Arc<Mutex<Vec<String>>>,
) {
    for name in [
        "echo",
        "record",
        "name",
        "is_positive",
        "is_less_than",
        "range",
    ] {
        let name = name.to_owned();
        let registration =
            if asynchronous {
                let calls = Arc::clone(&calls);
                let function_name = name.clone();
                runner.registry_mut().register_async(name, move |arguments| {
                let calls = Arc::clone(&calls);
                let function_name = function_name.clone();
                async move { continuation_matrix_response(&function_name, arguments, &calls) }
            })
            } else {
                let calls = Arc::clone(&calls);
                let function_name = name.clone();
                runner.registry_mut().register_sync(name, move |arguments| {
                    continuation_matrix_response(&function_name, arguments, &calls)
                })
            };
        assert!(registration.is_ok());
    }
}

/// Returns a deterministic value for one continuation source-position matrix host call.
fn continuation_matrix_response(
    name: &str,
    arguments: Vec<ExsValue>,
    calls: &Arc<Mutex<Vec<String>>>,
) -> ExsValue {
    calls
        .lock()
        .expect("matrix call log mutex poisoned")
        .push(name.to_owned());
    match name {
        "echo" => arguments.into_iter().next().unwrap_or(ExsValue::None),
        "record" => ExsValue::None,
        "name" => ExsValue::String("echo".to_owned()),
        "is_positive" => {
            ExsValue::Bool(matches!(arguments.as_slice(), [ExsValue::Int(value)] if *value > 0))
        }
        "is_less_than" => ExsValue::Bool(matches!(
            arguments.as_slice(),
            [ExsValue::Int(value), ExsValue::Int(limit)] if value < limit
        )),
        "range" => ExsValue::List(vec![ExsValue::Int(0), ExsValue::Int(1)]),
        _ => ExsValue::None,
    }
}

/// Asserts the shared host-call order for the continuation source-position matrix.
fn assert_continuation_matrix_calls(calls: Arc<Mutex<Vec<String>>>) {
    let calls = calls.lock().expect("matrix call log mutex poisoned");
    assert_eq!(
        calls.as_slice(),
        [
            "echo",
            "record",
            "name",
            "echo",
            "echo",
            "echo",
            "echo",
            "echo",
            "echo",
            "echo",
            "echo",
            "is_positive",
            "echo",
            "is_less_than",
            "echo",
            "is_less_than",
            "echo",
            "is_less_than",
            "echo",
            "is_less_than",
            "echo",
            "is_less_than",
            "range",
            "echo",
            "echo",
            "echo",
        ]
    );
}

/// Compiles the common source program used to exercise continuation source positions.
fn continuation_matrix_module() -> exs_compiler::CompiledModule {
    compile_source(
        r#"
        type Accumulator { value: Int, }
        impl Accumulator {
            fn new(value: Int) -> Accumulator { ret Accumulator { value: value }; }
            fn add(self, value: Int) -> Int {
                self.value = self.value + value;
                ret self.value;
            }
        }
        fn identity(value: Int) -> Int { ret value; }
        fn main(input: Int) -> Int | Error {
            host.call("record", host.call("echo", input));
            let name = host.call("name");
            let values = [host.call(name, input), host.call("echo", 2)];
            let object = {
                first: host.call("echo", values[0]),
                second: host.call("echo", values[1]),
            };
            values[0] = host.call("echo", object.first);
            object.second = host.call("echo", values[1]);
            let index = host.call("echo", 0);
            let total = identity(object["second"] + values[index]);
            let accumulator = Accumulator::new(host.call("echo", total));
            if host.call("is_positive", total) {
                total = accumulator.add(host.call("echo", 1));
            } else {
                ret 0;
            }
            while host.call("is_less_than", total, 10) {
                total = accumulator.add(host.call("echo", 1));
            }
            for item in host.call("range", 2) {
                total = accumulator.add(host.call("echo", item));
            }
            ret host.call("echo", total)?;
        }
        "#,
    )
}

/// Registers a Boolean host function that records each expression evaluated through it.
fn register_logical_host(
    runner: &mut ServerRunner,
    asynchronous: bool,
    calls: Arc<Mutex<Vec<String>>>,
) {
    let registration = if asynchronous {
        runner
            .registry_mut()
            .register_async("truth", move |arguments: Vec<ExsValue>| {
                let calls = Arc::clone(&calls);
                async move {
                    record_logical_host_call(&calls, &arguments);
                    ExsValue::Bool(true)
                }
            })
    } else {
        runner
            .registry_mut()
            .register_sync("truth", move |arguments: Vec<ExsValue>| {
                record_logical_host_call(&calls, &arguments);
                ExsValue::Bool(true)
            })
    };
    assert!(registration.is_ok());
}

/// Records the String label supplied to the logical-expression host function.
fn record_logical_host_call(calls: &Arc<Mutex<Vec<String>>>, arguments: &[ExsValue]) {
    let [ExsValue::String(label)] = arguments else {
        panic!("logical host call received unexpected arguments: {arguments:?}");
    };
    calls
        .lock()
        .expect("logical call log mutex poisoned")
        .push(label.clone());
}

/// Compiles a continuation program that distinguishes evaluated and short-circuited operands.
fn logical_continuation_module() -> exs_compiler::CompiledModule {
    compile_source(
        r#"
        fn main() {
            let and_short = false && host.call("truth", "and-short");
            let and_evaluated = true && host.call("truth", "and-evaluated");
            let or_short = true || host.call("truth", "or-short");
            let or_evaluated = false || host.call("truth", "or-evaluated");
            if and_short || !and_evaluated || !or_short || !or_evaluated {
                ret 0;
            }
            ret 1;
        }
        "#,
    )
}

/// Compiles a continuation program that constructs a nominal Object from a host result.
fn typed_object_continuation_module() -> exs_compiler::CompiledModule {
    compile_source(
        r#"
        type User { name: String, nickname: String | None, }
        impl User {
            fn display(self) -> String { ret self.name; }
        }
        fn main(input: String) -> String | Error {
            let user = User { name: host.call("echo", input) };
            if user.nickname != None {
                ret "invalid";
            }
            ret user.display();
        }
        "#,
    )
}

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
    let mut runner = ServerRunner::new();
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

/// Suspends and resumes the same generated frame for an asynchronous host function.
#[test]
fn executes_an_asynchronous_dynamic_host_function() {
    let compiled = compile_source("fn main(input) { ret host.call(\"echo\", input); }");
    let mut runner = ServerRunner::new();
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
    let mut runner = ServerRunner::new();
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
    let mut runner = ServerRunner::new();
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
    let runner = ServerRunner::new();
    let result = block_on(runner.execute(wasm.as_bytes(), &[], &cancellation));
    assert!(
        matches!(result, Err(RunnerError::Deadlock(message)) if message.contains("without a runner host future"))
    );
}

/// Continues through nested expressions and mutable sequential statements after ready host calls.
#[test]
fn executes_sequential_continuation_states_for_synchronous_host_calls() {
    let compiled = compile_source(
        r#"
        fn main(input) {
            let base = host.call("echo", input) + 1;
            let values = [base, host.call("echo", 2)];
            values[0] = host.call("echo", values[0]);
            ret values[0] + values[1];
        }
        "#,
    );
    let mut runner = ServerRunner::new();
    assert!(
        runner
            .registry_mut()
            .register_sync("echo", |arguments: Vec<ExsValue>| {
                arguments.into_iter().next().unwrap_or(ExsValue::None)
            })
            .is_ok()
    );
    let result = match block_on(runner.execute(
        &compiled.wasm,
        &[ExsValue::Int(4)],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(7));
}

/// Resumes nested host-call states and propagates their completed values through `?`.
#[test]
fn executes_sequential_continuation_states_for_asynchronous_host_calls() {
    let compiled = compile_source(
        r#"
        fn main(input) -> Int | Error {
            let value = host.call("echo", input)?;
            value = value + host.call("echo", 1);
            ret value;
        }
        "#,
    );
    let mut runner = ServerRunner::new();
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
        &[ExsValue::Int(41)],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(42));
}

/// Preserves conditional and loop control edges while host calls suspend and resume.
#[test]
fn executes_control_flow_continuation_states_for_asynchronous_host_calls() {
    let compiled = compile_source(
        r#"
        fn main(input) {
            let total = 0;
            for item in host.call("values", input) {
                if item > 2 {
                    continue;
                }
                if item == 2 {
                    break;
                }
                total = total + host.call("echo", item);
            }
            while total < 5 {
                total = total + host.call("echo", 1);
            }
            ret total;
        }
        "#,
    );
    let mut runner = ServerRunner::new();
    assert!(
        runner
            .registry_mut()
            .register_async("values", |_arguments: Vec<ExsValue>| async move {
                ExsValue::List(vec![ExsValue::Int(1), ExsValue::Int(3), ExsValue::Int(2)])
            })
            .is_ok()
    );
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
        &[ExsValue::None],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(5));
}

/// Preserves resumable loop execution across scheduler quantum checkpoints.
#[test]
fn executes_asynchronous_host_calls_after_scheduler_quantum_yields() {
    let compiled = compile_source(
        r#"
        fn main() {
            let count = 0;
            while count < 130 {
                count = count + 1;
            }
            ret host.call("echo", count);
        }
        "#,
    );
    let mut runner = ServerRunner::new();
    assert!(
        runner
            .registry_mut()
            .register_async("echo", |arguments: Vec<ExsValue>| async move {
                arguments.into_iter().next().unwrap_or(ExsValue::None)
            })
            .is_ok()
    );
    let result = match block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()))
    {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(130));
}

/// Short-circuits resumable logical expressions when host calls return immediately.
#[test]
fn short_circuits_logical_continuations_for_synchronous_host_calls() {
    let compiled = logical_continuation_module();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut runner = ServerRunner::new();
    register_logical_host(&mut runner, false, Arc::clone(&calls));

    let result = match block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()))
    {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(1));
    assert_eq!(
        calls
            .lock()
            .expect("logical call log mutex poisoned")
            .as_slice(),
        ["and-evaluated", "or-evaluated"]
    );
}

/// Short-circuits resumable logical expressions across pending host-call resumes.
#[test]
fn short_circuits_logical_continuations_for_asynchronous_host_calls() {
    let compiled = logical_continuation_module();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut runner = ServerRunner::new();
    register_logical_host(&mut runner, true, Arc::clone(&calls));

    let result = match block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()))
    {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(1));
    assert_eq!(
        calls
            .lock()
            .expect("logical call log mutex poisoned")
            .as_slice(),
        ["and-evaluated", "or-evaluated"]
    );
}

/// Constructs nominal Objects through the synchronous host fast path in resumable code.
#[test]
fn constructs_typed_objects_for_synchronous_host_calls() {
    let compiled = typed_object_continuation_module();
    let mut runner = ServerRunner::new();
    assert!(
        runner
            .registry_mut()
            .register_sync("echo", |arguments: Vec<ExsValue>| {
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

/// Constructs nominal Objects after resuming a pending host result in resumable code.
#[test]
fn constructs_typed_objects_for_asynchronous_host_calls() {
    let compiled = typed_object_continuation_module();
    let mut runner = ServerRunner::new();
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

/// Preserves nominal field contracts after a pending host result is delivered.
#[test]
fn validates_typed_object_fields_after_asynchronous_host_calls() {
    let compiled = compile_source(
        r#"
        type User { name: String, }
        fn main() -> Error { ret User { name: host.call("wrong") }; }
        "#,
    );
    let mut runner = ServerRunner::new();
    assert!(
        runner
            .registry_mut()
            .register_async("wrong", |_arguments: Vec<ExsValue>| async move {
                ExsValue::Int(7)
            })
            .is_ok()
    );

    let result = match block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()))
    {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    let ExsValue::Error(error) = result else {
        panic!("expected a TypeError result");
    };
    assert_eq!(error.kind, "TypeError");
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
}

/// Covers every supported host-call source position through immediately ready host responses.
#[test]
fn executes_continuation_source_positions_for_synchronous_host_calls() {
    let compiled = continuation_matrix_module();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut runner = ServerRunner::new();
    register_continuation_matrix_hosts(&mut runner, false, Arc::clone(&calls));

    let result = match block_on(runner.execute(
        &compiled.wasm,
        &[ExsValue::Int(3)],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(11));
    assert_continuation_matrix_calls(calls);
}

/// Covers every supported host-call source position through suspended frame resumes.
#[test]
fn executes_continuation_source_positions_for_asynchronous_host_calls() {
    let compiled = continuation_matrix_module();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut runner = ServerRunner::new();
    register_continuation_matrix_hosts(&mut runner, true, Arc::clone(&calls));

    let result = match block_on(runner.execute(
        &compiled.wasm,
        &[ExsValue::Int(3)],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(11));
    assert_continuation_matrix_calls(calls);
}

/// Enforces typed continuation parameters and results at the same boundaries as direct functions.
#[test]
fn validates_resumable_function_type_contracts() {
    let compiled = compile_source(
        r#"
        fn main(value: Int) -> Int {
            ret host.call("echo", value);
        }
        "#,
    );
    let mut runner = ServerRunner::new();
    assert!(
        runner
            .registry_mut()
            .register_sync("echo", |_arguments: Vec<ExsValue>| ExsValue::String(
                "wrong".to_owned()
            ))
            .is_ok()
    );
    let result = match block_on(runner.execute(
        &compiled.wasm,
        &[ExsValue::Int(1)],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    let ExsValue::Error(error) = result else {
        panic!("expected a TypeError result");
    };
    assert_eq!(error.kind, "TypeError");
    assert_eq!(error.severity, ErrorSeverity::Fatal);
}

/// Delivers a pending child-frame result into the direct caller continuation.
#[test]
fn executes_transitive_suspendable_direct_calls() {
    let compiled = compile_source(
        r#"
        fn double(value) {
            ret host.call("echo", value) * 2;
        }
        fn main(input) {
            ret double(input) + 1;
        }
        "#,
    );
    let mut runner = ServerRunner::new();
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
        &[ExsValue::Int(20)],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(41));
}

/// Routes a pending static implementation method through a child frame.
#[test]
fn executes_transitive_suspendable_static_calls() {
    let compiled = compile_source(
        r#"
        type Math {}
        impl Math {
            fn double(value) { ret host.call("echo", value) * 2; }
        }
        fn main(input) { ret Math::double(input) + 1; }
        "#,
    );
    let mut runner = ServerRunner::new();
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
        &[ExsValue::Int(20)],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(41));
}

/// Routes a pending nominal instance method through a child frame.
#[test]
fn executes_transitive_suspendable_instance_calls() {
    let compiled = compile_source(
        r#"
        type Number { value: Int, }
        impl Number {
            fn double(self) { ret host.call("echo", self.value) * 2; }
            fn new(value) -> Number { ret Number { value: value }; }
        }
        fn main(input) { ret Number::new(input).double() + 1; }
        "#,
    );
    let mut runner = ServerRunner::new();
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
        &[ExsValue::Int(20)],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(41));
}

/// Routes a trait-selected instance method through the same pending child-frame path.
#[test]
fn executes_transitive_suspendable_trait_calls() {
    let compiled = compile_source(
        r#"
        trait Double { fn double(self) -> Int; }
        type Number { value: Int, }
        impl Double for Number {
            fn double(self) -> Int { ret host.call("echo", self.value) * 2; }
        }
        impl Number {
            fn new(value) -> Number { ret Number { value: value }; }
        }
        fn render(value: Double) -> Int { ret value.double(); }
        fn main(input) { ret render(Number::new(input)) + 1; }
        "#,
    );
    let mut runner = ServerRunner::new();
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
        &[ExsValue::Int(20)],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(41));
}

/// Retains root and child language frames when a child host call returns an Error value.
#[test]
fn traces_errors_through_suspendable_child_frames() {
    let compiled = compile_source(
        r#"
        fn child(value) -> Error { ret host.call("echo", value) + "invalid"; }
        fn main(input) -> Error { ret child(input)?; }
        "#,
    );
    let mut runner = ServerRunner::new();
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
        &[ExsValue::Int(7)],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    let ExsValue::Error(error) = result else {
        panic!("expected a TypeError");
    };
    assert_eq!(error.kind, "TypeError");
    assert!(error.trace.len() >= 2);
}

/// Constructs nominal Objects, fills omitted optional fields, and dispatches implementation methods.
#[test]
fn executes_nominal_object_construction_and_implementation_methods() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            type User {
                name: String,
                nickname: String | None,
                metadata,
            }
            impl User {
                fn display(self) -> String { ret self.name; }
                fn named(name: String) -> User { ret User { name: name }; }
            }
            fn main() -> String {
                let user = User::named("Ada");
                ret user.display();
            }
        "#,
            &[],
        ),
        ExsValue::String("Ada".to_owned())
    );
}

/// Constructs tagged enum values, dispatches their methods, and returns them through CBOR.
#[test]
fn executes_enum_constructors_implementations_and_cbor_results() {
    let source = r#"
        enum Color {
            Rgb(red: Int, green: Int, blue: Int),
            Transparent,
        }
        trait Rank { fn rank(self) -> Int; }
        impl Color { fn channels(self) -> Int { ret 3; } }
        impl Rank for Color { fn rank(self) -> Int { ret self.channels(); } }
        fn main() -> Color {
            let color = Color::Rgb(255, 0, 128);
            let transparent = Color::Transparent;
            let count = color.rank() + transparent.channels();
            if count == 6 { ret color; }
            ret transparent;
        }
    "#;
    assert_eq!(
        execute_source_with_inputs(source, &[]),
        ExsValue::Enum {
            type_id: "test.exs::Color".to_owned(),
            variant: "Rgb".to_owned(),
            fields: vec![ExsValue::Int(255), ExsValue::Int(0), ExsValue::Int(128)],
        }
    );
}

/// Accepts a tagged enum supplied by a runner for its matching enum contract.
#[test]
fn accepts_cbor_enum_input_for_enum_contract() {
    assert_eq!(
        execute_source(
            "enum Color { Transparent, } fn main(value: Color) -> Color { ret value; }",
            ExsValue::Enum {
                type_id: "test.exs::Color".to_owned(),
                variant: "Transparent".to_owned(),
                fields: vec![],
            },
        ),
        ExsValue::Enum {
            type_id: "test.exs::Color".to_owned(),
            variant: "Transparent".to_owned(),
            fields: vec![],
        }
    );
}

/// Constructs an enum after a host suspension through the continuation lowerer.
#[test]
fn constructs_enum_after_a_host_call() {
    let compiled = compile_source(
        "enum Color { Gray(value: Int), } fn main() -> Color { let value = host.call(\"value\"); ret Color::Gray(value); }",
    );
    let mut runner = ServerRunner::new();
    assert!(
        runner
            .registry_mut()
            .register_sync("value", |_| ExsValue::Int(42))
            .is_ok()
    );
    let result = match block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()))
    {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(
        result,
        ExsValue::Enum {
            type_id: "test.exs::Color".to_owned(),
            variant: "Gray".to_owned(),
            fields: vec![ExsValue::Int(42)],
        }
    );
}

/// Dispatches enum matches and exposes ordered payload values only to the selected arm.
#[test]
fn executes_exhaustive_enum_match_expressions() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                enum Color {
                    Rgb(red: Int, green: Int, blue: Int),
                    Transparent,
                }
                fn main() -> Int {
                    let color = Color::Rgb(255, 0, 128);
                    ret match color {
                        Color::Rgb(red, green, blue) => red + green + blue,
                        Color::Transparent => 0,
                    };
                }
            "#,
            &[],
        ),
        ExsValue::Int(383)
    );
}

/// Returns directly from a selected statement-block enum match arm.
#[test]
fn returns_from_a_block_enum_match_arm() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                enum Color {
                    Rgb(r: Int, g: Int, b: Int),
                    Name(value),
                    Transparent,
                }
                impl Color {
                    fn as_number(self) -> Int {
                        ret match self {
                            Color::Rgb(r, g, b) => r + g + b,
                            Color::Name(_) => 0,
                            Color::Transparent => { ret -1; },
                        };
                    }
                }
                fn main() -> Int {
                    let color = Color::Transparent;
                    ret color.as_number();
                }
            "#,
            &[],
        ),
        ExsValue::Int(-1)
    );
}

/// Selects a wildcard match arm when no preceding variant arm accepts the value.
#[test]
fn executes_enum_match_wildcard_fallback() {
    assert_eq!(
        execute_source_with_inputs(
            "enum Color { Red, Blue, } fn main() -> Int { let color = Color::Blue; ret match color { Color::Red => 1, _ => 2, }; }",
            &[],
        ),
        ExsValue::Int(2)
    );
}

/// Returns MatchError when a host enum has the expected type identity but an unknown variant.
#[test]
fn returns_match_error_for_unknown_host_enum_variant() {
    let result = execute_source(
        "enum Color { Red, Blue, } fn main(value: Color) -> Int | Error { ret match value { Color::Red => 1, Color::Blue => 2, }; }",
        ExsValue::Enum {
            type_id: "test.exs::Color".to_owned(),
            variant: "Green".to_owned(),
            fields: vec![],
        },
    );
    let ExsValue::Error(error) = result else {
        panic!("unknown enum variant did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
    assert_eq!(error.kind, "MatchError");
}

/// Resumes a host call performed by the selected enum match arm.
#[test]
fn resumes_host_call_inside_enum_match_arm() {
    let compiled = compile_source(
        "enum Color { Red, Blue, } fn main() -> Int { let color = Color::Blue; ret match color { Color::Red => 1, Color::Blue => host.call(\"value\"), }; }",
    );
    let mut runner = ServerRunner::new();
    assert!(
        runner
            .registry_mut()
            .register_sync("value", |_| ExsValue::Int(42))
            .is_ok()
    );
    let result = match block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()))
    {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(42));
}

/// Executes a nominal Object construction without invoking an implementation method.
#[test]
fn executes_nominal_object_construction() {
    assert_eq!(
        execute_source(
            "type User { name: String, nickname: String | None, } fn main(input) -> String { let user = User { name: \"Ada\" }; ret user.name; }",
            ExsValue::None,
        ),
        ExsValue::String("Ada".to_owned())
    );
}

/// Inserts explicit None entries for omitted `Any` and None-permitting nominal Object fields.
#[test]
fn fills_omitted_nominal_object_fields_with_none() {
    assert_eq!(
        execute_source(
            "type User { name: String, nickname: String | None, metadata, } fn main(input) { let user = User { name: \"Ada\" }; ret user.has(\"nickname\") && user.nickname == None && user.has(\"metadata\"); }",
            ExsValue::None,
        ),
        ExsValue::Bool(true)
    );
}

/// Returns a language TypeError when a nominal Object field violates its declared contract.
#[test]
fn returns_type_error_for_invalid_nominal_object_field() {
    let result = execute_source_with_inputs(
        "type User { name: String, } fn main() -> Error { ret User { name: 1 }; }",
        &[],
    );
    let ExsValue::Error(error) = result else {
        panic!("invalid nominal field did not return an Error");
    };
    assert_eq!(error.kind, "TypeError");
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
}

/// Dispatches a required trait instance method through a trait-typed function parameter.
#[test]
fn dispatches_trait_instance_methods_and_validates_trait_contracts() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            trait Label { fn label(self) -> String; }
            type User { name: String, }
            impl Label for User { fn label(self) -> String { ret self.name; } }
            fn render(value: Label) -> String { ret value.label(); }
            fn main() -> String { ret render(User { name: "Ada" }); }
        "#,
            &[],
        ),
        ExsValue::String("Ada".to_owned())
    );
}

/// Dispatches a static trait default method through its implementing nominal type.
#[test]
fn dispatches_inherited_static_trait_default_methods() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            trait Category { fn category() -> String { ret "person"; } }
            type User {}
            impl Category for User {}
            fn main() -> String { ret User::category(); }
        "#,
            &[],
        ),
        ExsValue::String("person".to_owned())
    );
}

/// Executes calls, assignments, conditionals, and boolean operators.
#[test]
fn executes_calls_assignments_conditionals_and_booleans() {
    assert_eq!(
        execute_source(
            r#"
            fn double(value) { ret value * 2; }
            fn main(input) {
                let value = 20;
                value = double(value);
                if value == 40 && true {
                    ret value + 2;
                } else {
                    ret 0;
                }
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::Int(42)
    );
}

/// Preserves the inclusive lower integer bound in compiled code.
#[test]
fn executes_the_minimum_exs_integer_literal() {
    assert_eq!(
        execute_source("fn main(input) { ret -36028797018963968; }", ExsValue::None,),
        ExsValue::Int(exs_value::MIN_INT)
    );
}

/// Links the compiler's committed runtime template into an executable module.
#[test]
fn links_against_the_committed_runtime_template() {
    assert_eq!(
        execute_source("fn main(input) { ret 7 * 6; }", ExsValue::None),
        ExsValue::Int(42)
    );
}

/// Evaluates boolean equality inside the runtime rather than as a compiler shortcut.
#[test]
fn evaluates_boolean_equality_in_the_runtime() {
    assert_eq!(
        execute_source("fn main(input) { ret true == false; }", ExsValue::None),
        ExsValue::Bool(false)
    );
}

/// Promotes mixed arithmetic to Float and treats Bool as its numeric 0 or 1 value.
#[test]
fn executes_mixed_bool_integer_and_float_arithmetic() {
    assert_eq!(
        execute_source("fn main(input) { ret true + 2 + 0.5; }", ExsValue::None),
        ExsValue::Float(3.5)
    );
}

/// Compares Bool, Int, and Float values through the runtime numeric dispatch.
#[test]
fn compares_mixed_numeric_values() {
    assert_eq!(
        execute_source(
            "fn main(input) { ret true == 1.0 && false < 1; }",
            ExsValue::None,
        ),
        ExsValue::Bool(true)
    );
}

/// Decodes CBOR input in the runtime and passes it to the one main parameter.
#[test]
fn passes_cbor_input_to_main() {
    assert_eq!(
        execute_source("fn main(input) { ret input + 1; }", ExsValue::Int(41)),
        ExsValue::Int(42)
    );
}

/// Passes ordered CBOR values into typed multi-parameter main declarations.
#[test]
fn passes_multiple_cbor_inputs_to_main() {
    assert_eq!(
        execute_source_with_inputs(
            "fn main(number: Int, offset: Float, name: String) -> String { ret name; }",
            &[
                ExsValue::Int(1),
                ExsValue::Float(0.5),
                ExsValue::String("Ada".to_owned()),
            ],
        ),
        ExsValue::String("Ada".to_owned()),
    );
}

/// Substitutes None for missing main arguments before applying their contracts.
#[test]
fn substitutes_none_for_missing_main_inputs() {
    assert_eq!(
        execute_source_with_inputs("fn main(value: None) -> None { ret value; }", &[]),
        ExsValue::None,
    );
}

/// Rejects entry input arrays that contain more values than main declares.
#[test]
fn rejects_excess_main_inputs_with_a_fatal_arity_error() {
    let result = execute_source_with_inputs("fn main() { ret None; }", &[ExsValue::Int(1)]);
    let ExsValue::Error(error) = result else {
        panic!("excess main input did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Fatal);
    assert_eq!(error.kind, "ArityError");
    assert_eq!(error.data, Box::new(ExsValue::List(vec![ExsValue::Int(1)])));
}

/// Constructs a UTF-8 literal from a passive Wasm data segment and compares its contents.
#[test]
fn executes_string_literals_and_content_equality() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let name = "Ada\nLovelace \u{1f642}";
                if name == input {
                    ret name;
                } else {
                    ret "unexpected";
                }
            }
        "#,
            ExsValue::String("Ada\nLovelace 🙂".to_owned()),
        ),
        ExsValue::String("Ada\nLovelace 🙂".to_owned())
    );
}

/// Preserves list reference semantics through dynamic index and member dispatch.
#[test]
fn executes_list_literals_index_assignment_and_push() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let first = [input, 2];
                let second = first;
                second.push(3);
                first[1] = first[1] + 40;
                ret first;
            }
        "#,
            ExsValue::Int(1),
        ),
        ExsValue::List(vec![ExsValue::Int(1), ExsValue::Int(42), ExsValue::Int(3)]),
    );
}

/// Uses identity equality for Lists and exposes the new length from `push`.
#[test]
fn preserves_list_identity_and_returns_push_length() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let first = [1];
                let alias = first;
                let length = alias.push(2);
                if first == alias && first != [1, 2] {
                    ret length;
                }
                ret 0;
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::Int(2),
    );
}

/// Decodes a host list for the root input and returns a nested list result.
#[test]
fn passes_list_cbor_input_to_main() {
    assert_eq!(
        execute_source(
            "fn main(input) { input.push([3]); ret input; }",
            ExsValue::List(vec![ExsValue::Int(1), ExsValue::Int(2)]),
        ),
        ExsValue::List(vec![
            ExsValue::Int(1),
            ExsValue::Int(2),
            ExsValue::List(vec![ExsValue::Int(3)]),
        ]),
    );
}

/// Implements every remaining List mutation method with its specified return value.
#[test]
fn executes_remaining_list_operations() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let values = [1, 3];
                values.insert(1, 2);
                let removed = values.remove(0);
                let last = values.pop();
                values.clear();
                let empty = values.pop();
                ret [removed, last, empty, values];
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![
            ExsValue::Int(1),
            ExsValue::Int(3),
            ExsValue::None,
            ExsValue::List(vec![]),
        ]),
    );
}

/// Appends one value or chains two Lists without mutating either source List.
#[test]
fn adds_lists_to_values_and_other_lists() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let base = [1];
                let appended = base + input;
                let chained = appended + [3, 4];
                ret [base, appended, chained];
            }
        "#,
            ExsValue::Int(2),
        ),
        ExsValue::List(vec![
            ExsValue::List(vec![ExsValue::Int(1)]),
            ExsValue::List(vec![ExsValue::Int(1), ExsValue::Int(2)]),
            ExsValue::List(vec![
                ExsValue::Int(1),
                ExsValue::Int(2),
                ExsValue::Int(3),
                ExsValue::Int(4),
            ]),
        ]),
    );
}

/// Preserves object insertion order through literal construction and mutations.
#[test]
fn executes_object_literals_properties_dynamic_keys_and_methods() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let key = "name";
                let profile = { name: input, "role": "admin" };
                let alias = profile;
                alias.score = 42;
                profile[key] = "Ada";
                let keys = profile.keys();
                let values = profile.values();
                if profile.has("score") && keys[0] == "name" && keys[1] == "role" && keys[2] == "score" && values[2] == 42 {
                    ret profile;
                }
                ret {};
            }
        "#,
            ExsValue::Int(1),
        ),
        ExsValue::Object(vec![
            ("name".to_owned(), ExsValue::String("Ada".to_owned())),
            ("role".to_owned(), ExsValue::String("admin".to_owned())),
            ("score".to_owned(), ExsValue::Int(42)),
        ]),
    );
}

/// Uses identity equality and deletion semantics for Objects.
#[test]
fn preserves_object_identity_and_deletion_behavior() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let first = { value: input };
                let alias = first;
                let fresh = { value: input };
                let removed = alias.delete("value");
                if first == alias && first != fresh && removed == input && !first.has("value") {
                    ret first;
                }
                ret fresh;
            }
        "#,
            ExsValue::Int(42),
        ),
        ExsValue::Object(vec![]),
    );
}

/// Decodes a host object for the root input and returns it as an ordered CBOR map.
#[test]
fn passes_object_cbor_input_to_main() {
    assert_eq!(
        execute_source(
            "fn main(input) { input.updated = true; ret input; }",
            ExsValue::Object(vec![(
                "name".to_owned(),
                ExsValue::String("Ada".to_owned())
            )]),
        ),
        ExsValue::Object(vec![
            ("name".to_owned(), ExsValue::String("Ada".to_owned())),
            ("updated".to_owned(), ExsValue::Bool(true)),
        ]),
    );
}

/// Keeps aliased runtime Objects alive while repeated helper allocations trigger collection.
#[test]
fn preserves_live_aliases_across_allocation_triggered_collection() {
    assert_eq!(
        execute_source(
            r#"
            fn churn(value) {
                let discarded = [value, { value: value }, [value, value]];
                ret 0;
            }
            fn main(input) {
                let object = { value: input };
                let alias = object;
                churn(1);
                churn(2);
                churn(3);
                if alias == object && alias.value == input {
                    ret object;
                }
                ret {};
            }
        "#,
            ExsValue::Int(42),
        ),
        ExsValue::Object(vec![("value".to_owned(), ExsValue::Int(42))]),
    );
}

/// Traces a self-referential List without losing its identity or looping during collection.
#[test]
fn traces_cycles_during_allocation_triggered_collection() {
    assert_eq!(
        execute_source(
            r#"
            fn churn(value) {
                let discarded = [value, value, { value: value }];
                ret 0;
            }
            fn main(input) {
                let cycle = [];
                cycle.push(cycle);
                churn(1);
                churn(2);
                churn(3);
                ret cycle[0] == cycle;
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::Bool(true),
    );
}

/// Evaluates while conditions repeatedly and branches to the nearest loop targets.
#[test]
fn executes_while_with_break_and_continue() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let value = 0;
                let sum = 0;
                while value < 10 {
                    value = value + 1;
                    if value == 2 {
                        continue;
                    }
                    if value == 6 {
                        break;
                    }
                    sum = sum + value;
                }
                ret sum;
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::Int(13),
    );
}

/// Iterates a List snapshot even when the source List mutates during the loop.
#[test]
fn iterates_a_shallow_list_snapshot() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let values = [1, 2, 3];
                let sum = 0;
                for item in values {
                    if item == 1 {
                        values.push(4);
                    }
                    if item == 2 {
                        continue;
                    }
                    sum = sum + item;
                }
                ret [sum, values];
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![
            ExsValue::Int(4),
            ExsValue::List(vec![
                ExsValue::Int(1),
                ExsValue::Int(2),
                ExsValue::Int(3),
                ExsValue::Int(4),
            ]),
        ]),
    );
}

/// Iterates UTF-8 strings as individual Unicode scalar runtime Strings.
#[test]
fn iterates_string_unicode_scalars() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let scalars = [];
                for scalar in "A🙂B" {
                    scalars.push(scalar);
                }
                ret scalars;
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![
            ExsValue::String("A".to_owned()),
            ExsValue::String("🙂".to_owned()),
            ExsValue::String("B".to_owned()),
        ]),
    );
}

/// Preserves rooted values while loop allocations repeatedly trigger collection.
#[test]
fn preserves_live_values_during_allocation_heavy_loops() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let stable = { value: [input] };
                let alias = stable;
                let count = 0;
                while count < 64 {
                    let discarded = [{ count: count }, [count, count], "discarded"];
                    count = count + 1;
                }
                ret alias.value[0];
            }
        "#,
            ExsValue::Int(42),
        ),
        ExsValue::Int(42),
    );
}

/// Executes direct Option values through the linked runtime.
#[test]
fn executes_direct_option_values() {
    assert_eq!(
        execute_source("fn main(input) { ret input; }", ExsValue::Int(42)),
        ExsValue::Int(42),
    );
    assert_eq!(
        execute_source("fn main(input) { ret None; }", ExsValue::None),
        ExsValue::None,
    );
}

/// Enforces annotated argument and return types at direct function boundaries.
#[test]
fn validates_function_type_contracts() {
    assert_eq!(
        execute_source(
            r#"
            fn convert(value: Int, offset: Float) -> Float | Error {
                ret value + offset;
            }
            fn main(input) {
                ret convert(input, 0.5);
            }
            "#,
            ExsValue::Int(2),
        ),
        ExsValue::Float(2.5),
    );
    assert_eq!(
        execute_source(
            r#"
            fn echo(value: Any) -> Any {
                ret value;
            }
            fn main(input) {
                ret echo(input);
            }
            "#,
            ExsValue::Object(vec![("enabled".to_owned(), ExsValue::Bool(true))]),
        ),
        ExsValue::Object(vec![("enabled".to_owned(), ExsValue::Bool(true))]),
    );
    assert_error_kind_with_input(
        r#"
        fn identity(value: Int) -> Int | Error {
            ret value;
        }
        fn main(input) {
            ret identity(input);
        }
        "#,
        ExsValue::String("invalid".to_owned()),
        "TypeError",
    );
    assert_error_kind(
        r#"
        fn wrong() -> Int | Error {
            ret "invalid";
        }
        fn main(input) {
            ret wrong();
        }
        "#,
        "TypeError",
    );
}

/// Preserves direct Error values that are explicitly accepted by a return union.
#[test]
fn accepts_error_values_in_function_type_contracts() {
    let result = execute_source(
        r#"
        fn fail(value: Error) -> Int | Error {
            ret value;
        }
        fn main(input) {
            ret fail(input);
        }
        "#,
        ExsValue::Error(ExsError {
            severity: ErrorSeverity::Recoverable,
            kind: "Expected".to_owned(),
            message: "expected failure".to_owned(),
            data: Box::new(ExsValue::None),
            origin: None,
            trace: Vec::new(),
            cause: None,
        }),
    );
    let ExsValue::Error(error) = result else {
        panic!("typed Error value was not returned");
    };
    assert_eq!(error.kind, "Expected");
}

/// Accepts None when a return union explicitly includes it.
#[test]
fn accepts_none_in_function_type_contracts() {
    assert_eq!(
        execute_source(
            "fn missing() -> None | Int { ret None; } fn main(input) { ret missing(); }",
            ExsValue::None,
        ),
        ExsValue::None,
    );
}

/// Returns a fatal Error when a strict function contract rejects a value.
#[test]
fn returns_a_fatal_error_for_a_strict_function_type_contract_violation() {
    let result = execute_source(
        "fn wrong() -> Int { ret \"invalid\"; } fn main(input) { ret wrong(); }",
        ExsValue::None,
    );
    let ExsValue::Error(error) = result else {
        panic!("strict type contract did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Fatal);
    assert_eq!(error.kind, "TypeError");
    assert!(error.origin.is_some());
    assert_eq!(error.trace.len(), 2);
}

/// Keeps malformed Wasm modules on the technical runner-error path.
#[test]
fn reports_malformed_wasm_as_a_runner_error() {
    assert!(execute(&[0], &[]).is_err());
}

/// Preserves direct values and propagates Error values unchanged with question mark.
#[test]
fn propagates_option_and_result_values() {
    assert_eq!(
        execute_source(
            "fn main(input) { let value = input?; ret value; }",
            ExsValue::Int(42),
        ),
        ExsValue::Int(42),
    );
    let error = ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: "Example".to_owned(),
        message: "example error".to_owned(),
        data: Box::new(ExsValue::None),
        origin: None,
        trace: Vec::new(),
        cause: None,
    });
    assert_eq!(
        execute_source(
            "fn main(input) { let value = input?; ret value; }",
            error.clone()
        ),
        error,
    );
}

/// Converts None propagation into a MissingValue Error.
#[test]
fn converts_none_propagation_to_missing_value_error() {
    let result = execute_source("fn main(input) { ret None?; }", ExsValue::None);
    let ExsValue::Error(error) = result else {
        panic!("None propagation did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
    assert_eq!(error.kind, "MissingValue");
    assert_eq!(error.data, Box::new(ExsValue::None));
    assert!(error.origin.is_some());
    assert_eq!(error.trace.len(), 1);
}

/// Captures direct generated function frames when an Error is created.
#[test]
fn captures_direct_function_error_trace() {
    let result = execute_source(
        "fn inner(value) { ret None?; } fn main(input) { ret inner(input); }",
        ExsValue::None,
    );
    let ExsValue::Error(error) = result else {
        panic!("missing Error result");
    };
    assert_eq!(error.trace.len(), 2);
    assert_eq!(error.trace[0].function_id, 0);
    assert_eq!(error.trace[1].function_id, 1);
}

/// Tests host-provided Error values through source-level is Error.
#[test]
fn tests_error_values_in_source() {
    let error = ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: "Example".to_owned(),
        message: "example error".to_owned(),
        data: Box::new(ExsValue::None),
        origin: None,
        trace: Vec::new(),
        cause: None,
    });
    assert_eq!(
        execute_source("fn main(input) { ret input is Error; }", error),
        ExsValue::Bool(true),
    );
}

/// Constructs a source-level recoverable Error with its data and source trace intact.
#[test]
fn constructs_errors_with_the_error_builtin() {
    let result = execute_source(
        r#"
        fn main(input) {
            ret Error("ValidationError", "invalid input", { value: input });
        }
        "#,
        ExsValue::Int(42),
    );
    let ExsValue::Error(error) = result else {
        panic!("error builtin did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
    assert_eq!(error.kind, "ValidationError");
    assert_eq!(error.message, "invalid input");
    assert_eq!(
        error.data,
        Box::new(ExsValue::Object(vec![(
            "value".to_owned(),
            ExsValue::Int(42)
        )]))
    );
    assert!(error.origin.is_some());
    assert_eq!(error.trace.len(), 1);
}

/// Validates the kind and message arguments accepted by the Error builtin.
#[test]
fn validates_error_builtin_string_arguments() {
    assert_error_kind(
        "fn main(input) { ret Error(1, \"message\", input); }",
        "TypeError",
    );
    assert_error_kind(
        "fn main(input) { ret Error(\"Kind\", 1, input); }",
        "TypeError",
    );
}

/// Returns a recoverable Error instead of trapping for invalid dynamic source operations.
#[test]
fn returns_recoverable_errors_for_invalid_dynamic_operations() {
    assert_error_kind("fn main(input) { ret [] - 1; }", "TypeError");
    assert_error_kind("fn main(input) { ret [][0]; }", "IndexError");
    assert_error_kind(
        "fn main(input) { let value = 1; ret value.push(2); }",
        "TypeError",
    );
    assert_error_kind(
        "fn main(input) { for item in 1 { ret item; } ret 0; }",
        "NotIterable",
    );
}

/// Returns a recoverable Error when a non-Boolean value is used as a condition.
#[test]
fn returns_a_recoverable_error_for_an_invalid_condition() {
    let result = execute_source("fn main(input) { if 1 { ret 2; } ret 3; }", ExsValue::None);
    let ExsValue::Error(error) = result else {
        panic!("invalid condition did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
    assert_eq!(error.kind, "TypeError");
    assert!(error.origin.is_some());
    assert_eq!(error.trace.len(), 1);
}

/// Executes source and verifies that it returns a recoverable Error of the requested kind.
fn assert_error_kind(source: &str, kind: &str) {
    assert_error_kind_with_input(source, ExsValue::None, kind);
}

/// Executes source and verifies an input-specific recoverable Error result.
fn assert_error_kind_with_input(source: &str, input: ExsValue, kind: &str) {
    let result = execute_source(source, input);
    let ExsValue::Error(error) = result else {
        panic!("source did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
    assert_eq!(error.kind, kind);
}
