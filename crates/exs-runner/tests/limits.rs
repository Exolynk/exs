//! Native Wasmtime and generic runner resource-limit integration tests.

mod support;

use std::time::Duration;

use exs_abi::ExsValue;
use exs_runner::{ExecutionCancellation, ExecutionLimits, LimitKind, RunnerError, ServerRunner};
use support::{block_on, compile_source, runner_test_module};

/// Rejects a module whose initial linear memory exceeds the runner memory budget.
#[test]
fn rejects_wasm_memory_over_the_configured_limit() {
    let wasm = runner_test_module(2, "", "i32.const 0");
    let runner = ServerRunner::new(ExecutionLimits {
        max_memory_bytes: 64 * 1024,
        ..ExecutionLimits::default()
    });
    let result = block_on(runner.execute(wasm.as_bytes(), &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::Memory))
    ));
}

/// Rejects guest instruction execution once Wasmtime consumes the fuel budget.
#[test]
fn rejects_execution_after_the_configured_fuel_budget() {
    let compiled = compile_source(
        r#"
        fn main() -> Int {
            ret 1;
        }
        "#,
    );
    let runner = ServerRunner::new(ExecutionLimits {
        max_fuel: 1,
        ..ExecutionLimits::default()
    });
    let result = block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::Fuel))
    ));
}

/// Interrupts an executing guest loop once the root execution reaches its deadline.
#[test]
fn rejects_guest_execution_after_the_configured_timeout() {
    let compiled = compile_source(
        r#"
        fn main() {
            while true {}
        }
        "#,
    );
    let runner = ServerRunner::new(ExecutionLimits {
        max_fuel: u64::MAX,
        timeout: Duration::from_millis(10),
        ..ExecutionLimits::default()
    });
    let result = block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::Timeout))
    ));
}

/// Rejects a completed guest call when elapsed time exceeds the deadline before its timer runs.
#[test]
fn rejects_completed_execution_after_a_zero_timeout() {
    let compiled = compile_source(
        r#"
        fn main() -> Int {
            ret 1;
        }
        "#,
    );
    let runner = ServerRunner::new(ExecutionLimits {
        timeout: Duration::ZERO,
        ..ExecutionLimits::default()
    });
    let result = block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::Timeout))
    ));
}

/// Interrupts a pending asynchronous host call once the root execution reaches its deadline.
#[test]
fn rejects_pending_host_call_after_the_configured_timeout() {
    let compiled = compile_source(
        r#"
        fn main() {
            Host::call("wait");
        }
        "#,
    );
    let mut runner = ServerRunner::new(ExecutionLimits {
        timeout: Duration::from_millis(10),
        ..ExecutionLimits::default()
    });
    assert!(
        runner
            .registry_mut()
            .register_async("wait", |_| std::future::pending::<ExsValue>())
            .is_ok()
    );
    let result = block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::Timeout))
    ));
}

/// Rejects recursive guest calls once their native Wasm stack reaches the configured cap.
#[test]
fn rejects_wasm_stack_over_the_configured_limit() {
    let wasm = runner_test_module(
        1,
        "(func $recurse call $recurse)",
        "call $recurse i32.const 0",
    );
    let runner = ServerRunner::new(ExecutionLimits {
        max_fuel: u64::MAX,
        max_wasm_stack_bytes: 64 * 1024,
        ..ExecutionLimits::default()
    });
    let result = block_on(runner.execute(wasm.as_bytes(), &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::WasmStack))
    ));
}

/// Applies the configured byte limit before allocating the main-input buffer in Wasm memory.
#[test]
fn rejects_main_input_over_the_cbor_payload_limit() {
    let compiled = compile_source(
        r#"
        fn main(value: String) -> String {
            ret value;
        }
        "#,
    );
    let runner = ServerRunner::new(ExecutionLimits {
        max_cbor_payload_bytes: 1,
        ..ExecutionLimits::default()
    });
    let result = block_on(runner.execute(
        &compiled.wasm,
        &[ExsValue::String("input".to_owned())],
        &ExecutionCancellation::new(),
    ));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::CborPayload))
    ));
}

/// Rejects a final runtime result before decoding when it crosses the result-byte limit.
#[test]
fn rejects_result_over_the_configured_limit() {
    let compiled = compile_source(
        r#"
        fn main() -> String {
            ret "result";
        }
        "#,
    );
    let runner = ServerRunner::new(ExecutionLimits {
        max_result_bytes: 1,
        ..ExecutionLimits::default()
    });
    let result = block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::Result))
    ));
}

/// Rejects synchronous host responses that cross the configured CBOR payload limit.
#[test]
fn rejects_host_response_over_the_cbor_payload_limit() {
    let compiled = compile_source(
        r#"
        fn main() -> String {
            ret Host::call("large");
        }
        "#,
    );
    let mut runner = ServerRunner::new(ExecutionLimits {
        max_cbor_payload_bytes: 1,
        ..ExecutionLimits::default()
    });
    assert!(
        runner
            .registry_mut()
            .register_sync("large", |_| ExsValue::String("response".to_owned()))
            .is_ok()
    );
    let result = block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::CborPayload))
    ));
}

/// Rejects a host request whose nested argument value crosses the policy nesting limit.
#[test]
fn rejects_host_request_over_the_cbor_nesting_limit() {
    let compiled = compile_source(
        r#"
        fn main() {
            Host::call("accept", [1]);
        }
        "#,
    );
    let mut runner = ServerRunner::new(ExecutionLimits {
        max_cbor_nesting: 1,
        ..ExecutionLimits::default()
    });
    assert!(
        runner
            .registry_mut()
            .register_sync("accept", |_| ExsValue::None)
            .is_ok()
    );
    let result = block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::CborNesting))
    ));
}

/// Rejects `par` when its active root and child tasks exceed the runner task-permit budget.
#[test]
fn rejects_parallel_tasks_over_the_configured_limit() {
    let compiled = compile_source(
        r#"
        fn main() -> List {
            ret par {
                1;
                2;
            };
        }
        "#,
    );
    let runner = ServerRunner::new(ExecutionLimits {
        max_tasks: 2,
        ..ExecutionLimits::default()
    });
    let result = block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::Tasks))
    ));
}

/// Rejects host calls after the root execution consumes its total host-call budget.
#[test]
fn rejects_host_calls_over_the_configured_total_limit() {
    let compiled = compile_source(
        r#"
        fn main() -> Int {
            Host::call("value");
            ret Host::call("value");
        }
        "#,
    );
    let mut runner = ServerRunner::new(ExecutionLimits {
        max_host_calls: 1,
        ..ExecutionLimits::default()
    });
    assert!(
        runner
            .registry_mut()
            .register_sync("value", |_| ExsValue::Int(1))
            .is_ok()
    );
    let result = block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::HostCalls))
    ));
}

/// Rejects parallel asynchronous host calls once they exceed the pending-call budget.
#[test]
fn rejects_pending_host_calls_over_the_configured_limit() {
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
    let mut runner = ServerRunner::new(ExecutionLimits {
        max_pending_host_calls: 1,
        ..ExecutionLimits::default()
    });
    assert!(
        runner
            .registry_mut()
            .register_async("wait", |_| std::future::pending::<ExsValue>())
            .is_ok()
    );
    let result = block_on(runner.execute(&compiled.wasm, &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::PendingHostCalls))
    ));
}
/// Keeps malformed Wasm modules on the technical runner-error path.
#[test]
fn reports_malformed_wasm_as_a_runner_error() {
    let runner = ServerRunner::new(ExecutionLimits::default());
    let cancellation = ExecutionCancellation::new();
    assert!(block_on(runner.execute(&[0], &[], &cancellation)).is_err());
}

/// Rejects a valid Wasm module that cannot participate in runner task metering.
#[test]
fn rejects_modules_missing_runner_task_metering_imports() {
    let runner = ServerRunner::new(ExecutionLimits::default());
    let cancellation = ExecutionCancellation::new();
    let empty_wasm_module = [0, 97, 115, 109, 1, 0, 0, 0];
    let result = block_on(runner.execute(&empty_wasm_module, &[], &cancellation));
    assert!(matches!(result, Err(RunnerError::Abi(message)) if message.contains("task-metering")));
}
