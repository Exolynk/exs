//! Browser-backed execution for compiled `ExS` modules.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use exs_abi::{ABI_VERSION, ErrorSeverity, ExsError, ExsValue, SourcePositionId};
use js_sys::{Error as JsError, Function, Promise, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};
use wasm_bindgen_futures::{JsFuture, future_to_promise};

use crate::{HostCborError, RunnerError, decode_arguments, encode_result};

/// One non-threaded future returned by a browser host function.
type BrowserHostFuture = Pin<Box<dyn Future<Output = ExsValue> + 'static>>;

/// One browser host implementation registered under a static name.
#[derive(Clone)]
enum BrowserHostFunction {
    /// A host function that completes during the initial Wasm import call.
    Sync(Rc<dyn Fn(Vec<ExsValue>) -> ExsValue>),
    /// A host function whose result is delivered through a browser Promise.
    Async(Rc<dyn Fn(Vec<ExsValue>) -> BrowserHostFuture>),
}

/// The result of starting one browser host function.
enum BrowserHostCall {
    /// A synchronous host function completed immediately.
    Ready(ExsValue),
    /// An asynchronous host function must resolve before the ExS task can resume.
    Pending(BrowserHostFuture),
}

/// Host functions available to one browser runner configuration.
#[derive(Clone, Default)]
pub struct BrowserHostFunctionRegistry {
    /// Browser-local functions that may capture non-thread-safe Rust-Wasm state.
    functions: Rc<RefCell<HashMap<String, BrowserHostFunction>>>,
}

impl BrowserHostFunctionRegistry {
    /// Creates an empty browser host-function registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a synchronous browser host implementation under one static name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is empty or already registered.
    pub fn register_sync<Host>(
        &mut self,
        name: impl Into<String>,
        function: Host,
    ) -> Result<(), BrowserRegistryError>
    where
        Host: Fn(Vec<ExsValue>) -> ExsValue + 'static,
    {
        self.insert(name.into(), BrowserHostFunction::Sync(Rc::new(function)))
    }

    /// Registers an asynchronous browser host implementation under one static name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is empty or already registered.
    pub fn register_async<Host, HostFutureValue>(
        &mut self,
        name: impl Into<String>,
        function: Host,
    ) -> Result<(), BrowserRegistryError>
    where
        Host: Fn(Vec<ExsValue>) -> HostFutureValue + 'static,
        HostFutureValue: Future<Output = ExsValue> + 'static,
    {
        self.insert(
            name.into(),
            BrowserHostFunction::Async(Rc::new(move |arguments| Box::pin(function(arguments)))),
        )
    }

    /// Starts the browser host implementation registered for one name.
    fn start(
        &self,
        name: &str,
        arguments: Vec<ExsValue>,
    ) -> Result<BrowserHostCall, BrowserRegistryError> {
        let function = self
            .functions
            .borrow()
            .get(name)
            .cloned()
            .ok_or_else(|| BrowserRegistryError::UnknownName(name.to_owned()))?;
        Ok(match function {
            BrowserHostFunction::Sync(function) => BrowserHostCall::Ready(function(arguments)),
            BrowserHostFunction::Async(function) => BrowserHostCall::Pending(function(arguments)),
        })
    }

    /// Inserts one function after enforcing stable browser-registry name rules.
    fn insert(
        &mut self,
        name: String,
        function: BrowserHostFunction,
    ) -> Result<(), BrowserRegistryError> {
        if name.is_empty() {
            return Err(BrowserRegistryError::EmptyName);
        }
        let mut functions = self.functions.borrow_mut();
        if functions.contains_key(&name) {
            return Err(BrowserRegistryError::DuplicateName(name));
        }
        let _previous = functions.insert(name, function);
        Ok(())
    }
}

/// An error caused by browser host-function registration or lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BrowserRegistryError {
    /// A registration name was empty.
    EmptyName,
    /// A registration attempted to replace an existing static host name.
    DuplicateName(String),
    /// A host call referenced an unregistered static host name.
    UnknownName(String),
}

impl std::fmt::Display for BrowserRegistryError {
    /// Formats one browser host-registry error for application diagnostics.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyName => formatter.write_str("host function name must not be empty"),
            Self::DuplicateName(name) => {
                write!(formatter, "host function `{name}` is already registered")
            }
            Self::UnknownName(name) => {
                write!(formatter, "host function `{name}` is not registered")
            }
        }
    }
}

impl std::error::Error for BrowserRegistryError {}

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
    _host_callback: Closure<dyn FnMut(String, Uint8Array, i32) -> JsValue>,
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
            move |name: String, arguments: Uint8Array, source_position: i32| {
                start_host_call(
                    &callback_registry,
                    &name,
                    &arguments.to_vec(),
                    source_position,
                )
            },
        )
            as Box<dyn FnMut(String, Uint8Array, i32) -> JsValue>);
        let wasm = Uint8Array::from(wasm);
        let host = callback.as_ref().unchecked_ref::<Function>();
        let promise =
            create_browser_runner(&wasm, host, ABI_VERSION.cast_signed()).map_err(browser_error)?;
        let controller = JsFuture::from(promise).await.map_err(browser_error)?;
        Ok(Self {
            controller,
            _host_callback: callback,
        })
    }

    /// Executes one isolated ExS instance with ordered main-function arguments.
    ///
    /// # Errors
    ///
    /// Returns an error when Wasm instantiation, ABI validation, host dispatch, or CBOR boundary
    /// handling fails. Recoverable language failures remain `Ok(ExsValue::Error(...))`.
    pub async fn execute(&self, inputs: &[ExsValue]) -> Result<ExsValue, RunnerError> {
        let input = ExsValue::List(inputs.to_vec())
            .to_cbor()
            .map_err(|error| RunnerError::Abi(format!("could not encode input CBOR: {error}")))?;
        let input = Uint8Array::from(input.as_slice());
        let promise = execute_browser_runner(&self.controller, &input).map_err(browser_error)?;
        let result = JsFuture::from(promise).await.map_err(browser_error)?;
        let result = result.dyn_into::<Uint8Array>().map_err(|_| {
            RunnerError::Abi("browser runner returned a non-byte result".to_owned())
        })?;
        ExsValue::from_cbor(&result.to_vec())
            .map_err(|error| RunnerError::Abi(format!("invalid result CBOR: {error}")))
    }
}

/// Starts one Rust browser host function and converts its result into the JavaScript bridge form.
fn start_host_call(
    registry: &BrowserHostFunctionRegistry,
    name: &str,
    arguments: &[u8],
    source_position: i32,
) -> JsValue {
    let arguments = match decode_arguments(arguments) {
        Ok(arguments) => arguments,
        Err(error) => {
            return rejected_browser_value(&format!("invalid host-call request: {error}"));
        }
    };
    match registry.start(name, arguments) {
        Ok(BrowserHostCall::Ready(value)) => match browser_response(&value) {
            Ok(value) => value,
            Err(error) => rejected_browser_value(&error),
        },
        Ok(BrowserHostCall::Pending(future)) => future_to_promise(async move {
            browser_response(&future.await).map_err(|error| JsError::new(&error).into())
        })
        .into(),
        Err(BrowserRegistryError::UnknownName(name)) => {
            let origin = u32::try_from(source_position).ok().map(SourcePositionId);
            match browser_response(&missing_host_error(name, origin)) {
                Ok(value) => value,
                Err(error) => rejected_browser_value(&error),
            }
        }
        Err(error) => rejected_browser_value(&error.to_string()),
    }
}

/// Encodes one host response into the byte-array value expected by the JavaScript bridge.
fn browser_response(value: &ExsValue) -> Result<JsValue, String> {
    let bytes = encode_result(value).map_err(host_cbor_error)?;
    Ok(Uint8Array::from(bytes.as_slice()).into())
}

/// Returns one rejected Promise used to surface a technical browser bridge failure.
fn rejected_browser_value(message: &str) -> JsValue {
    Promise::reject(&JsError::new(message)).into()
}

/// Converts one host-boundary CBOR error into a browser bridge error message.
fn host_cbor_error(error: HostCborError) -> String {
    format!("could not encode host response: {error}")
}

/// Builds the recoverable language value used for an unregistered browser host name.
fn missing_host_error(name: String, origin: Option<SourcePositionId>) -> ExsValue {
    ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: "HostFunctionNotFound".to_owned(),
        message: format!("Host function `{name}` is not registered."),
        data: Box::new(ExsValue::None),
        origin,
        trace: Vec::new(),
        cause: None,
    })
}

/// Converts a JavaScript rejection or exception into one runner technical error.
fn browser_error(error: JsValue) -> RunnerError {
    RunnerError::Wasm(
        error
            .as_string()
            .unwrap_or_else(|| format!("browser WebAssembly operation failed: {error:?}")),
    )
}

#[wasm_bindgen(inline_js = r#"
const HOST_CALL_READY = 0;
const HOST_CALL_PENDING = 1;
const STATUS_COMPLETE = 2;
const STATUS_PENDING = 1;
const STATUS_CANCELLED = 3;

function bytes(value, label) {
    if (value instanceof Uint8Array) {
        return value;
    }
    throw new TypeError(`${label} must be a Uint8Array`);
}

function range(memory, pointer, length, label) {
    if (!Number.isInteger(pointer) || !Number.isInteger(length) || pointer < 0 || length < 0) {
        throw new RangeError(`${label} has an invalid memory range`);
    }
    const end = pointer + length;
    if (!Number.isSafeInteger(end) || end > memory.buffer.byteLength) {
        throw new RangeError(`${label} lies outside linear memory`);
    }
    return [pointer, end];
}

function read(memory, pointer, length, label) {
    const [start, end] = range(memory, pointer, length, label);
    return new Uint8Array(memory.buffer.slice(start, end));
}

function write(memory, pointer, value, label) {
    const output = bytes(value, label);
    const [start] = range(memory, pointer, output.length, label);
    new Uint8Array(memory.buffer).set(output, start);
}

function exportedFunction(exports, name) {
    const value = exports[name];
    if (typeof value !== "function") {
        throw new TypeError(`missing or invalid ExS ABI export ${name}`);
    }
    return value;
}

export async function createBrowserRunner(wasm, host, expectedAbiVersion) {
    const module = await WebAssembly.compile(bytes(wasm, "compiled ExS module"));
    if (typeof host !== "function") {
        throw new TypeError("browser host dispatcher must be a function");
    }
    return {
        async execute(input) {
            const ready = new Map();
            const pending = new Map();
            let activeTasks = 0;
            let memory;
            const imports = {
                exs: {
                    __exs_host_call_start(callId, namePointer, nameLength, requestPointer, requestLength, sourcePosition) {
                        const name = new TextDecoder("utf-8", { fatal: true }).decode(
                            read(memory, namePointer, nameLength, "host function name"),
                        );
                        const request = read(memory, requestPointer, requestLength, "host-call request");
                        const response = host(name, request, sourcePosition);
                        if (response && typeof response.then === "function") {
                            pending.set(
                                callId,
                                Promise.resolve(response).then((value) => ({ callId, value: bytes(value, "host response") })),
                            );
                            return HOST_CALL_PENDING;
                        }
                        ready.set(callId, bytes(response, "host response"));
                        return HOST_CALL_READY;
                    },
                    __exs_host_call_response_len(callId) {
                        const response = ready.get(callId);
                        if (!response) {
                            throw new Error("host response is not ready");
                        }
                        return response.length;
                    },
                    __exs_host_call_response_copy(callId, pointer, length) {
                        const response = ready.get(callId);
                        if (!response || response.length !== length) {
                            throw new Error("host response has an unexpected length");
                        }
                        write(memory, pointer, response, "host response destination");
                        ready.delete(callId);
                        return 0;
                    },
                },
                runner: {
                    __runner_task_acquire() {
                        if (activeTasks >= Number.MAX_SAFE_INTEGER) {
                            throw new RangeError("browser task counter overflow");
                        }
                        activeTasks += 1;
                        return 0;
                    },
                    __runner_task_release() {
                        if (activeTasks === 0) {
                            return 1;
                        }
                        activeTasks -= 1;
                        return 0;
                    },
                },
            };
            const instance = await WebAssembly.instantiate(module, imports);
            memory = instance.exports.memory;
            if (!(memory instanceof WebAssembly.Memory)) {
                throw new TypeError("missing exported ExS linear memory");
            }
            const version = exportedFunction(instance.exports, "__exs_abi_version")();
            if (version !== expectedAbiVersion) {
                throw new TypeError(`expected ExS ABI version ${expectedAbiVersion}, received ${version}`);
            }
            const inputBytes = bytes(input, "ExS input");
            const allocate = exportedFunction(instance.exports, "__exs_input_alloc");
            const inputPointer = allocate(inputBytes.length);
            write(memory, inputPointer, inputBytes, "ExS input");
            const start = exportedFunction(instance.exports, "__exs_start");
            const resume = exportedFunction(instance.exports, "__exs_resume_host");
            const resultPointer = exportedFunction(instance.exports, "__exs_result_ptr");
            const resultLength = exportedFunction(instance.exports, "__exs_result_len");
            let status = start(inputPointer, inputBytes.length);
            while (true) {
                if (status === STATUS_COMPLETE) {
                    return read(memory, resultPointer(), resultLength(), "ExS result");
                }
                if (status === STATUS_CANCELLED) {
                    throw new Error("ExS execution was cancelled");
                }
                if (status !== STATUS_PENDING) {
                    throw new Error(`unexpected ExS execution status ${status}`);
                }
                if (pending.size === 0) {
                    throw new Error("ExS execution is pending without a host Promise");
                }
                const { callId, value } = await Promise.race(pending.values());
                pending.delete(callId);
                const responsePointer = allocate(value.length);
                write(memory, responsePointer, value, "host response");
                status = resume(callId, responsePointer, value.length);
            }
        },
    };
}

export function executeBrowserRunner(controller, input) {
    if (!controller || typeof controller.execute !== "function") {
        throw new TypeError("invalid ExS browser runner controller");
    }
    return controller.execute(input);
}
"#)]
extern "C" {
    /// Creates one JavaScript controller around a browser-compiled ExS module.
    #[wasm_bindgen(catch, js_name = createBrowserRunner)]
    fn create_browser_runner(
        wasm: &Uint8Array,
        host: &Function,
        expected_abi_version: i32,
    ) -> Result<Promise, JsValue>;

    /// Executes one isolated ExS instance through the JavaScript controller.
    #[wasm_bindgen(catch, js_name = executeBrowserRunner)]
    fn execute_browser_runner(controller: &JsValue, input: &Uint8Array)
    -> Result<Promise, JsValue>;
}
