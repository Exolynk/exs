//! Registered synchronous and asynchronous host-function implementations.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use exs_abi::ExsValue;

/// The owned future returned by an asynchronous host function.
pub type HostFuture = Pin<Box<dyn Future<Output = ExsValue> + Send>>;

/// The result of requesting one value from a host-owned pull stream.
pub enum HostStreamItem {
    /// One stream value is available.
    Item(ExsValue),
    /// The stream has no remaining values.
    End,
}

/// The owned future returned when advancing one host-owned pull stream.
pub type HostStreamFuture = Pin<Box<dyn Future<Output = HostStreamItem> + Send>>;

/// A single-consumer host source that yields values on demand.
pub trait HostStream: Send {
    /// Asynchronously produces one item or reports the end of the source.
    fn next(&mut self) -> HostStreamFuture;
}

/// Opens one host-owned pull stream from ordered ExS arguments.
pub trait HostStreamFunction: Send + Sync {
    /// Creates a fresh stream instance for one ExS invocation.
    fn open(&self, arguments: Vec<ExsValue>) -> Result<Box<dyn HostStream>, ExsValue>;
}

impl<Function, Stream> HostStreamFunction for Function
where
    Function: Fn(Vec<ExsValue>) -> Result<Stream, ExsValue> + Send + Sync,
    Stream: HostStream + 'static,
{
    fn open(&self, arguments: Vec<ExsValue>) -> Result<Box<dyn HostStream>, ExsValue> {
        self(arguments).map(|stream| Box::new(stream) as Box<dyn HostStream>)
    }
}

/// A synchronous host function registered under a runner-owned static name.
pub trait SyncHostFunction: Send + Sync {
    /// Runs the function with its ordered ExS arguments.
    fn call(&self, arguments: Vec<ExsValue>) -> ExsValue;
}

impl<Function> SyncHostFunction for Function
where
    Function: Fn(Vec<ExsValue>) -> ExsValue + Send + Sync,
{
    fn call(&self, arguments: Vec<ExsValue>) -> ExsValue {
        self(arguments)
    }
}

/// An asynchronous host function registered under a runner-owned static name.
pub trait AsyncHostFunction: Send + Sync {
    /// Starts the function with its ordered ExS arguments.
    fn call(&self, arguments: Vec<ExsValue>) -> HostFuture;
}

impl<Function, FutureValue> AsyncHostFunction for Function
where
    Function: Fn(Vec<ExsValue>) -> FutureValue + Send + Sync,
    FutureValue: Future<Output = ExsValue> + Send + 'static,
{
    fn call(&self, arguments: Vec<ExsValue>) -> HostFuture {
        Box::pin(self(arguments))
    }
}

/// The result of starting one registered host function.
pub enum HostCall {
    /// A synchronous function completed without suspending.
    Ready(ExsValue),
    /// An asynchronous function must be completed by the runner in a later phase.
    Pending(HostFuture),
}

/// One host-function implementation stored by the registry.
#[derive(Clone)]
pub(crate) enum RegisteredHostFunction {
    /// A synchronously executed implementation.
    Sync(Arc<dyn SyncHostFunction>),
    /// An asynchronously executed implementation.
    Async(Arc<dyn AsyncHostFunction>),
}

impl RegisteredHostFunction {
    /// Starts this host function with ordered ExS arguments.
    pub(crate) fn start(&self, arguments: Vec<ExsValue>) -> HostCall {
        match self {
            Self::Sync(function) => HostCall::Ready(function.call(arguments)),
            Self::Async(function) => HostCall::Pending(function.call(arguments)),
        }
    }
}
