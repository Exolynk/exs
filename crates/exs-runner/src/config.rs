//! The configuration of a server runner.

use std::time::Duration;

use crate::module_cache::DEFAULT_MODULE_CACHE_CAPACITY;
use crate::{CompilerStrategy, ProtectionLevel};

/// The execution policy and engine settings of one server runner.
///
/// Every field has a default suited to untrusted ExS source; override the fields an
/// application needs with struct update syntax.
#[derive(Clone, Debug, Eq, PartialEq)]
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
    /// Compiled modules kept for reuse; 0 disables caching.
    pub module_cache_capacity: usize,
}

impl Default for ServerRunnerConfig {
    /// Returns the standard policy: 16 MiB of memory, 10 million fuel, 10 seconds, the standard
    /// protection level, Winch and 16 cached modules.
    fn default() -> Self {
        Self {
            max_memory_bytes: 16 * 1024 * 1024,
            max_fuel: 10_000_000,
            timeout: Duration::from_secs(10),
            protection: ProtectionLevel::Standard,
            compiler: CompilerStrategy::Winch,
            module_cache_capacity: DEFAULT_MODULE_CACHE_CAPACITY,
        }
    }
}
