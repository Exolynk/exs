use super::bindings::{create_browser_runner, execute_browser_runner};
use super::host::{rejected_browser_value, start_host_call};
use super::images;
use super::*;

/// The host callback JavaScript invokes for every host call: name, request, source position,
/// execution identity and execution start time.
type HostCallback = dyn FnMut(String, JsValue, i32, i32, f64) -> JsValue;

/// The configuration of a browser runner, which so far has no settings: the browser enforces no
/// execution limits.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct BrowserRunnerConfig {}

impl BrowserRunnerConfig {
    /// Creates the default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// A browser-compiled module with the JavaScript bridge of its host calls.
struct LoadedModule {
    /// The module's bytes, which identify it.
    wasm: Vec<u8>,
    /// Browser-compiled module and its JavaScript execution controller.
    controller: JsValue,
    /// Rust callback retained for as long as JavaScript may invoke the Host ABI imports.
    _host_callback: Closure<HostCallback>,
    /// Rust callback retained for execution-scoped host stream cleanup.
    _release_callback: Closure<dyn FnMut(i32)>,
}

/// A reusable browser runner that executes compiled ExS Wasm through the browser engine.
///
/// It keeps the module it compiled last, so executing the same module again compiles nothing.
#[derive(Default)]
pub struct BrowserRunner {
    /// Host implementations captured by the JavaScript import bridge.
    registry: BrowserHostFunctionRegistry,
    /// The module compiled last.
    module: RefCell<Option<Rc<LoadedModule>>>,
}

impl BrowserRunner {
    /// Creates a browser runner without host functions.
    #[must_use]
    pub fn new(_config: BrowserRunnerConfig) -> Self {
        Self::default()
    }

    /// Returns mutable access to the runner-owned dynamic host-function registry.
    pub fn registry_mut(&mut self) -> &mut BrowserHostFunctionRegistry {
        // Bridges capture the registry, so the next execution builds a new one.
        self.module.get_mut().take();
        &mut self.registry
    }

    /// Returns the loaded module of `wasm`, compiling it unless it is the module compiled last.
    async fn load(&self, wasm: &[u8]) -> Result<Rc<LoadedModule>, RunnerError> {
        if let Some(module) = self.module.borrow().as_ref()
            && module.wasm == wasm
        {
            return Ok(Rc::clone(module));
        }
        let registry = &self.registry;
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
        let bytes = Uint8Array::from(wasm);
        let host = callback.as_ref().unchecked_ref::<Function>();
        let release_host = release.as_ref().unchecked_ref::<Function>();
        let promise = create_browser_runner(
            &bytes,
            host,
            release_host,
            MIN_SUPPORTED_ABI_VERSION.cast_signed(),
            ABI_VERSION.cast_signed(),
        )
        .map_err(browser_error)?;
        let controller = JsFuture::from(promise).await.map_err(browser_error)?;
        let module = Rc::new(LoadedModule {
            wasm: wasm.to_vec(),
            controller,
            _host_callback: callback,
            _release_callback: release,
        });
        *self.module.borrow_mut() = Some(Rc::clone(&module));
        Ok(module)
    }

    /// Executes one isolated named public function of the module `wasm` with ordered arguments.
    ///
    /// # Errors
    ///
    /// Returns an error when Wasm instantiation, ABI validation, host dispatch, or boundary
    /// handling fails. Recoverable language failures remain `Ok(ExsValue::Error(...))`.
    pub async fn execute(
        &self,
        wasm: &[u8],
        function: &str,
        inputs: &[ExsValue],
    ) -> Result<ExsValue, RunnerError> {
        self.execute_with(wasm, function, inputs, &ExecutionOptions::default())
            .await
            .map(|report| report.value)
    }

    /// Executes one named public function like [`Self::execute`] under `options` and reports its
    /// times. A cancellation takes effect before the function starts and between host calls.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::execute`], and `Cancelled` once its cancellation fires.
    pub async fn execute_with(
        &self,
        wasm: &[u8],
        function: &str,
        inputs: &[ExsValue],
        options: &ExecutionOptions,
    ) -> Result<ExecutionReport, RunnerError> {
        let started = js_sys::Date::now();
        let module = self.load(wasm).await?;
        let loaded = js_sys::Date::now();
        let input = images::list_image(inputs).map_err(RunnerError::Abi)?;
        let input = Uint8Array::from(input.as_slice());
        let cancellation = options.cancellation.clone();
        let is_cancelled = Closure::wrap(Box::new(move || {
            cancellation
                .as_ref()
                .is_some_and(ExecutionCancellation::is_cancelled)
        }) as Box<dyn FnMut() -> bool>);
        let promise = execute_browser_runner(
            &module.controller,
            function,
            &input,
            is_cancelled.as_ref().unchecked_ref::<Function>(),
        )
        .map_err(browser_error)?;
        let result = JsFuture::from(promise).await.map_err(browser_error)?;
        let Some(result) = result.dyn_ref::<Uint8Array>() else {
            return Err(RunnerError::Abi(
                "browser runner returned an invalid result".to_owned(),
            ));
        };
        let value = images::read_image(&result.to_vec())
            .map_err(|error| RunnerError::Abi(format!("invalid result image: {error}")))?;
        let finished = js_sys::Date::now();
        Ok(ExecutionReport {
            value,
            fuel_consumed: 0,
            compile_time: milliseconds(loaded - started),
            execution_time: milliseconds(finished - loaded),
        })
    }
}

/// Returns the duration of a clock difference of `value` milliseconds.
fn milliseconds(value: f64) -> Duration {
    Duration::try_from_secs_f64(value / 1000.0).unwrap_or_default()
}

/// Converts a JavaScript rejection or exception into one runner technical error.
fn browser_error(error: JsValue) -> RunnerError {
    let field = |name: &str| js_sys::Reflect::get(&error, &JsValue::from_str(name)).ok();
    if field("exsCancelled").and_then(|value| value.as_bool()) == Some(true) {
        return RunnerError::Cancelled;
    }
    if let Some(module) = field("exsAbiVersion").and_then(|value| value.as_f64()) {
        return RunnerError::AbiMismatch {
            module: module as i32,
            min: MIN_SUPPORTED_ABI_VERSION,
            max: ABI_VERSION,
        };
    }
    RunnerError::Wasm(
        error
            .as_string()
            .unwrap_or_else(|| format!("browser WebAssembly operation failed: {error:?}")),
    )
}
