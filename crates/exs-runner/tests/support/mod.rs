//! Shared integration-test helpers for native runner behavior.

#![allow(dead_code)] // Each integration-test crate uses only a subset of these shared helpers.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use exs_abi::ExsValue;
use exs_compiler::{CompileOptions, SourceInput, compile};
use exs_runner::{ExecutionCancellation, ExecutionLimits, ServerRunner};

/// Compiles source text for native runner integration tests.
pub fn compile_source(source: &str) -> exs_compiler::CompiledModule {
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

/// Executes source text with one main input and unwraps a successful runner result.
pub fn execute_source(source: &str, input: ExsValue) -> ExsValue {
    execute_source_with_inputs(source, &[input])
}

/// Executes source text with all supplied main inputs and unwraps a successful runner result.
pub fn execute_source_with_inputs(source: &str, inputs: &[ExsValue]) -> ExsValue {
    let compiled = compile_source(source);
    let runner = ServerRunner::new(ExecutionLimits::default());
    let cancellation = ExecutionCancellation::new();
    match block_on(runner.execute(&compiled.wasm, "main", inputs, &cancellation)) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    }
}

/// Builds a minimal ABI-compatible Wasm module for native runner-limit tests.
pub fn runner_test_module(memory_pages: u32, helper: &str, start_body: &str) -> String {
    format!(
        r#"
        (module
            (import "runner" "__runner_task_acquire" (func (result i32)))
            (import "runner" "__runner_task_release" (func (result i32)))
            (memory (export "memory") {memory_pages})
            (func (export "__exs_abi_version") (result i32)
                i32.const {})
            (func (export "__exs_input_alloc") (param i32) (result i32)
                i32.const 0)
            {helper}
            (func (export "__exs_start_main") (param i32 i32) (result i32)
                {start_body})
        )
        "#,
        exs_abi::ABI_VERSION
    )
}

/// Polls one immediately-ready test future without adding an executor dependency.
pub fn block_on<Output>(future: impl Future<Output = Output>) -> Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}
