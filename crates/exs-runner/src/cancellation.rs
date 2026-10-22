//! Cooperative cancellation for one running ExS execution.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

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
}

impl Default for CancellationState {
    /// Creates an uncancelled state without registered host-future wakers.
    fn default() -> Self {
        Self {
            cancelled: false,
            next_waker_id: 1,
            wakers: Vec::new(),
        }
    }
}

impl ExecutionCancellation {
    /// Creates an uncancelled execution control.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation and wakes blocked host-future pollers. Running guest code stops
    /// at its next yield to the executor.
    pub fn cancel(&self) {
        let wakers = {
            let mut state = self.state();
            if state.cancelled {
                return;
            }
            state.cancelled = true;
            std::mem::take(&mut state.wakers)
        };
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

    /// Locks the shared cancellation state, recovering consistently after a panic.
    fn state(&self) -> MutexGuard<'_, CancellationState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Why a host future ended before its operation completed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Interruption {
    /// Cancellation was requested.
    Cancelled,
    /// The execution deadline was reached.
    Timeout,
}

/// Future wrapper that resolves when its host future completes, cancellation is requested, or
/// the execution deadline is reached.
pub(crate) struct CancellableHostFuture<'cancellation> {
    /// The runner-owned pending host future.
    future: HostFuture,
    /// Caller-owned cancellation control for this execution.
    cancellation: &'cancellation ExecutionCancellation,
    /// Runner-owned wall-clock deadline for this root execution.
    deadline: &'cancellation ExecutionDeadline,
    /// Timer future of the deadline, created once the host future is pending.
    expiry: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    /// Registered wake-up slot for this future, when it is pending.
    registration: Option<u64>,
}

impl<'cancellation> CancellableHostFuture<'cancellation> {
    /// Wraps one pending host future with the execution cancellation control and deadline.
    pub(crate) fn new(
        future: HostFuture,
        cancellation: &'cancellation ExecutionCancellation,
        deadline: &'cancellation ExecutionDeadline,
    ) -> Self {
        Self {
            future,
            cancellation,
            deadline,
            expiry: None,
            registration: None,
        }
    }

    /// Releases the cancellation waker and reports why the host future ended.
    fn interrupted(&mut self, interruption: Interruption) -> Poll<Result<ExsValue, Interruption>> {
        self.cancellation.unregister_waker(&mut self.registration);
        Poll::Ready(Err(interruption))
    }
}

impl Future for CancellableHostFuture<'_> {
    type Output = Result<ExsValue, Interruption>;

    /// Polls the host future while cancellation and the deadline wake and complete it.
    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.cancellation.is_cancelled() {
            return self.interrupted(Interruption::Cancelled);
        }
        if self.deadline.is_expired() {
            return self.interrupted(Interruption::Timeout);
        }
        let this = &mut *self;
        this.cancellation
            .register_waker(&mut this.registration, context.waker());
        if this.cancellation.is_cancelled() {
            return this.interrupted(Interruption::Cancelled);
        }
        if let Poll::Ready(value) = this.future.as_mut().poll(context) {
            this.cancellation.unregister_waker(&mut this.registration);
            return Poll::Ready(Ok(value));
        }
        // The timer is authoritative: it may signal the deadline slightly before the clock.
        let deadline = this.deadline;
        let expiry = this.expiry.get_or_insert_with(|| deadline.expiry());
        if expiry.as_mut().poll(context).is_ready() {
            return this.interrupted(Interruption::Timeout);
        }
        Poll::Pending
    }
}

impl Drop for CancellableHostFuture<'_> {
    /// Releases the registered waker when the runner drops an unfinished execution.
    fn drop(&mut self) {
        self.cancellation.unregister_waker(&mut self.registration);
    }
}
