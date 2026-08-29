//! Host-function registration and lookup for one server runner.

use std::collections::HashMap;
use std::fmt;
#[cfg(feature = "serde")]
use std::future::{Future, ready};
use std::sync::Arc;

use exs_abi::{ErrorSeverity, ExsError, ExsValue, is_reserved_host_name};
#[cfg(feature = "serde")]
use serde::{Serialize, de::DeserializeOwned};

use crate::host_function::{RegisteredHostFunction, SyncHostFunction};
use crate::{AsyncHostFunction, HostCall, HostStream, HostStreamFunction};

/// An error caused by host-function registration or lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryError {
    /// A registration name was empty.
    EmptyName,
    /// A registration attempted to replace an existing static host name.
    DuplicateName(String),
    /// A registration attempted to claim a runner-internal host name.
    ReservedName(String),
    /// A host call referenced an unregistered static host name.
    UnknownName(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
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

impl std::error::Error for RegistryError {}

/// Runner-owned mappings from static host names to host-function implementations.
#[derive(Clone, Default)]
pub struct HostFunctionRegistry {
    functions: HashMap<String, RegisteredHostFunction>,
    streams: HashMap<String, Arc<dyn HostStreamFunction>>,
}

impl HostFunctionRegistry {
    /// Creates an empty host-function registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one typed synchronous implementation under a static host name.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is empty or already registered.
    #[cfg(feature = "serde")]
    pub fn fn_sync<Request, Response, Function>(
        &mut self,
        name: impl Into<String>,
        function: Function,
    ) -> Result<(), RegistryError>
    where
        Request: DeserializeOwned + Send + 'static,
        Response: Serialize + Send + 'static,
        Function: Fn(Request) -> Result<Response, ExsError> + Send + Sync + 'static,
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

    /// Registers one low-level synchronous implementation under a static host name.
    pub fn fn_sync_raw(
        &mut self,
        name: impl Into<String>,
        function: impl SyncHostFunction + 'static,
    ) -> Result<(), RegistryError> {
        self.insert(
            name.into(),
            RegisteredHostFunction::Sync(Arc::new(function)),
        )
    }

    /// Registers one typed asynchronous implementation under a static host name.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is empty or already registered.
    #[cfg(feature = "serde")]
    pub fn fn_async<Request, Response, Function, FutureValue>(
        &mut self,
        name: impl Into<String>,
        function: Function,
    ) -> Result<(), RegistryError>
    where
        Request: DeserializeOwned + Send + 'static,
        Response: Serialize + Send + 'static,
        Function: Fn(Request) -> FutureValue + Send + Sync + 'static,
        FutureValue: Future<Output = Result<Response, ExsError>> + Send + 'static,
    {
        let function = Arc::new(function);
        self.fn_async_raw(name, move |arguments| {
            let request = decode_typed_request(arguments);
            let function = Arc::clone(&function);
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

    /// Registers one low-level asynchronous implementation under a static host name.
    pub fn fn_async_raw(
        &mut self,
        name: impl Into<String>,
        function: impl AsyncHostFunction + 'static,
    ) -> Result<(), RegistryError> {
        self.insert(
            name.into(),
            RegisteredHostFunction::Async(Arc::new(function)),
        )
    }

    /// Registers one low-level pull-stream factory under a static host name.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is empty or already registered.
    pub fn stream_raw(
        &mut self,
        name: impl Into<String>,
        function: impl HostStreamFunction + 'static,
    ) -> Result<(), RegistryError> {
        let name = name.into();
        if name.is_empty() {
            return Err(RegistryError::EmptyName);
        }
        if is_reserved_host_name(&name) {
            return Err(RegistryError::ReservedName(name));
        }
        if self.functions.contains_key(&name) || self.streams.contains_key(&name) {
            return Err(RegistryError::DuplicateName(name));
        }
        let _previous = self.streams.insert(name, Arc::new(function));
        Ok(())
    }

    /// Registers one typed iterator factory under a static host name.
    ///
    /// Each iterator item is serialized to the ExS boundary value as it is requested. Use
    /// [`Self::stream_raw`] when the source itself must yield asynchronously.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is empty or already registered.
    #[cfg(feature = "serde")]
    pub fn stream<Request, Item, Items, Function>(
        &mut self,
        name: impl Into<String>,
        function: Function,
    ) -> Result<(), RegistryError>
    where
        Request: DeserializeOwned + Send + 'static,
        Item: Serialize + Send + 'static,
        Items: IntoIterator<Item = Item> + Send + 'static,
        Items::IntoIter: Send + 'static,
        Function: Fn(Request) -> Result<Items, ExsError> + Send + Sync + 'static,
    {
        self.stream_raw(name, move |arguments| {
            let request = decode_typed_request(arguments)?;
            let items = function(request).map_err(ExsValue::Error)?;
            Ok(SerializedIterator {
                items: items.into_iter(),
            })
        })
    }

    /// Starts the implementation registered for `name` with ordered ExS arguments.
    ///
    /// # Errors
    ///
    /// Returns an error when no implementation is registered for `name`.
    pub fn start(&self, name: &str, arguments: Vec<ExsValue>) -> Result<HostCall, RegistryError> {
        self.functions
            .get(name)
            .ok_or_else(|| RegistryError::UnknownName(name.to_owned()))
            .map(|function| function.start(arguments))
    }

    /// Opens one fresh pull stream registered for `name`.
    ///
    /// # Errors
    ///
    /// Returns an error or language Error when opening the stream fails.
    pub fn open_stream(
        &self,
        name: &str,
        arguments: Vec<ExsValue>,
    ) -> Result<Box<dyn HostStream>, ExsValue> {
        let function = self
            .streams
            .get(name)
            .ok_or_else(|| missing_stream_error(name))?;
        function.open(arguments)
    }

    /// Inserts one implementation after enforcing the stable registry-name rules.
    fn insert(
        &mut self,
        name: String,
        function: RegisteredHostFunction,
    ) -> Result<(), RegistryError> {
        if name.is_empty() {
            return Err(RegistryError::EmptyName);
        }
        if is_reserved_host_name(&name) {
            return Err(RegistryError::ReservedName(name));
        }
        if self.functions.contains_key(&name) || self.streams.contains_key(&name) {
            return Err(RegistryError::DuplicateName(name));
        }
        let _previous = self.functions.insert(name, function);
        Ok(())
    }
}

/// A typed iterator exposed through the low-level asynchronous stream protocol.
#[cfg(feature = "serde")]
struct SerializedIterator<Items> {
    /// Remaining application values.
    items: Items,
}

#[cfg(feature = "serde")]
impl<Items> HostStream for SerializedIterator<Items>
where
    Items: Iterator + Send + 'static,
    Items::Item: Serialize,
{
    fn next(&mut self) -> crate::HostStreamFuture {
        let item = match self.items.next() {
            Some(value) => match ExsValue::from_serialize(&value) {
                Ok(value) => crate::HostStreamItem::Item(value),
                Err(error) => crate::HostStreamItem::Item(typed_encode_error(error.to_string())),
            },
            None => crate::HostStreamItem::End,
        };
        Box::pin(ready(item))
    }
}

/// Decodes the zero-or-one request argument accepted by one typed host function.
#[cfg(feature = "serde")]
fn decode_typed_request<Request: DeserializeOwned>(
    arguments: Vec<ExsValue>,
) -> Result<Request, ExsValue> {
    let value = match arguments.as_slice() {
        [] => ExsValue::None,
        [value] => value.clone(),
        values => {
            return Err(typed_decode_error(format!(
                "typed host functions expect zero or one argument, received {}",
                values.len()
            )));
        }
    };
    value
        .into_deserialize()
        .map_err(|error| typed_decode_error(error.to_string()))
}

/// Builds one recoverable language error for typed host request decoding.
#[cfg(feature = "serde")]
fn typed_decode_error(message: String) -> ExsValue {
    typed_error("WireDecodeError", message)
}

/// Builds one recoverable language error for typed host response encoding.
#[cfg(feature = "serde")]
fn typed_encode_error(message: String) -> ExsValue {
    typed_error("WireEncodeError", message)
}

/// Builds one recoverable error emitted by the typed host adapter.
#[cfg(feature = "serde")]
fn typed_error(kind: &str, message: String) -> ExsValue {
    ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: kind.to_owned(),
        message,
        data: Box::new(ExsValue::None),
        origin: None,
        trace: Vec::new(),
        cause: None,
    })
}

/// Builds the recoverable language value used for an unregistered dynamic stream name.
fn missing_stream_error(name: &str) -> ExsValue {
    ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: "HostFunctionNotFound".to_owned(),
        message: format!("unknown host stream `{name}`"),
        data: Box::new(ExsValue::None),
        origin: None,
        trace: Vec::new(),
        cause: None,
    })
}
