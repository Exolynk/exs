//! Bounded in-memory cache of compiled, pre-linked modules for the native server runner.
//!
//! Compiling a module dominates the wall-clock cost of short executions. The cache owns one
//! Wasmtime engine and keeps the most recently executed modules compiled and linked against the
//! Host ABI, so a repeated execution only creates a fresh store and instance. Every execution
//! still gets its own store with its own memory, fuel, limits and host functions.
//!
//! One cache can be shared by every runner of an application, also by runners created per
//! execution and by executions running concurrently.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use wasmtime::{Config, Engine, InstancePre, Linker, Module};

use crate::host_abi::{self, HostAbiState};
use crate::limits::ExecutionLimits;
use crate::{CompilerStrategy, ServerRunnerConfig};

/// Default number of compiled modules one cache keeps.
pub(crate) const DEFAULT_MODULE_CACHE_CAPACITY: usize = 16;

/// One cached compilation result.
struct CachedModule {
    /// Exact module bytes, compared in full so different modules never share a compilation.
    wasm: Arc<[u8]>,
    /// The compiled module, linked against the Host ABI of the cache's engine.
    instance_pre: InstancePre<HostAbiState>,
}

/// Engine and cached modules shared by every clone of one [`SharedModuleCache`].
struct CacheInner {
    /// Engine every cached module was compiled for, or why it could not be created.
    engine: Result<Engine, String>,
    /// Code generator the engine compiles with.
    compiler: CompilerStrategy,
    /// Native Wasm call-stack bytes the engine allows.
    max_wasm_stack_bytes: usize,
    /// Maximum number of cached modules; 0 disables caching.
    capacity: usize,
    /// Cached modules, most recently used last.
    entries: Mutex<Vec<CachedModule>>,
}

/// Least-recently-used cache of compiled modules that runners can share.
///
/// Create one cache per application with the same [`ServerRunnerConfig`] its runners use and
/// pass it in [`ServerRunnerConfig::module_cache`]. Cloning is cheap and shares the cache.
#[derive(Clone)]
pub struct SharedModuleCache {
    /// The shared engine and modules.
    inner: Arc<CacheInner>,
}

impl SharedModuleCache {
    /// Creates an empty cache for runners with the compiler and protection level of `config`,
    /// holding at most `config.module_cache_capacity` modules.
    #[must_use]
    pub fn new(config: &ServerRunnerConfig) -> Self {
        let limits = ExecutionLimits::new(
            config.max_memory_bytes,
            config.max_fuel,
            config.timeout,
            config.protection,
        );
        Self::with_engine_settings(
            config.compiler,
            limits.max_wasm_stack_bytes,
            config.module_cache_capacity,
        )
    }

    /// Creates an empty cache whose engine uses `compiler` and `max_wasm_stack_bytes`.
    pub(crate) fn with_engine_settings(
        compiler: CompilerStrategy,
        max_wasm_stack_bytes: usize,
        capacity: usize,
    ) -> Self {
        Self {
            inner: Arc::new(CacheInner {
                engine: limited_engine(compiler, max_wasm_stack_bytes),
                compiler,
                max_wasm_stack_bytes,
                capacity,
                entries: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Returns whether this cache's engine matches the given engine settings.
    pub(crate) fn fits(&self, compiler: CompilerStrategy, max_wasm_stack_bytes: usize) -> bool {
        self.inner.compiler == compiler && self.inner.max_wasm_stack_bytes == max_wasm_stack_bytes
    }

    /// Returns the engine every module of this cache is compiled for.
    ///
    /// # Errors
    ///
    /// Returns Wasmtime's message when the engine could not be created.
    pub(crate) fn engine(&self) -> Result<&Engine, String> {
        self.inner.engine.as_ref().map_err(Clone::clone)
    }

    /// Returns `wasm` compiled and linked against the Host ABI, reusing a cached module when
    /// one exists.
    ///
    /// Concurrent misses of the same module may compile it more than once; the cache keeps one.
    ///
    /// # Errors
    ///
    /// Returns Wasmtime's message when the engine is unavailable or the module does not compile
    /// or link.
    pub(crate) fn instance_pre(&self, wasm: &[u8]) -> Result<InstancePre<HostAbiState>, String> {
        if let Some(instance_pre) = self.lookup(wasm) {
            return Ok(instance_pre);
        }
        let engine = self.engine()?;
        let module = Module::new(engine, wasm).map_err(|error| error.to_string())?;
        let mut linker = Linker::new(engine);
        host_abi::define(&mut linker).map_err(|error| error.to_string())?;
        let instance_pre = linker
            .instantiate_pre(&module)
            .map_err(|error| error.to_string())?;
        self.insert(wasm, instance_pre.clone());
        Ok(instance_pre)
    }

    /// Returns the cached module for `wasm` and marks it most recently used.
    fn lookup(&self, wasm: &[u8]) -> Option<InstancePre<HostAbiState>> {
        let mut entries = self.entries();
        let position = entries.iter().position(|entry| *entry.wasm == *wasm)?;
        let entry = entries.remove(position);
        let instance_pre = entry.instance_pre.clone();
        entries.push(entry);
        Some(instance_pre)
    }

    /// Stores one module, evicting the least recently used module when the cache is full.
    fn insert(&self, wasm: &[u8], instance_pre: InstancePre<HostAbiState>) {
        if self.inner.capacity == 0 {
            return;
        }
        let mut entries = self.entries();
        if entries.iter().any(|entry| *entry.wasm == *wasm) {
            return;
        }
        if entries.len() >= self.inner.capacity {
            entries.remove(0);
        }
        entries.push(CachedModule {
            wasm: wasm.into(),
            instance_pre,
        });
    }

    /// Locks the cached modules, recovering them when another execution panicked.
    fn entries(&self) -> std::sync::MutexGuard<'_, Vec<CachedModule>> {
        self.inner
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Returns whether two handles share the same cache.
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Returns the number of cached modules.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries().len()
    }

    /// Returns whether the cache holds no module.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Debug for SharedModuleCache {
    /// Formats the engine settings, the capacity and the number of cached modules.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedModuleCache")
            .field("compiler", &self.inner.compiler)
            .field("max_wasm_stack_bytes", &self.inner.max_wasm_stack_bytes)
            .field("capacity", &self.inner.capacity)
            .field("len", &self.len())
            .finish()
    }
}

/// Creates a native Wasmtime engine instrumented for the runner policy.
fn limited_engine(
    compiler: CompilerStrategy,
    max_wasm_stack_bytes: usize,
) -> Result<Engine, String> {
    let mut configuration = Config::new();
    configuration.strategy(compiler.wasmtime_strategy());
    configuration.consume_fuel(true);
    configuration.max_wasm_stack(max_wasm_stack_bytes);
    // The runner ABI accounts for one linear memory. Threads, function references and
    // exceptions are not compiled into the engine at all.
    configuration.wasm_multi_memory(false);
    configuration.wasm_memory64(false);
    configuration.wasm_shared_everything_threads(false);
    configuration.wasm_gc(false);
    configuration.wasm_stack_switching(false);
    configuration.wasm_custom_page_sizes(false);
    Engine::new(&configuration).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns a valid empty module whose custom section makes its bytes unique.
    fn module_bytes(marker: u8) -> Vec<u8> {
        let mut bytes = b"\0asm\x01\0\0\0".to_vec();
        bytes.extend([0, 3, 1, b'm', marker]);
        bytes
    }

    /// Returns a cache with the standard engine settings and `capacity` slots.
    fn cache(capacity: usize) -> SharedModuleCache {
        SharedModuleCache::new(&ServerRunnerConfig {
            module_cache_capacity: capacity,
            ..ServerRunnerConfig::default()
        })
    }

    /// Keeps one module per distinct byte sequence and evicts the least recently used one.
    #[test]
    fn reuses_and_evicts_compiled_modules() -> Result<(), String> {
        let cache = cache(2);
        let (first, second, third) = (module_bytes(1), module_bytes(2), module_bytes(3));
        cache.instance_pre(&first)?;
        cache.instance_pre(&second)?;
        cache.instance_pre(&first)?;
        assert_eq!(cache.len(), 2);
        cache.instance_pre(&third)?;
        assert_eq!(cache.len(), 2);
        assert!(cache.lookup(&first).is_some());
        assert!(cache.lookup(&second).is_none());
        Ok(())
    }

    /// Compiles every module directly when the capacity is zero.
    #[test]
    fn skips_caching_without_capacity() -> Result<(), String> {
        let cache = cache(0);
        cache.instance_pre(&module_bytes(1))?;
        assert!(cache.is_empty());
        Ok(())
    }

    /// Shares cached modules between clones and only fits matching engine settings.
    #[test]
    fn clones_share_modules_and_settings() -> Result<(), String> {
        let cache = cache(2);
        let clone = cache.clone();
        clone.instance_pre(&module_bytes(1))?;
        assert_eq!(cache.len(), 1);
        assert!(cache.ptr_eq(&clone));
        assert!(cache.fits(CompilerStrategy::Winch, 1024 * 1024));
        assert!(!cache.fits(CompilerStrategy::Cranelift, 1024 * 1024));
        assert!(!cache.fits(CompilerStrategy::Winch, 512 * 1024));
        Ok(())
    }
}
