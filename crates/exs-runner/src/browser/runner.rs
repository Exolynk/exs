use super::bindings::{ValueReader, create_browser_runner, execute_browser_runner};
use super::direct;
use super::host::{rejected_browser_value, start_host_call};
use super::*;
use crate::value_reader::{ReadError, serialization_error};

/// The host callback JavaScript invokes for every host call: name, request, source position,
/// execution identity and execution start time.
type HostCallback = dyn FnMut(String, JsValue, i32, i32, f64) -> JsValue;

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
    _host_callback: Closure<HostCallback>,
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
                  request: JsValue,
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
                    &request,
                    source_position,
                    execution_started_at,
                )
            },
        ) as Box<HostCallback>);
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
    /// Returns an error when Wasm instantiation, ABI validation, host dispatch, or boundary
    /// handling fails. Recoverable language failures remain `Ok(ExsValue::Error(...))`.
    pub async fn execute(
        &self,
        function: &str,
        inputs: &[ExsValue],
    ) -> Result<ExsValue, RunnerError> {
        let inputs = ExsValue::List(inputs.to_vec());
        let input = direct::image(&inputs).map_err(RunnerError::Abi)?;
        let input = Uint8Array::from(input.as_slice());
        let promise =
            execute_browser_runner(&self.controller, function, &input).map_err(browser_error)?;
        let result = JsFuture::from(promise).await.map_err(browser_error)?;
        if let Some(result) = result.dyn_ref::<Uint8Array>() {
            return direct::read_image(&result.to_vec())
                .map_err(|error| RunnerError::Abi(format!("invalid result image: {error}")));
        }
        if !result.is_object() {
            return Err(RunnerError::Abi(
                "browser runner returned an invalid result".to_owned(),
            ));
        }
        // A result that cannot cross becomes a fatal SerializationError.
        match direct::read(result.unchecked_ref::<ValueReader>()) {
            Ok(value) => Ok(value),
            Err(ReadError::Unsupported(message)) => {
                Ok(ExsValue::Error(serialization_error(message, true)))
            }
            Err(error) => Err(RunnerError::Abi(format!("invalid result: {error:?}"))),
        }
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
