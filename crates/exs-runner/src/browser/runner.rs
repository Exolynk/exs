use super::bindings::{create_browser_runner, execute_browser_runner};
use super::host::{rejected_browser_value, start_host_call};
use super::*;

/// Browser-specific configuration used when creating one reusable runner.
#[derive(Default)]
pub struct BrowserRunnerConfig {
    /// Host implementations captured by the JavaScript import bridge.
    registry: BrowserHostFunctionRegistry,
}

impl BrowserRunnerConfig {
    /// Creates an empty browser runner configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns mutable access to the configured browser host functions.
    pub fn registry_mut(&mut self) -> &mut BrowserHostFunctionRegistry {
        &mut self.registry
    }
}

/// A reusable browser runner that executes compiled ExS Wasm through the browser engine.
pub struct BrowserRunner {
    /// Browser-compiled module and its JavaScript execution controller.
    controller: JsValue,
    /// Rust callback retained for as long as JavaScript may invoke the Host ABI imports.
    _host_callback: Closure<dyn FnMut(String, Uint8Array, i32, i32, f64) -> JsValue>,
    /// Rust callback retained for execution-scoped host stream cleanup.
    _release_callback: Closure<dyn FnMut(i32)>,
}

impl BrowserRunner {
    /// Compiles one linked ExS module through the browser's native WebAssembly API.
    ///
    /// # Errors
    ///
    /// Returns an error when the browser rejects the Wasm module or cannot construct its host
    /// import bridge.
    pub async fn new(wasm: &[u8], configuration: BrowserRunnerConfig) -> Result<Self, RunnerError> {
        let registry = configuration.registry;
        let callback_registry = registry.clone();
        let callback = Closure::wrap(Box::new(
            move |name: String,
                  arguments: Uint8Array,
                  source_position: i32,
                  execution_id: i32,
                  execution_started_at: f64| {
                let Ok(execution_id) = u32::try_from(execution_id) else {
                    return rejected_browser_value("browser execution identity is invalid");
                };
                start_host_call(
                    &callback_registry,
                    execution_id,
                    &name,
                    &arguments.to_vec(),
                    source_position,
                    execution_started_at,
                )
            },
        )
            as Box<dyn FnMut(String, Uint8Array, i32, i32, f64) -> JsValue>);
        let release_registry = registry.clone();
        let release = Closure::wrap(Box::new(move |execution_id: i32| {
            if let Ok(execution_id) = u32::try_from(execution_id) {
                release_registry.release_execution(execution_id);
            }
        }) as Box<dyn FnMut(i32)>);
        let wasm = Uint8Array::from(wasm);
        let host = callback.as_ref().unchecked_ref::<Function>();
        let release_host = release.as_ref().unchecked_ref::<Function>();
        let promise = create_browser_runner(&wasm, host, release_host, ABI_VERSION.cast_signed())
            .map_err(browser_error)?;
        let controller = JsFuture::from(promise).await.map_err(browser_error)?;
        Ok(Self {
            controller,
            _host_callback: callback,
            _release_callback: release,
        })
    }

    /// Executes one isolated named public function with ordered arguments.
    ///
    /// # Errors
    ///
    /// Returns an error when Wasm instantiation, ABI validation, host dispatch, or CBOR boundary
    /// handling fails. Recoverable language failures remain `Ok(ExsValue::Error(...))`.
    pub async fn execute(
        &self,
        function: &str,
        inputs: &[ExsValue],
    ) -> Result<ExsValue, RunnerError> {
        let input = ExsValue::List(inputs.to_vec())
            .to_cbor()
            .map_err(|error| RunnerError::Abi(format!("could not encode input CBOR: {error}")))?;
        let input = Uint8Array::from(input.as_slice());
        let promise =
            execute_browser_runner(&self.controller, function, &input).map_err(browser_error)?;
        let result = JsFuture::from(promise).await.map_err(browser_error)?;
        let result = result.dyn_into::<Uint8Array>().map_err(|_| {
            RunnerError::Abi("browser runner returned a non-byte result".to_owned())
        })?;
        ExsValue::from_cbor(&result.to_vec())
            .map_err(|error| RunnerError::Abi(format!("invalid result CBOR: {error}")))
    }
}

/// Converts a JavaScript rejection or exception into one runner technical error.
fn browser_error(error: JsValue) -> RunnerError {
    RunnerError::Wasm(
        error
            .as_string()
            .unwrap_or_else(|| format!("browser WebAssembly operation failed: {error:?}")),
    )
}
