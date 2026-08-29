//! Native Wasmtime and generic runner resource-policy integration tests.

mod support;

use std::time::Duration;
use std::{sync::mpsc, thread};

use exs_abi::ExsValue;
use exs_runner::{ExecutionCancellation, LimitKind, ProtectionLevel, RunnerError, ServerRunner};
use support::{block_on, compile_source, runner_test_module};

/// Creates a runner with one explicit public execution policy.
fn runner(
    max_memory_bytes: usize,
    max_fuel: u64,
    timeout: Duration,
    protection: ProtectionLevel,
) -> ServerRunner {
    ServerRunner::new(max_memory_bytes, max_fuel, timeout, protection)
}

/// Rejects a module whose initial linear memory exceeds the runner memory budget.
#[test]
fn rejects_wasm_memory_over_the_configured_limit() {
    let wasm = runner_test_module(2, "", "i32.const 0");
    let runner = runner(
        64 * 1024,
        10_000_000,
        Duration::from_secs(10),
        ProtectionLevel::Standard,
    );
    let result =
        block_on(runner.execute(wasm.as_bytes(), "main", &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::Memory))
    ));
}

/// Rejects an oversized module before native compilation begins.
#[test]
fn rejects_wasm_over_the_derived_module_size_limit() {
    let wasm = vec![0; 4 * 1024 * 1024 + 1];
    let runner = runner(
        16 * 1024 * 1024,
        10_000_000,
        Duration::from_secs(10),
        ProtectionLevel::High,
    );
    let result = block_on(runner.execute(&wasm, "main", &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::Module))
    ));
}

/// Rejects guest instruction execution once Wasmtime consumes the fuel budget.
#[test]
fn rejects_execution_after_the_configured_fuel_budget() {
    let compiled = compile_source("fn main() -> Int { ret 1; }");
    let runner = runner(
        16 * 1024 * 1024,
        1,
        Duration::from_secs(10),
        ProtectionLevel::Standard,
    );
    let result =
        block_on(runner.execute(&compiled.wasm, "main", &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::Fuel))
    ));
}

/// Interrupts an executing guest loop once the root execution reaches its deadline.
#[test]
fn rejects_guest_execution_after_the_configured_timeout() {
    let compiled = compile_source("fn main() { while true {} }");
    let runner = runner(
        16 * 1024 * 1024,
        u64::MAX,
        Duration::from_millis(10),
        ProtectionLevel::Standard,
    );
    let result =
        block_on(runner.execute(&compiled.wasm, "main", &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::Timeout))
    ));
}

/// Interrupts an active guest loop when the caller requests cancellation.
#[test]
fn cancels_an_active_guest_execution() {
    let compiled = compile_source(
        r#"
        fn main() {
            let started = Host::call("started");
            while started == None {}
        }
        "#,
    );
    let (started_sender, started_receiver) = mpsc::channel();
    let mut runner = runner(
        16 * 1024 * 1024,
        u64::MAX,
        Duration::from_secs(1),
        ProtectionLevel::Standard,
    );
    assert!(
        runner
            .registry_mut()
            .fn_sync_raw("started", move |_| {
                let _ = started_sender.send(());
                ExsValue::None
            })
            .is_ok()
    );
    let cancellation = ExecutionCancellation::new();
    let execution_cancellation = cancellation.clone();
    let execution = thread::spawn(move || {
        block_on(runner.execute(&compiled.wasm, "main", &[], &execution_cancellation))
    });
    started_receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("guest execution did not reach the synchronization host call");
    cancellation.cancel();
    let result = execution.join().expect("guest execution thread panicked");
    assert!(matches!(result, Err(RunnerError::Cancelled)));
}

/// Bounds a pending asynchronous host call by the public execution timeout.
#[test]
fn rejects_pending_host_call_after_the_configured_timeout() {
    let compiled = compile_source("fn main() { Host::call(\"wait\"); }");
    let mut runner = runner(
        16 * 1024 * 1024,
        10_000_000,
        Duration::from_millis(10),
        ProtectionLevel::Standard,
    );
    assert!(
        runner
            .registry_mut()
            .fn_async_raw("wait", |_| std::future::pending::<ExsValue>())
            .is_ok()
    );
    let result =
        block_on(runner.execute(&compiled.wasm, "main", &[], &ExecutionCancellation::new()));
    assert!(matches!(
        result,
        Err(RunnerError::LimitExceeded(LimitKind::Timeout))
    ));
}

/// Rejects a table whose initial allocation exceeds the high-protection budget.
#[test]
fn rejects_wasm_table_over_the_derived_limit() {
    let wasm = runner_test_module(1, "(table 4097 funcref)", "i32.const 0");
    let runner = runner(
        16 * 1024 * 1024,
        10_000_000,
        Duration::from_secs(10),
        ProtectionLevel::High,
    );
    let result =
        block_on(runner.execute(wasm.as_bytes(), "main", &[], &ExecutionCancellation::new()));
    assert!(result.is_err());
}

/// Rejects multiple linear memories to keep public memory accounting exact.
#[test]
fn rejects_wasm_with_multiple_memories() {
    let wasm = runner_test_module(1, "(memory 1)", "i32.const 0");
    let runner = ServerRunner::default();
    let result =
        block_on(runner.execute(wasm.as_bytes(), "main", &[], &ExecutionCancellation::new()));
    assert!(result.is_err());
}

/// Runs ordinary ExS output under every supported protection profile.
#[test]
fn executes_compiled_exs_with_every_protection_level() {
    let compiled = compile_source("fn main() -> Int { ret 42; }");
    for protection in [
        ProtectionLevel::Low,
        ProtectionLevel::Standard,
        ProtectionLevel::High,
    ] {
        let runner = runner(
            16 * 1024 * 1024,
            10_000_000,
            Duration::from_secs(10),
            protection,
        );
        let result =
            block_on(runner.execute(&compiled.wasm, "main", &[], &ExecutionCancellation::new()));
        assert!(matches!(result, Ok(ExsValue::Int(42))));
    }
}

/// Keeps malformed Wasm modules on the technical runner-error path.
#[test]
fn reports_malformed_wasm_as_a_runner_error() {
    let runner = ServerRunner::default();
    let cancellation = ExecutionCancellation::new();
    assert!(block_on(runner.execute(&[0], "main", &[], &cancellation)).is_err());
}

/// Rejects a valid Wasm module that cannot participate in runner task metering.
#[test]
fn rejects_modules_missing_runner_task_metering_imports() {
    let runner = ServerRunner::default();
    let cancellation = ExecutionCancellation::new();
    let empty_wasm_module = [0, 97, 115, 109, 1, 0, 0, 0];
    let result = block_on(runner.execute(&empty_wasm_module, "main", &[], &cancellation));
    assert!(matches!(result, Err(RunnerError::Abi(message)) if message.contains("task-metering")));
}
