//! The configuration of a server runner.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::module_cache::DEFAULT_MODULE_CACHE_CAPACITY;
use crate::{CompilerStrategy, ProtectionLevel, RunnerTimer, SharedModuleCache};

/// The execution policy and engine settings of one server runner.
///
/// Every field has a default suited to untrusted ExS source; override the fields an
/// application needs with struct update syntax.
#[derive(Clone)]
pub struct ServerRunnerConfig {
    /// Maximum linear-memory bytes of one execution.
    pub max_memory_bytes: usize,
    /// Wasmtime fuel available to one execution.
    pub max_fuel: u64,
    /// Wall-clock time one execution may take.
    pub timeout: Duration,
    /// Budget for the runner's host-side resources.
    pub protection: ProtectionLevel,
    /// Code generator that compiles every executed module.
    pub compiler: CompilerStrategy,
    /// Compiled modules a runner without a shared cache keeps for reuse; 0 disables caching.
    pub module_cache_capacity: usize,
    /// Cache of compiled modules shared with other runners, usually every runner of the
    /// application. Create it with [`SharedModuleCache::new`] from a configuration with the same
    /// compiler and protection level; a cache with other engine settings is ignored.
    pub module_cache: Option<SharedModuleCache>,
    /// Timer that wakes `Host::sleep` and executions at their deadline, usually the timer of
    /// the application's async runtime. Without one, the runner starts a single process-wide
    /// timer thread on first use.
    pub timer: Option<Arc<dyn RunnerTimer>>,
}

impl Default for ServerRunnerConfig {
    /// Returns the standard policy: 16 MiB of memory, 10 million fuel, 10 seconds, the standard
    /// protection level, Winch and a private cache of 16 modules.
    fn default() -> Self {
        Self {
            max_memory_bytes: 16 * 1024 * 1024,
            max_fuel: 10_000_000,
            timeout: Duration::from_secs(10),
            protection: ProtectionLevel::Standard,
            compiler: CompilerStrategy::Winch,
            module_cache_capacity: DEFAULT_MODULE_CACHE_CAPACITY,
            module_cache: None,
            timer: None,
        }
    }
}

impl fmt::Debug for ServerRunnerConfig {
    /// Formats every field; the timer shows only whether one is set.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServerRunnerConfig")
            .field("max_memory_bytes", &self.max_memory_bytes)
            .field("max_fuel", &self.max_fuel)
            .field("timeout", &self.timeout)
            .field("protection", &self.protection)
            .field("compiler", &self.compiler)
            .field("module_cache_capacity", &self.module_cache_capacity)
            .field("module_cache", &self.module_cache)
            .field("timer", &self.timer.as_ref().map(|_| "RunnerTimer"))
            .finish()
    }
}

impl PartialEq for ServerRunnerConfig {
    /// Compares every field; timers and module caches are equal when both are unset or the
    /// same instance.
    fn eq(&self, other: &Self) -> bool {
        self.max_memory_bytes == other.max_memory_bytes
            && self.max_fuel == other.max_fuel
            && self.timeout == other.timeout
            && self.protection == other.protection
            && self.compiler == other.compiler
            && self.module_cache_capacity == other.module_cache_capacity
            && match (&self.module_cache, &other.module_cache) {
                (None, None) => true,
                (Some(left), Some(right)) => left.ptr_eq(right),
                _ => false,
            }
            && match (&self.timer, &other.timer) {
                (None, None) => true,
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                _ => false,
            }
    }
}

impl Eq for ServerRunnerConfig {}
