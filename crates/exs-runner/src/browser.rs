//! Browser-backed execution for compiled `ExS` modules.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use exs_abi::{
    ABI_VERSION, ErrorSeverity, ExsError, ExsValue, HOST_SLEEP_HOST_NAME,
    HOST_STREAM_NEXT_HOST_NAME, HOST_STREAM_OPEN_HOST_NAME, STANDARD_ITERATOR_STEP_TYPE_IDENTITY,
    SourcePositionId,
};
use js_sys::{Error as JsError, Function, Promise, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};
use wasm_bindgen_futures::{JsFuture, future_to_promise};

use crate::{HostCborError, RunnerError, decode_arguments, encode_result};

/// One non-threaded future returned by a browser host function.
type BrowserHostFuture = Pin<Box<dyn Future<Output = ExsValue> + 'static>>;

/// The result of requesting one value from a browser host-owned pull stream.
pub enum BrowserHostStreamItem {
    /// One stream value is available.
    Item(ExsValue),
    /// The stream has no remaining values.
    End,
}

/// The owned future returned when advancing one browser host-owned pull stream.
pub type BrowserHostStreamFuture = Pin<Box<dyn Future<Output = BrowserHostStreamItem> + 'static>>;

/// A single-consumer browser host source that yields values on demand.
pub trait BrowserHostStream {
    /// Asynchronously produces one item or reports the end of the source.
    fn next(&mut self) -> BrowserHostStreamFuture;
}

/// Opens one browser host-owned pull stream from ordered ExS arguments.
pub trait BrowserHostStreamFunction {
    /// Creates a fresh stream instance for one ExS invocation.
    fn open(&self, arguments: Vec<ExsValue>) -> Result<Box<dyn BrowserHostStream>, ExsValue>;
}

impl<Function, Stream> BrowserHostStreamFunction for Function
where
    Function: Fn(Vec<ExsValue>) -> Result<Stream, ExsValue> + 'static,
    Stream: BrowserHostStream + 'static,
{
    fn open(&self, arguments: Vec<ExsValue>) -> Result<Box<dyn BrowserHostStream>, ExsValue> {
        self(arguments).map(|stream| Box::new(stream) as Box<dyn BrowserHostStream>)
    }
}

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
    /// A stream advance that must release its handle state after completion.
    StreamPending {
        future: BrowserHostStreamFuture,
        stream_id: i64,
    },
}

/// Browser-local host stream state isolated to one root execution.
#[derive(Default)]
struct BrowserStreamState {
    /// Active streams keyed first by their JavaScript execution identity.
    active: HashMap<u32, HashMap<i64, Box<dyn BrowserHostStream>>>,
    /// Stream advances currently awaiting completion.
    pending: HashSet<(u32, i64)>,
    /// Next globally unique stream handle.
    next_id: i64,
}

/// Host functions available to one browser runner configuration.
#[derive(Clone, Default)]
pub struct BrowserHostFunctionRegistry {
    /// Browser-local functions that may capture non-thread-safe Rust-Wasm state.
    functions: Rc<RefCell<HashMap<String, BrowserHostFunction>>>,
    /// Browser-local pull-stream factories keyed by application name.
    streams: Rc<RefCell<HashMap<String, Rc<dyn BrowserHostStreamFunction>>>>,
    /// Per-execution stream instances owned by the browser runner bridge.
    stream_state: Rc<RefCell<BrowserStreamState>>,
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

    /// Registers one browser pull-stream factory under a static host name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is empty or already registered.
    pub fn register_stream<Host>(
        &mut self,
        name: impl Into<String>,
        function: Host,
    ) -> Result<(), BrowserRegistryError>
    where
        Host: BrowserHostStreamFunction + 'static,
    {
        let name = name.into();
        if name.is_empty() {
            return Err(BrowserRegistryError::EmptyName);
        }
        if is_reserved_host_name(&name) {
            return Err(BrowserRegistryError::ReservedName(name));
        }
        if self.functions.borrow().contains_key(&name) || self.streams.borrow().contains_key(&name)
        {
            return Err(BrowserRegistryError::DuplicateName(name));
        }
        let _previous = self.streams.borrow_mut().insert(name, Rc::new(function));
        Ok(())
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

    /// Opens one fresh stream instance for the current browser execution.
    fn open_stream(
        &self,
        execution_id: u32,
        mut arguments: Vec<ExsValue>,
        origin: Option<SourcePositionId>,
    ) -> BrowserHostCall {
        let Some(name) = arguments.first() else {
            return BrowserHostCall::Ready(stream_name_missing_error(origin));
        };
        let ExsValue::String(name) = name else {
            return BrowserHostCall::Ready(stream_name_error(name.clone(), origin));
        };
        let name = name.clone();
        let _stream_name = arguments.remove(0);
        let Some(factory) = self.streams.borrow().get(&name).cloned() else {
            return BrowserHostCall::Ready(missing_stream_error(&name, origin));
        };
        match factory.open(arguments) {
            Ok(stream) => {
                let mut state = self.stream_state.borrow_mut();
                let stream_id = state.next_id;
                state.next_id = state.next_id.saturating_add(1);
                state
                    .active
                    .entry(execution_id)
                    .or_default()
                    .insert(stream_id, stream);
                BrowserHostCall::Ready(ExsValue::Int(stream_id))
            }
            Err(error) => BrowserHostCall::Ready(error),
        }
    }

    /// Starts one stream advance while enforcing single-consumer access.
    fn start_stream_next(
        &self,
        execution_id: u32,
        arguments: Vec<ExsValue>,
        origin: Option<SourcePositionId>,
    ) -> BrowserHostCall {
        let stream_id = match arguments.as_slice() {
            [ExsValue::Int(stream_id)] => *stream_id,
            [other, ..] => {
                return BrowserHostCall::Ready(stream_handle_error(other.clone(), origin));
            }
            [] => return BrowserHostCall::Ready(stream_handle_missing_error(origin)),
        };
        let mut state = self.stream_state.borrow_mut();
        if state.pending.contains(&(execution_id, stream_id)) {
            return BrowserHostCall::Ready(stream_busy_error(stream_id, origin));
        }
        let Some(stream) = state
            .active
            .get_mut(&execution_id)
            .and_then(|streams| streams.get_mut(&stream_id))
        else {
            return BrowserHostCall::Ready(invalid_stream_handle_error(stream_id, origin));
        };
        let future = stream.next();
        let _inserted = state.pending.insert((execution_id, stream_id));
        BrowserHostCall::StreamPending { future, stream_id }
    }

    /// Releases one completed stream advance and drops streams that reached End.
    fn complete_stream_next(
        &self,
        execution_id: u32,
        stream_id: i64,
        item: &BrowserHostStreamItem,
    ) {
        let mut state = self.stream_state.borrow_mut();
        state.pending.remove(&(execution_id, stream_id));
        if matches!(item, BrowserHostStreamItem::End)
            && let Some(streams) = state.active.get_mut(&execution_id)
        {
            let _stream = streams.remove(&stream_id);
        }
    }

    /// Drops every stream retained by one completed or cancelled browser execution.
    fn release_execution(&self, execution_id: u32) {
        let mut state = self.stream_state.borrow_mut();
        let _streams = state.active.remove(&execution_id);
        state.pending.retain(|(id, _)| *id != execution_id);
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
        if is_reserved_host_name(&name) {
            return Err(BrowserRegistryError::ReservedName(name));
        }
        let mut functions = self.functions.borrow_mut();
        if functions.contains_key(&name) || self.streams.borrow().contains_key(&name) {
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
    /// A registration attempted to claim a runner-internal host name.
    ReservedName(String),
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
            Self::ReservedName(name) => {
                write!(
                    formatter,
                    "host function `{name}` is reserved by the runner"
                )
            }
            Self::UnknownName(name) => {
                write!(formatter, "host function `{name}` is not registered")
            }
        }
    }
}

impl std::error::Error for BrowserRegistryError {}

/// Returns whether one name is intercepted by the browser Host ABI.
fn is_reserved_host_name(name: &str) -> bool {
    matches!(
        name,
        HOST_SLEEP_HOST_NAME | HOST_STREAM_OPEN_HOST_NAME | HOST_STREAM_NEXT_HOST_NAME
    )
}

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
    _host_callback: Closure<dyn FnMut(String, Uint8Array, i32, i32) -> JsValue>,
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
            move |name: String, arguments: Uint8Array, source_position: i32, execution_id: i32| {
                let Ok(execution_id) = u32::try_from(execution_id) else {
                    return rejected_browser_value("browser execution identity is invalid");
                };
                start_host_call(
                    &callback_registry,
                    execution_id,
                    &name,
                    &arguments.to_vec(),
                    source_position,
                )
            },
        )
            as Box<dyn FnMut(String, Uint8Array, i32, i32) -> JsValue>);
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

/// Starts one Rust browser host function and converts its result into the JavaScript bridge form.
fn start_host_call(
    registry: &BrowserHostFunctionRegistry,
    execution_id: u32,
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
    let origin = u32::try_from(source_position).ok().map(SourcePositionId);
    let call = if name == HOST_SLEEP_HOST_NAME {
        Ok(start_host_sleep(arguments, origin))
    } else if name == HOST_STREAM_OPEN_HOST_NAME {
        Ok(registry.open_stream(execution_id, arguments, origin))
    } else if name == HOST_STREAM_NEXT_HOST_NAME {
        Ok(registry.start_stream_next(execution_id, arguments, origin))
    } else {
        registry.start(name, arguments)
    };
    match call {
        Ok(BrowserHostCall::Ready(value)) => match browser_response(&value) {
            Ok(value) => value,
            Err(error) => rejected_browser_value(&error),
        },
        Ok(BrowserHostCall::Pending(future)) => future_to_promise(async move {
            browser_response(&future.await).map_err(|error| JsError::new(&error).into())
        })
        .into(),
        Ok(BrowserHostCall::StreamPending { future, stream_id }) => {
            let registry = registry.clone();
            future_to_promise(async move {
                let item = future.await;
                registry.complete_stream_next(execution_id, stream_id, &item);
                let value = match item {
                    BrowserHostStreamItem::Item(value) => ExsValue::Enum {
                        type_id: STANDARD_ITERATOR_STEP_TYPE_IDENTITY.to_owned(),
                        variant: "Item".to_owned(),
                        fields: vec![value],
                    },
                    BrowserHostStreamItem::End => ExsValue::Enum {
                        type_id: STANDARD_ITERATOR_STEP_TYPE_IDENTITY.to_owned(),
                        variant: "Done".to_owned(),
                        fields: Vec::new(),
                    },
                };
                browser_response(&value).map_err(|error| JsError::new(&error).into())
            })
            .into()
        }
        Err(BrowserRegistryError::UnknownName(name)) => {
            match browser_response(&missing_host_error(name, origin)) {
                Ok(value) => value,
                Err(error) => rejected_browser_value(&error),
            }
        }
        Err(error) => rejected_browser_value(&error.to_string()),
    }
}

/// Starts one browser Host sleep Promise after validating its serialized Duration argument.
fn start_host_sleep(arguments: Vec<ExsValue>, origin: Option<SourcePositionId>) -> BrowserHostCall {
    let (seconds, nanoseconds) = match duration_parts(arguments) {
        Ok(parts) => parts,
        Err(message) => return BrowserHostCall::Ready(sleep_error(message, origin)),
    };
    match browser_host_sleep(seconds, nanoseconds) {
        Ok(promise) => BrowserHostCall::Pending(Box::pin(async move {
            match JsFuture::from(promise).await {
                Ok(_) => ExsValue::None,
                Err(error) => sleep_error(format!("Host sleep failed: {error:?}"), origin),
            }
        })),
        Err(error) => BrowserHostCall::Ready(sleep_error(
            format!("could not start Host sleep: {error:?}"),
            origin,
        )),
    }
}

/// Validates one serialized Duration Object and returns normalized duration parts.
fn duration_parts(arguments: Vec<ExsValue>) -> Result<(u64, u32), String> {
    let [ExsValue::Object(entries)] = arguments.as_slice() else {
        return Err("Host::sleep expects exactly one Duration argument".to_owned());
    };
    let mut seconds = None;
    let mut nanoseconds = None;
    for (key, value) in entries {
        let ExsValue::Int(value) = value else {
            return Err("Host::sleep received an invalid Duration value".to_owned());
        };
        match key.as_str() {
            "seconds" if seconds.replace(*value).is_none() => {}
            "nanoseconds" if nanoseconds.replace(*value).is_none() => {}
            _ => return Err("Host::sleep received an invalid Duration value".to_owned()),
        }
    }
    let (Some(seconds), Some(nanoseconds)) = (seconds, nanoseconds) else {
        return Err("Host::sleep received an invalid Duration value".to_owned());
    };
    let seconds = u64::try_from(seconds)
        .map_err(|_| "Host::sleep received a negative Duration value".to_owned())?;
    let nanoseconds = u32::try_from(nanoseconds)
        .map_err(|_| "Host::sleep received an invalid Duration value".to_owned())?;
    if nanoseconds >= 1_000_000_000 {
        return Err("Host::sleep received a non-normalized Duration value".to_owned());
    }
    Ok((seconds, nanoseconds))
}

/// Creates one recoverable Host sleep capability Error.
fn sleep_error(message: String, origin: Option<SourcePositionId>) -> ExsValue {
    ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: "HostSleepError".to_owned(),
        message,
        data: Box::new(ExsValue::None),
        origin,
        trace: Vec::new(),
        cause: None,
    })
}

/// Builds one recoverable browser Host stream error.
fn stream_error(
    kind: &str,
    message: String,
    data: ExsValue,
    origin: Option<SourcePositionId>,
) -> ExsValue {
    ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: kind.to_owned(),
        message,
        data: Box::new(data),
        origin,
        trace: Vec::new(),
        cause: None,
    })
}

/// Builds the Error returned when Host::stream omits its registered stream name.
fn stream_name_missing_error(origin: Option<SourcePositionId>) -> ExsValue {
    stream_error(
        "TypeError",
        "Host::stream expects a stream name as its first argument".to_owned(),
        ExsValue::None,
        origin,
    )
}

/// Builds the Error returned when Host::stream receives a non-String stream name.
fn stream_name_error(value: ExsValue, origin: Option<SourcePositionId>) -> ExsValue {
    stream_error(
        "TypeError",
        "Host::stream expects a String stream name".to_owned(),
        value,
        origin,
    )
}

/// Builds the Error returned when a stream advance omits its handle.
fn stream_handle_missing_error(origin: Option<SourcePositionId>) -> ExsValue {
    stream_error(
        "TypeError",
        "stream next expects a stream handle".to_owned(),
        ExsValue::None,
        origin,
    )
}

/// Builds the Error returned when a stream advance receives a non-Int handle.
fn stream_handle_error(value: ExsValue, origin: Option<SourcePositionId>) -> ExsValue {
    stream_error(
        "TypeError",
        "stream handle must be an Int".to_owned(),
        value,
        origin,
    )
}

/// Builds the Error returned when a stream already has a pending advance.
fn stream_busy_error(stream_id: i64, origin: Option<SourcePositionId>) -> ExsValue {
    stream_error(
        "StreamBusy",
        format!("stream handle `{stream_id}` already has a pending next call"),
        ExsValue::Int(stream_id),
        origin,
    )
}

/// Builds the Error returned when a stream handle is no longer active.
fn invalid_stream_handle_error(stream_id: i64, origin: Option<SourcePositionId>) -> ExsValue {
    stream_error(
        "InvalidStreamHandle",
        format!("stream handle `{stream_id}` is not open"),
        ExsValue::Int(stream_id),
        origin,
    )
}

/// Builds the Error returned for an unregistered stream factory.
fn missing_stream_error(name: &str, origin: Option<SourcePositionId>) -> ExsValue {
    stream_error(
        "HostFunctionNotFound",
        format!("unknown host stream `{name}`"),
        ExsValue::None,
        origin,
    )
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

export async function createBrowserRunner(wasm, host, release, expectedAbiVersion) {
    const module = await WebAssembly.compile(bytes(wasm, "compiled ExS module"));
    if (typeof host !== "function") {
        throw new TypeError("browser host dispatcher must be a function");
    }
    if (typeof release !== "function") {
        throw new TypeError("browser host cleanup callback must be a function");
    }
    let nextExecutionId = 1;
    return {
        async execute(functionName, input) {
            if (nextExecutionId > 2_147_483_647) {
                throw new RangeError("browser execution identity overflow");
            }
            const executionId = nextExecutionId++;
            const ready = new Map();
            const pending = new Map();
            let activeTasks = 0;
            let memory;
            try {
                const imports = {
                exs: {
                    __exs_host_call_start(callId, namePointer, nameLength, requestPointer, requestLength, sourcePosition) {
                        const name = new TextDecoder("utf-8", { fatal: true }).decode(
                            read(memory, namePointer, nameLength, "host function name"),
                        );
                        const request = read(memory, requestPointer, requestLength, "host-call request");
                        const response = host(name, request, sourcePosition, executionId);
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
                if (typeof functionName !== "string" || functionName.length === 0) {
                throw new TypeError("ExS function name must be a non-empty string");
                }
                const start = exportedFunction(instance.exports, `__exs_start_${functionName}`);
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
                const resume = exportedFunction(instance.exports, "__exs_resume_host");
                status = resume(callId, responsePointer, value.length);
                }
            } finally {
                release(executionId);
            }
        },
    };
}

export function executeBrowserRunner(controller, functionName, input) {
    if (!controller || typeof controller.execute !== "function") {
        throw new TypeError("invalid ExS browser runner controller");
    }
    return controller.execute(functionName, input);
}
"#)]
extern "C" {
    /// Creates one JavaScript controller around a browser-compiled ExS module.
    #[wasm_bindgen(catch, js_name = createBrowserRunner)]
    fn create_browser_runner(
        wasm: &Uint8Array,
        host: &Function,
        release: &Function,
        expected_abi_version: i32,
    ) -> Result<Promise, JsValue>;

    /// Executes one isolated ExS instance through the JavaScript controller.
    #[wasm_bindgen(catch, js_name = executeBrowserRunner)]
    fn execute_browser_runner(
        controller: &JsValue,
        function: &str,
        input: &Uint8Array,
    ) -> Result<Promise, JsValue>;
}

#[wasm_bindgen(inline_js = r#"
export function exsHostSleep(seconds, nanoseconds) {
    const maxMilliseconds = 2_147_483_647n;
    let remainingNanoseconds = BigInt(seconds) * 1_000_000_000n + BigInt(nanoseconds);
    return new Promise((resolve) => {
        const schedule = () => {
            if (remainingNanoseconds === 0n) {
                resolve();
                return;
            }
            const milliseconds = remainingNanoseconds / 1_000_000n;
            const delay = milliseconds > maxMilliseconds ? maxMilliseconds : milliseconds;
            const delayNanoseconds = delay * 1_000_000n;
            if (delayNanoseconds === 0n) {
                remainingNanoseconds = 0n;
                globalThis.setTimeout(schedule, 0);
                return;
            }
            remainingNanoseconds -= delayNanoseconds;
            globalThis.setTimeout(schedule, Number(delay));
        };
        schedule();
    });
}
"#)]
extern "C" {
    /// Starts one browser-native timeout Promise for validated Duration parts.
    #[wasm_bindgen(catch, js_name = exsHostSleep)]
    fn browser_host_sleep(seconds: u64, nanoseconds: u32) -> Result<Promise, JsValue>;
}
