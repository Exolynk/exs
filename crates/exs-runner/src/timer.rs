//! The timer that wakes executions: `Host::sleep` and the wall-clock deadline of executions
//! that wait for host operations.
//!
//! An application passes the timer of its async runtime, such as `tokio::time::sleep_until`,
//! through [`crate::ServerRunnerConfig::timer`]. Without one, the runner uses its own: a single
//! process-wide thread, started on first use, that serves every execution.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::Instant;

/// Process-wide scheduler used by native Host timers.
static TIMER: OnceLock<Result<Arc<TimerService>, String>> = OnceLock::new();

/// Wakes executions at points in time.
///
/// Implement it with the timer of the application's async runtime, for example with tokio:
///
/// ```ignore
/// struct TokioTimer;
///
/// impl RunnerTimer for TokioTimer {
///     fn sleep_until(&self, deadline: Instant) -> Pin<Box<dyn Future<Output = ()> + Send>> {
///         Box::pin(tokio::time::sleep_until(deadline.into()))
///     }
/// }
/// ```
pub trait RunnerTimer: Send + Sync {
    /// Returns a future that completes at `deadline`, or at once when it has passed.
    fn sleep_until(&self, deadline: Instant) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

/// The runner's own timer: one process-wide thread, started on first use.
pub(crate) struct ThreadTimer;

impl RunnerTimer for ThreadTimer {
    /// Schedules the wake-up on the shared timer thread. When the thread cannot start, the
    /// future completes at once, which ends waiting early instead of never.
    fn sleep_until(&self, deadline: Instant) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        match service() {
            Ok(timer) => Box::pin(timer.schedule(deadline)),
            Err(_) => Box::pin(std::future::ready(())),
        }
    }
}

/// Returns the shared timer service, starting it on first use.
fn service() -> Result<Arc<TimerService>, String> {
    match TIMER.get_or_init(|| {
        TimerService::new()
            .map(Arc::new)
            .map_err(|error| error.to_string())
    }) {
        Ok(timer) => Ok(Arc::clone(timer)),
        Err(error) => Err(error.clone()),
    }
}

/// One shared timer scheduler backed by a single native thread.
struct TimerService {
    /// Scheduled deadlines and their cancellation-aware completion states.
    state: Arc<(Mutex<TimerState>, Condvar)>,
}

impl TimerService {
    /// Creates the scheduler and starts its single event-loop thread.
    fn new() -> std::io::Result<Self> {
        let state = Arc::new((Mutex::new(TimerState::default()), Condvar::new()));
        let timer_state = Arc::clone(&state);
        thread::Builder::new()
            .name("exs-timer".to_owned())
            .spawn(move || run_timer(timer_state))?;
        Ok(Self { state })
    }

    /// Schedules one completion at `deadline` and returns its cancellation-aware future.
    fn schedule(self: &Arc<Self>, deadline: Instant) -> TimerSleep {
        let completion = Arc::new(TimerCompletion::default());
        let identifier = self.register(deadline, Arc::clone(&completion));
        TimerSleep {
            completion,
            registration: Some(TimerRegistration {
                timer: Arc::clone(self),
                identifier,
            }),
        }
    }

    /// Registers `completion` to complete at `deadline` and returns its identifier.
    fn register(&self, deadline: Instant, completion: Arc<TimerCompletion>) -> u64 {
        let (lock, condition) = &*self.state;
        let mut state = lock_state(lock);
        let identifier = state.next_identifier;
        state.next_identifier = state.next_identifier.wrapping_add(1);
        state.pending.insert(identifier, deadline);
        state.deadlines.insert((deadline, identifier), completion);
        condition.notify_one();
        identifier
    }

    /// Removes one unfinished timer so its deadline no longer retains state.
    fn cancel(&self, identifier: u64) {
        let (lock, condition) = &*self.state;
        let mut state = lock_state(lock);
        if let Some(deadline) = state.pending.remove(&identifier) {
            state.deadlines.remove(&(deadline, identifier));
            condition.notify_one();
        }
    }
}

/// Mutable scheduler state protected by the timer mutex.
struct TimerState {
    /// Identifier allocated to the next scheduled completion.
    next_identifier: u64,
    /// Deadlines indexed by their cancellation registration.
    pending: HashMap<u64, Instant>,
    /// Completion states sorted by their deadline and cancellation registration.
    deadlines: BTreeMap<(Instant, u64), Arc<TimerCompletion>>,
}

impl Default for TimerState {
    /// Creates an empty scheduler state.
    fn default() -> Self {
        Self {
            next_identifier: 1,
            pending: HashMap::new(),
            deadlines: BTreeMap::new(),
        }
    }
}

/// Shared completion signal and latest blocked executor waker.
#[derive(Default)]
struct TimerCompletion {
    /// Whether the timer has expired and its sleep may complete.
    completed: AtomicBool,
    /// Latest executor waker to notify when the timer expires.
    waker: Mutex<Option<Waker>>,
}

impl TimerCompletion {
    /// Marks the timer as complete and wakes its latest poller.
    fn complete(&self) {
        self.completed.store(true, Ordering::Release);
        let mut waker = lock_waker(&self.waker);
        if let Some(waker) = waker.take() {
            waker.wake();
        }
    }
}

/// A future that resolves when its shared timer reaches its deadline.
pub(crate) struct TimerSleep {
    /// Completion state set by the timer thread.
    completion: Arc<TimerCompletion>,
    /// Registration removed when this future completes or is dropped.
    registration: Option<TimerRegistration>,
}

impl Future for TimerSleep {
    type Output = ();

    /// Polls the completion state without blocking the runner executor.
    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.completion.completed.load(Ordering::Acquire) {
            self.registration.take();
            return Poll::Ready(());
        }
        let mut waker = lock_waker(&self.completion.waker);
        *waker = Some(context.waker().clone());
        let completed = self.completion.completed.load(Ordering::Acquire);
        drop(waker);
        if completed {
            self.registration.take();
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

/// Cancellation registration owned by one scheduled timer future.
struct TimerRegistration {
    /// Shared scheduler that owns the pending timer entry.
    timer: Arc<TimerService>,
    /// Identifier used to remove the pending timer entry.
    identifier: u64,
}

impl Drop for TimerRegistration {
    /// Removes the timer as soon as its future completes or is dropped.
    fn drop(&mut self) {
        self.timer.cancel(self.identifier);
    }
}

/// Runs the shared scheduler until the process exits.
fn run_timer(state: Arc<(Mutex<TimerState>, Condvar)>) {
    loop {
        let completion = {
            let (lock, condition) = &*state;
            let mut timer_state = lock_state(lock);
            loop {
                let Some((&(deadline, _), _)) = timer_state.deadlines.first_key_value() else {
                    timer_state = condition
                        .wait(timer_state)
                        .unwrap_or_else(PoisonError::into_inner);
                    continue;
                };
                let now = Instant::now();
                if deadline > now {
                    let wait = deadline.saturating_duration_since(now);
                    let (next_state, _) = condition
                        .wait_timeout(timer_state, wait)
                        .unwrap_or_else(PoisonError::into_inner);
                    timer_state = next_state;
                    continue;
                }
                let Some(((.., identifier), completion)) = timer_state.deadlines.pop_first() else {
                    continue;
                };
                timer_state.pending.remove(&identifier);
                break completion;
            }
        };
        completion.complete();
    }
}

/// Locks the shared scheduler state while recovering consistently after a panic.
fn lock_state(state: &Mutex<TimerState>) -> MutexGuard<'_, TimerState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Locks a timer waker slot while recovering consistently after a panic.
fn lock_waker(waker: &Mutex<Option<Waker>>) -> MutexGuard<'_, Option<Waker>> {
    waker.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Removes a long-running sleep from the shared scheduler when its future is dropped.
    #[test]
    fn dropping_sleep_cancels_its_timer_registration() {
        let timer = Arc::new(TimerService::new().unwrap());
        let sleep = timer.schedule(Instant::now() + std::time::Duration::from_secs(60));
        let identifier = sleep.registration.as_ref().unwrap().identifier;
        {
            let (lock, _) = &*timer.state;
            assert!(lock_state(lock).pending.contains_key(&identifier));
        }
        drop(sleep);
        let (lock, _) = &*timer.state;
        assert!(!lock_state(lock).pending.contains_key(&identifier));
    }
}
