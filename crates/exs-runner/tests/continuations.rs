//! Resumable host-call continuation integration tests.

mod support;

use std::sync::{Arc, Mutex};

use exs_abi::{ErrorSeverity, ExsValue};
use exs_runner::{ExecutionCancellation, ExecutionLimits, ServerRunner};
use support::{block_on, compile_source};

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
    let mut runner = ServerRunner::new(ExecutionLimits::default());
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

/// Preserves the minimum signed 64-bit literal through continuation lowering.
#[test]
fn executes_the_minimum_signed_64_bit_literal_in_a_continuation() {
    let compiled = compile_source(
        r#"
        fn main() -> Int {
            host.call("ready");
            ret -9223372036854775808;
        }
        "#,
    );
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .register_sync("ready", |_| ExsValue::None)
            .is_ok()
    );
    let result = match block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()))
    {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(i64::MIN));
}

/// Terminates a resumable caller after a direct callee violates a strict return contract.
#[test]
fn terminates_continuation_after_a_fatal_direct_call_result() {
    let compiled = compile_source(
        r#"
        fn wrong() -> Int { ret "invalid"; }
        fn main() -> Int {
            host.call("ready");
            wrong();
            ret 42;
        }
        "#,
    );
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .register_sync("ready", |_| ExsValue::None)
            .is_ok()
    );
    let result = match block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()))
    {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    let ExsValue::Error(error) = result else {
        panic!("fatal direct call result was discarded by the continuation");
    };
    assert_eq!(error.severity, ErrorSeverity::Fatal);
    assert_eq!(error.kind, "TypeError");
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
                } else if item == 2 {
                    break;
                }
                {
                    total = total + host.call("echo", item);
                }
            }
            while total < 5 {
                total = total + host.call("echo", 1);
            }
            ret total;
        }
        "#,
    );
    let mut runner = ServerRunner::new(ExecutionLimits::default());
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
    let mut runner = ServerRunner::new(ExecutionLimits::default());
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
    let mut runner = ServerRunner::new(ExecutionLimits::default());
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
    let mut runner = ServerRunner::new(ExecutionLimits::default());
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
    let mut runner = ServerRunner::new(ExecutionLimits::default());
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

/// Preserves nominal field contracts after a pending host result is delivered.
#[test]
fn validates_typed_object_fields_after_asynchronous_host_calls() {
    let compiled = compile_source(
        r#"
        type User { name: String, }
        fn main() -> Error { ret User { name: host.call("wrong") }; }
        "#,
    );
    let mut runner = ServerRunner::new(ExecutionLimits::default());
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
    let mut runner = ServerRunner::new(ExecutionLimits::default());
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
    let mut runner = ServerRunner::new(ExecutionLimits::default());
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
    let mut runner = ServerRunner::new(ExecutionLimits::default());
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
        &[ExsValue::Int(20)],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(41));
}
