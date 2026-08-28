//! Cooperative cancellation for one running ExS execution.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Waker;

use wasmtime::Engine;

use crate::{ExsValue, HostFuture, deadline::ExecutionDeadline};

/// Cloneable cancellation control supplied to one runner execution.
#[derive(Clone, Default)]
pub struct ExecutionCancellation {
    /// Shared cancellation state and registered executor wakers.
    state: Arc<Mutex<CancellationState>>,
}

/// Mutable shared state for one cancellation control.
struct CancellationState {
    /// Whether cancellation was requested.
    cancelled: bool,
    /// Monotonic identifier assigned to one registered waker.
    next_waker_id: u64,
    /// Executor wakers currently blocked on cancellable host futures.
    wakers: Vec<(u64, Waker)>,
    /// Monotonic identifier assigned to one registered Wasm interruption target.
    next_interrupt_id: u64,
    /// Wasmtime engines whose active guest executions must be interrupted on cancellation.
    interrupts: Vec<(u64, Engine)>,
}

impl Default for CancellationState {
    /// Creates an uncancelled state without registered host-future wakers.
    fn default() -> Self {
        Self {
            cancelled: false,
            next_waker_id: 1,
            wakers: Vec::new(),
            next_interrupt_id: 1,
            interrupts: Vec::new(),
        }
    }
}

impl ExecutionCancellation {
    /// Creates an uncancelled execution control.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation, interrupts guest Wasm, and wakes blocked host-future pollers.
    pub fn cancel(&self) {
        let (interrupts, wakers) = {
            let mut state = self.state();
            if state.cancelled {
                return;
            }
            state.cancelled = true;
            (
                std::mem::take(&mut state.interrupts),
                std::mem::take(&mut state.wakers),
            )
        };
        for (_, engine) in interrupts {
            engine.increment_epoch();
        }
        for (_, waker) in wakers {
            waker.wake();
        }
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state().cancelled
    }

    /// Registers or refreshes the executor waker for one pending host future.
    fn register_waker(&self, registration: &mut Option<u64>, waker: &Waker) {
        let mut state = self.state();
        if state.cancelled {
            return;
        }
        if let Some(identifier) = registration
            && let Some((_, registered)) = state
                .wakers
                .iter_mut()
                .find(|(registered_id, _)| registered_id == identifier)
        {
            if !registered.will_wake(waker) {
                *registered = waker.clone();
            }
            return;
        }
        let identifier = state.next_waker_id;
        let Some(next_identifier) = identifier.checked_add(1) else {
            return;
        };
        state.next_waker_id = next_identifier;
        state.wakers.push((identifier, waker.clone()));
        *registration = Some(identifier);
    }

    /// Removes one completed host future's executor waker.
    fn unregister_waker(&self, registration: &mut Option<u64>) {
        let Some(identifier) = registration.take() else {
            return;
        };
        let mut state = self.state();
        if let Some(position) = state
            .wakers
            .iter()
            .position(|(registered_id, _)| *registered_id == identifier)
        {
            state.wakers.swap_remove(position);
        }
    }

    /// Registers one engine to interrupt if cancellation occurs during guest execution.
    pub(crate) fn register_interrupt(&self, engine: Engine) -> CancellationInterrupt<'_> {
        let mut state = self.state();
        if state.cancelled {
            return CancellationInterrupt {
                cancellation: self,
                registration: None,
            };
        }
        let identifier = state.next_interrupt_id;
        let Some(next_identifier) = identifier.checked_add(1) else {
            return CancellationInterrupt {
                cancellation: self,
                registration: None,
            };
        };
        state.next_interrupt_id = next_identifier;
        state.interrupts.push((identifier, engine));
        CancellationInterrupt {
            cancellation: self,
            registration: Some(identifier),
        }
    }

    /// Removes one guest-Wasm interruption target after its execution returns.
    fn unregister_interrupt(&self, registration: &mut Option<u64>) {
        let Some(identifier) = registration.take() else {
            return;
        };
        let mut state = self.state();
        if let Some(position) = state
            .interrupts
            .iter()
            .position(|(registered_id, _)| *registered_id == identifier)
        {
            state.interrupts.swap_remove(position);
        }
    }

    /// Locks the shared cancellation state, recovering consistently after a panic.
    fn state(&self) -> MutexGuard<'_, CancellationState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Registered guest-Wasm interruption target owned by one runner execution.
pub(crate) struct CancellationInterrupt<'cancellation> {
    /// Caller-owned cancellation control for this execution.
    cancellation: &'cancellation ExecutionCancellation,
    /// Registered engine identifier removed when execution completes.
    registration: Option<u64>,
}

impl CancellationInterrupt<'_> {
    /// Returns whether cancellation was already requested before registration completed.
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

impl Drop for CancellationInterrupt<'_> {
    /// Releases the engine interruption target once guest execution has returned.
    fn drop(&mut self) {
        self.cancellation
            .unregister_interrupt(&mut self.registration);
    }
}

/// Future wrapper that resolves when its host future completes or cancellation is requested.
pub(crate) struct CancellableHostFuture<'cancellation> {
    /// The runner-owned pending host future.
    future: HostFuture,
    /// Caller-owned cancellation control for this execution.
    cancellation: &'cancellation ExecutionCancellation,
    /// Runner-owned wall-clock deadline for this root execution.
    deadline: &'cancellation ExecutionDeadline,
    /// Registered wake-up slot for this future, when it is pending.
    registration: Option<u64>,
    /// Registered wake-up slot for the root deadline, when this future is pending.
    deadline_registration: Option<u64>,
}

impl<'cancellation> CancellableHostFuture<'cancellation> {
    /// Wraps one pending host future with the execution cancellation control.
    pub(crate) fn new(
        future: HostFuture,
        cancellation: &'cancellation ExecutionCancellation,
        deadline: &'cancellation ExecutionDeadline,
    ) -> Self {
        Self {
            future,
            cancellation,
            deadline,
            registration: None,
            deadline_registration: None,
        }
    }
}

impl std::future::Future for CancellableHostFuture<'_> {
    type Output = Result<ExsValue, ()>;

    /// Polls the host future while ensuring cancellation wakes and completes this future.
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if self.cancellation.is_cancelled() {
            self.cancellation.unregister_waker(&mut self.registration);
            self.deadline
                .unregister_waker(&mut self.deadline_registration);
            return std::task::Poll::Ready(Err(()));
        }
        if self.deadline.is_expired() {
            self.cancellation.unregister_waker(&mut self.registration);
            self.deadline
                .unregister_waker(&mut self.deadline_registration);
            return std::task::Poll::Ready(Err(()));
        }
        self.cancellation
            .register_waker(&mut self.registration, context.waker());
        self.deadline
            .register_waker(&mut self.deadline_registration, context.waker());
        if self.cancellation.is_cancelled() {
            self.cancellation.unregister_waker(&mut self.registration);
            self.deadline
                .unregister_waker(&mut self.deadline_registration);
            return std::task::Poll::Ready(Err(()));
        }
        if self.deadline.is_expired() {
            self.cancellation.unregister_waker(&mut self.registration);
            self.deadline
                .unregister_waker(&mut self.deadline_registration);
            return std::task::Poll::Ready(Err(()));
        }
        match self.future.as_mut().poll(context) {
            std::task::Poll::Ready(value) => {
                self.cancellation.unregister_waker(&mut self.registration);
                self.deadline
                    .unregister_waker(&mut self.deadline_registration);
                std::task::Poll::Ready(Ok(value))
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

impl Drop for CancellableHostFuture<'_> {
    /// Releases the registered waker when the runner drops an unfinished execution.
    fn drop(&mut self) {
        self.cancellation.unregister_waker(&mut self.registration);
        self.deadline
            .unregister_waker(&mut self.deadline_registration);
    }
}
