//! Registered synchronous and asynchronous host-function implementations.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use exs_abi::ExsValue;

/// The owned future returned by an asynchronous host function.
pub type HostFuture = Pin<Box<dyn Future<Output = ExsValue> + Send>>;

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
