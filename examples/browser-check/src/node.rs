//! The browser side of the check, run under Node.

use exs_abi::ExsValue;
use exs_compiler::{CompileOptions, SourceInput};
use exs_runner::{
    BrowserHostStream, BrowserHostStreamFuture, BrowserHostStreamItem, BrowserRunner,
    BrowserRunnerConfig,
};
use wasm_bindgen::prelude::wasm_bindgen;

use crate::cases::{CASES, GUEST_CASES, add, counter, echo, line};

/// The host stream `counter` in the browser.
struct Counter(std::vec::IntoIter<ExsValue>);

impl BrowserHostStream for Counter {
    fn next(&mut self) -> BrowserHostStreamFuture {
        let item = self
            .0
            .next()
            .map_or(BrowserHostStreamItem::End, BrowserHostStreamItem::Item);
        Box::pin(std::future::ready(item))
    }
}

/// Runs every case, then every entry of the Rust guest `guest`, and returns one result line
/// per run.
#[wasm_bindgen(js_name = runCases)]
pub async fn run_cases(guest: Vec<u8>) -> String {
    let mut lines = Vec::new();
    for case in CASES {
        let result = match compile(case.source) {
            Ok(wasm) => run(&wasm, "main", (case.inputs)()).await,
            Err(error) => Err(error),
        };
        lines.push(line(case.name, "exs", &result));
    }
    for case in GUEST_CASES {
        let result = run(&guest, case.entry, (case.inputs)()).await;
        lines.push(line(case.entry, "rust", &result));
    }
    lines.join("\n")
}

/// Compiles `source`.
fn compile(source: &str) -> Result<Vec<u8>, String> {
    exs_compiler::compile(
        SourceInput {
            source_id: "test.exs",
            text: source,
        },
        CompileOptions::default(),
    )
    .map(|module| module.wasm)
    .map_err(|error| error.to_string())
}

/// Runs the entry `function` of the module `wasm` in the browser runner.
async fn run(wasm: &[u8], function: &str, inputs: Vec<ExsValue>) -> Result<ExsValue, String> {
    let mut configuration = BrowserRunnerConfig::new();
    let registry = configuration.registry_mut();
    registry
        .fn_sync_raw("add", |arguments| add(&arguments))
        .map_err(|error| error.to_string())?;
    registry
        .fn_sync_raw("println", |_| ExsValue::None)
        .map_err(|error| error.to_string())?;
    registry
        .fn_async_raw("echo", |arguments| async move { echo(arguments) })
        .map_err(|error| error.to_string())?;
    registry
        .stream_raw("counter", |arguments: Vec<ExsValue>| {
            Ok(Counter(counter(&arguments).into_iter()))
        })
        .map_err(|error| error.to_string())?;
    let runner = BrowserRunner::new(wasm, configuration)
        .await
        .map_err(|error| error.to_string())?;
    runner
        .execute(function, &inputs)
        .await
        .map_err(|error| error.to_string())
}
