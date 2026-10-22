//! The browser side of the check, run under Node.

use exs_abi::ExsValue;
use exs_compiler::{Backend, CompileOptions, SourceInput, compile_with_backend};
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

/// Runs every case as a v1 and a runtime v2 module, then every entry of the Rust guest
/// `guest`, and returns one result line per run.
#[wasm_bindgen(js_name = runCases)]
pub async fn run_cases(guest: Vec<u8>) -> String {
    let mut lines = Vec::new();
    for case in CASES {
        for (backend, name) in [(Backend::V1, "v1"), (Backend::V2, "v2")] {
            let result = match compile(case.source, backend) {
                Ok(wasm) => run(&wasm, "main", (case.inputs)()).await,
                Err(error) => Err(error),
            };
            lines.push(line(case.name, name, &result));
        }
    }
    for case in GUEST_CASES {
        let result = run(&guest, case.entry, (case.inputs)()).await;
        lines.push(line(case.entry, "rust", &result));
    }
    lines.join("\n")
}

/// Compiles `source` for `backend`.
fn compile(source: &str, backend: Backend) -> Result<Vec<u8>, String> {
    compile_with_backend(
        SourceInput {
            source_id: "test.exs",
            text: source,
        },
        CompileOptions::default(),
        backend,
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
