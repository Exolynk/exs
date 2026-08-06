//! Wall-clock deadline coordination for one native runner execution.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::task::Waker;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use wasmtime::Engine;

/// Shared state observed by the execution thread and its deadline timer.
struct DeadlineState {
    /// Whether the root execution has completed and the timer may exit.
    completed: bool,
    /// Whether the timer reached the configured deadline.
    expired: bool,
    /// Monotonic identifier assigned to each blocked host-future waker.
    next_waker_id: u64,
    /// Executor wakers blocked on host futures for this root execution.
    wakers: Vec<(u64, Waker)>,
}

impl Default for DeadlineState {
    /// Creates an unfinished, unexpired deadline state.
    fn default() -> Self {
        Self {
            completed: false,
            expired: false,
            next_waker_id: 1,
            wakers: Vec::new(),
        }
    }
}

/// Interrupts guest Wasm and wakes pending host futures when one execution expires.
pub(crate) struct ExecutionDeadline {
    /// Monotonic instant from which the execution timeout is measured.
    started_at: Instant,
    /// Configured duration after which the execution must not report success.
    timeout: Duration,
    /// Shared expiry state and pending host-future wakers.
    state: Arc<(Mutex<DeadlineState>, Condvar)>,
    /// Timer thread joined after the root execution finishes.
    timer: Option<JoinHandle<()>>,
}

impl ExecutionDeadline {
    /// Starts a deadline that interrupts `engine` after `timeout`.
    pub(crate) fn new(engine: Engine, timeout: Duration) -> std::io::Result<Self> {
        let started_at = Instant::now();
        let state = Arc::new((Mutex::new(DeadlineState::default()), Condvar::new()));
        let timer_state = Arc::clone(&state);
        let timer = thread::Builder::new()
            .name("exs-runner-deadline".to_owned())
            .spawn(move || {
                let wakers = {
                    let (lock, condition) = &*timer_state;
                    let state = lock_state(lock);
                    let (mut state, wait) = condition
                        .wait_timeout_while(state, timeout, |state| !state.completed)
                        .unwrap_or_else(PoisonError::into_inner);
                    if state.completed || !wait.timed_out() {
                        return;
                    }
                    state.expired = true;
                    std::mem::take(&mut state.wakers)
                };
                engine.increment_epoch();
                for (_, waker) in wakers {
                    waker.wake();
                }
            })?;
        Ok(Self {
            started_at,
            timeout,
            state,
            timer: Some(timer),
        })
    }

    /// Returns whether the configured wall-clock deadline has elapsed.
    pub(crate) fn is_expired(&self) -> bool {
        if self.started_at.elapsed() >= self.timeout {
            return true;
        }
        let (lock, _) = &*self.state;
        lock_state(lock).expired
    }

    /// Registers or refreshes one executor waker while a host future is pending.
    pub(crate) fn register_waker(&self, registration: &mut Option<u64>, waker: &Waker) {
        let (lock, _) = &*self.state;
        let mut state = lock_state(lock);
        if state.expired || state.completed {
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

    /// Removes one completed or dropped host future's executor waker.
    pub(crate) fn unregister_waker(&self, registration: &mut Option<u64>) {
        let Some(identifier) = registration.take() else {
            return;
        };
        let (lock, _) = &*self.state;
        let mut state = lock_state(lock);
        if let Some(position) = state
            .wakers
            .iter()
            .position(|(registered_id, _)| *registered_id == identifier)
        {
            state.wakers.swap_remove(position);
        }
    }
}

impl Drop for ExecutionDeadline {
    /// Stops and joins the timer once the root execution has returned.
    fn drop(&mut self) {
        {
            let (lock, condition) = &*self.state;
            let mut state = lock_state(lock);
            state.completed = true;
            condition.notify_one();
        }
        if let Some(timer) = self.timer.take() {
            let _ = timer.join();
        }
    }
}

/// Locks shared deadline state while recovering consistently after a panic.
fn lock_state(state: &Mutex<DeadlineState>) -> MutexGuard<'_, DeadlineState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}
