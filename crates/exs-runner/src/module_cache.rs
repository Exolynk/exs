//! Bounded in-memory cache of compiled module artifacts for the native server runner.
//!
//! Compiling a module dominates the wall-clock cost of short executions, while loading a
//! compiled artifact into a fresh engine is cheap. Each execution keeps its own engine, so its
//! deadline and cancellation epochs stay isolated; only the compiled artifact is shared.

use std::sync::{Arc, Mutex, PoisonError};

use wasmtime::{Engine, Module};

use crate::CompilerStrategy;

/// Default number of compiled modules one runner keeps.
pub(crate) const DEFAULT_MODULE_CACHE_CAPACITY: usize = 16;

/// One cached compilation result.
struct CachedModule {
    /// Code generator that produced the artifact.
    compiler: CompilerStrategy,
    /// Exact module bytes, compared in full so different modules never share an artifact.
    wasm: Arc<[u8]>,
    /// Artifact produced by `Module::serialize` for an engine of this runner's configuration.
    artifact: Arc<[u8]>,
}

/// Least-recently-used cache of compiled module artifacts shared by clones of one runner.
#[derive(Clone)]
pub(crate) struct ModuleCache {
    /// Cached modules, most recently used last.
    entries: Arc<Mutex<Vec<CachedModule>>>,
    /// Maximum number of cached modules; 0 disables caching.
    capacity: usize,
}

impl ModuleCache {
    /// Creates an empty cache holding at most `capacity` modules.
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            entries: Arc::new(Mutex::new(Vec::new())),
            capacity,
        }
    }

    /// Returns `wasm` compiled for `engine`, reusing a cached artifact when one exists.
    ///
    /// # Errors
    ///
    /// Returns Wasmtime's error when the module does not compile or an artifact cannot be
    /// loaded.
    pub(crate) fn module(
        &self,
        engine: &Engine,
        compiler: CompilerStrategy,
        wasm: &[u8],
    ) -> wasmtime::Result<Module> {
        if self.capacity == 0 {
            return Module::new(engine, wasm);
        }
        if let Some(artifact) = self.lookup(compiler, wasm) {
            // SAFETY: the artifact was produced in this process by `Module::serialize` for an
            // engine of the same runner, which has the same configuration and compiler.
            return unsafe { Module::deserialize(engine, &artifact) };
        }
        let module = Module::new(engine, wasm)?;
        self.insert(compiler, wasm, module.serialize()?.into());
        Ok(module)
    }

    /// Returns the cached artifact for `wasm` and marks it most recently used.
    fn lookup(&self, compiler: CompilerStrategy, wasm: &[u8]) -> Option<Arc<[u8]>> {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let position = entries
            .iter()
            .position(|entry| entry.compiler == compiler && *entry.wasm == *wasm)?;
        let entry = entries.remove(position);
        let artifact = Arc::clone(&entry.artifact);
        entries.push(entry);
        Some(artifact)
    }

    /// Stores one artifact, evicting the least recently used module when the cache is full.
    fn insert(&self, compiler: CompilerStrategy, wasm: &[u8], artifact: Arc<[u8]>) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if entries
            .iter()
            .any(|entry| entry.compiler == compiler && *entry.wasm == *wasm)
        {
            return;
        }
        if entries.len() >= self.capacity {
            entries.remove(0);
        }
        entries.push(CachedModule {
            compiler,
            wasm: wasm.into(),
            artifact,
        });
    }

    /// Returns the number of cached modules.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
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

    /// Keeps one artifact per distinct module and evicts the least recently used one.
    #[test]
    fn reuses_and_evicts_compiled_modules() -> wasmtime::Result<()> {
        let engine = Engine::default();
        let cache = ModuleCache::new(2);
        let (first, second, third) = (module_bytes(1), module_bytes(2), module_bytes(3));
        cache.module(&engine, CompilerStrategy::Cranelift, &first)?;
        cache.module(&engine, CompilerStrategy::Cranelift, &second)?;
        cache.module(&engine, CompilerStrategy::Cranelift, &first)?;
        assert_eq!(cache.len(), 2);
        cache.module(&engine, CompilerStrategy::Cranelift, &third)?;
        assert_eq!(cache.len(), 2);
        assert!(cache.lookup(CompilerStrategy::Cranelift, &first).is_some());
        assert!(cache.lookup(CompilerStrategy::Cranelift, &second).is_none());
        assert!(cache.lookup(CompilerStrategy::Winch, &first).is_none());
        Ok(())
    }

    /// Compiles every module directly when the capacity is zero.
    #[test]
    fn skips_caching_without_capacity() -> wasmtime::Result<()> {
        let engine = Engine::default();
        let cache = ModuleCache::new(0);
        cache.module(&engine, CompilerStrategy::Cranelift, &module_bytes(1))?;
        assert_eq!(cache.len(), 0);
        Ok(())
    }
}
