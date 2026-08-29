//! Browser-backed execution for compiled `ExS` modules.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use exs_abi::{
    ABI_VERSION, ErrorSeverity, ExsError, ExsValue, STANDARD_ITERATOR_STEP_TYPE_IDENTITY,
    SourcePositionId, is_reserved_host_name,
};
use js_sys::{Array, Error as JsError, Function, Promise, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};
use wasm_bindgen_futures::{JsFuture, future_to_promise};

#[cfg(feature = "serde")]
use crate::typed_registry::{decode_typed_request, typed_encode_error};
use crate::{HostCborError, RunnerError, decode_arguments, encode_result};
#[cfg(feature = "serde")]
use serde::{Serialize, de::DeserializeOwned};

mod bindings;
mod host;
mod runner;

use host::{
    invalid_stream_handle_error, missing_stream_error, stream_busy_error, stream_handle_error,
    stream_handle_missing_error, stream_name_error, stream_name_missing_error,
};

pub use runner::{BrowserRunner, BrowserRunnerConfig};

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

    /// Registers a typed synchronous browser host implementation under one static name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is empty or already registered.
    #[cfg(feature = "serde")]
    pub fn fn_sync<Request, Response, Host>(
        &mut self,
        name: impl Into<String>,
        function: Host,
    ) -> Result<(), BrowserRegistryError>
    where
        Request: DeserializeOwned + 'static,
        Response: Serialize + 'static,
        Host: Fn(Request) -> Result<Response, ExsError> + 'static,
    {
        self.fn_sync_raw(name, move |arguments| {
            let request = match decode_typed_request(arguments) {
                Ok(request) => request,
                Err(error) => return error,
            };
            match function(request) {
                Ok(response) => match ExsValue::from_serialize(&response) {
                    Ok(value) => value,
                    Err(error) => typed_encode_error(error.to_string()),
                },
                Err(error) => ExsValue::Error(error),
            }
        })
    }

    /// Registers a low-level synchronous browser host implementation under one static name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is empty or already registered.
    pub fn fn_sync_raw<Host>(
        &mut self,
        name: impl Into<String>,
        function: Host,
    ) -> Result<(), BrowserRegistryError>
    where
        Host: Fn(Vec<ExsValue>) -> ExsValue + 'static,
    {
        self.insert(name.into(), BrowserHostFunction::Sync(Rc::new(function)))
    }

    /// Registers a typed asynchronous browser host implementation under one static name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is empty or already registered.
    #[cfg(feature = "serde")]
    pub fn fn_async<Request, Response, Host, HostFutureValue>(
        &mut self,
        name: impl Into<String>,
        function: Host,
    ) -> Result<(), BrowserRegistryError>
    where
        Request: DeserializeOwned + 'static,
        Response: Serialize + 'static,
        Host: Fn(Request) -> HostFutureValue + 'static,
        HostFutureValue: Future<Output = Result<Response, ExsError>> + 'static,
    {
        let function = Rc::new(function);
        self.fn_async_raw(name, move |arguments| {
            let request = decode_typed_request(arguments);
            let function = Rc::clone(&function);
            async move {
                let request = match request {
                    Ok(request) => request,
                    Err(error) => return error,
                };
                match function(request).await {
                    Ok(response) => match ExsValue::from_serialize(&response) {
                        Ok(value) => value,
                        Err(error) => typed_encode_error(error.to_string()),
                    },
                    Err(error) => ExsValue::Error(error),
                }
            }
        })
    }

    /// Registers a low-level asynchronous browser host implementation under one static name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is empty or already registered.
    pub fn fn_async_raw<Host, HostFutureValue>(
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

    /// Registers one typed iterator factory under a static host name.
    ///
    /// Each iterator item is serialized to the ExS boundary value as it is requested. Use
    /// [`Self::stream_raw`] when the source itself must yield asynchronously.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is empty or already registered.
    #[cfg(feature = "serde")]
    pub fn stream<Request, Item, Items, Host>(
        &mut self,
        name: impl Into<String>,
        function: Host,
    ) -> Result<(), BrowserRegistryError>
    where
        Request: DeserializeOwned + 'static,
        Item: Serialize + 'static,
        Items: IntoIterator<Item = Item> + 'static,
        Items::IntoIter: 'static,
        Host: Fn(Request) -> Result<Items, ExsError> + 'static,
    {
        self.stream_raw(name, move |arguments| {
            let request = decode_typed_request(arguments)?;
            let items = function(request).map_err(ExsValue::Error)?;
            Ok(BrowserSerializedIterator {
                items: items.into_iter(),
            })
        })
    }

    /// Registers one low-level browser pull-stream factory under a static host name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is empty or already registered.
    pub fn stream_raw<Host>(
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

/// A typed iterator exposed through the browser pull-stream protocol.
#[cfg(feature = "serde")]
struct BrowserSerializedIterator<Items> {
    /// Remaining application values.
    items: Items,
}

#[cfg(feature = "serde")]
impl<Items> BrowserHostStream for BrowserSerializedIterator<Items>
where
    Items: Iterator + 'static,
    Items::Item: Serialize,
{
    fn next(&mut self) -> BrowserHostStreamFuture {
        let item = match self.items.next() {
            Some(value) => match ExsValue::from_serialize(&value) {
                Ok(value) => BrowserHostStreamItem::Item(value),
                Err(error) => BrowserHostStreamItem::Item(typed_encode_error(error.to_string())),
            },
            None => BrowserHostStreamItem::End,
        };
        Box::pin(std::future::ready(item))
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

#[cfg(test)]
mod tests {
    use exs_abi::BuiltinHostOperation;

    use super::{BrowserHostFunctionRegistry, BrowserRegistryError, ExsValue};

    /// Ensures browser registration rejects every runner-provided Host operation.
    #[test]
    fn rejects_every_reserved_host_name() {
        for operation in BuiltinHostOperation::ALL {
            let name = operation.host_name();
            let mut registry = BrowserHostFunctionRegistry::new();
            assert_eq!(
                registry.fn_sync_raw(name, |_| ExsValue::None),
                Err(BrowserRegistryError::ReservedName(name.to_owned()))
            );
        }
    }
}
