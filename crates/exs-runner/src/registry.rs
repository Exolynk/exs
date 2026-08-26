//! Host-function registration and lookup for one server runner.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use exs_abi::{
    ErrorSeverity, ExsError, ExsValue, HOST_SLEEP_HOST_NAME, HOST_STREAM_NEXT_HOST_NAME,
    HOST_STREAM_OPEN_HOST_NAME,
};

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

    /// Registers one synchronous implementation under a static host name.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is empty or already registered.
    pub fn register_sync(
        &mut self,
        name: impl Into<String>,
        function: impl SyncHostFunction + 'static,
    ) -> Result<(), RegistryError> {
        self.insert(
            name.into(),
            RegisteredHostFunction::Sync(Arc::new(function)),
        )
    }

    /// Registers one asynchronous implementation under a static host name.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is empty or already registered.
    pub fn register_async(
        &mut self,
        name: impl Into<String>,
        function: impl AsyncHostFunction + 'static,
    ) -> Result<(), RegistryError> {
        self.insert(
            name.into(),
            RegisteredHostFunction::Async(Arc::new(function)),
        )
    }

    /// Registers one pull-stream factory under a static host name.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is empty or already registered.
    pub fn register_stream(
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

/// Returns whether one name is intercepted by the runner Host ABI.
fn is_reserved_host_name(name: &str) -> bool {
    matches!(
        name,
        HOST_SLEEP_HOST_NAME | HOST_STREAM_OPEN_HOST_NAME | HOST_STREAM_NEXT_HOST_NAME
    )
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
