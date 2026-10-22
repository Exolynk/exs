//! Wall-clock deadline of one native runner execution.
//!
//! Running guest code yields to the executor regularly, and every poll checks the clock. A
//! future that waits for a host operation also waits for the runner's timer at the deadline,
//! so an operation that never completes still ends the execution in time.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::timer::RunnerTimer;

/// The wall-clock limit of one execution and the timer that signals it.
pub(crate) struct ExecutionDeadline {
    /// Monotonic instant from which the execution timeout is measured.
    started_at: Instant,
    /// Configured duration after which the execution must not report success.
    timeout: Duration,
    /// The runner's timer.
    timer: Arc<dyn RunnerTimer>,
}

impl ExecutionDeadline {
    /// Starts a deadline `timeout` from now that `timer` signals.
    pub(crate) fn new(timer: Arc<dyn RunnerTimer>, timeout: Duration) -> Self {
        Self {
            started_at: Instant::now(),
            timeout,
            timer,
        }
    }

    /// Returns whether the configured wall-clock deadline has elapsed.
    pub(crate) fn is_expired(&self) -> bool {
        self.started_at.elapsed() >= self.timeout
    }

    /// Returns the monotonic instant at which this execution must time out.
    pub(crate) fn expires_at(&self) -> Instant {
        self.started_at
            .checked_add(self.timeout)
            .unwrap_or(self.started_at)
    }

    /// Returns a future that completes when the deadline is reached.
    pub(crate) fn expiry(&self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        self.timer.sleep_until(self.expires_at())
    }
}
