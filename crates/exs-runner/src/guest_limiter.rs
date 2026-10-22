//! The resource limiter of one execution.
//!
//! Every growth beyond the configured limits traps, so the runner reports the exceeded limit.
//! A compiled module instead sees its linear memory refuse to grow, as the WebAssembly
//! specification allows, so its collector can reclaim memory first; the refusal is recorded,
//! and a later trap is reported as the exceeded memory limit.

use wasmtime::{ResourceLimiter, StoreLimits, StoreLimitsBuilder};

use crate::limits::ExecutionLimits;

/// Limits of one execution's instance, memory and tables.
pub(crate) struct GuestLimiter {
    /// The trapping Wasmtime limits.
    limits: StoreLimits,
    /// Byte size the linear memory may reach.
    max_memory_bytes: usize,
    /// Whether memory growth beyond the limit is refused instead of trapping.
    soft_memory: bool,
    /// Whether memory growth was refused.
    memory_refused: bool,
}

impl GuestLimiter {
    /// Creates the limiter of `limits`, which traps on every growth beyond them.
    pub(crate) fn new(limits: &ExecutionLimits) -> Self {
        Self {
            limits: StoreLimitsBuilder::new()
                .memory_size(limits.max_memory_bytes)
                .table_elements(limits.max_table_elements)
                .instances(1)
                .tables(limits.max_tables)
                .memories(limits.max_memories)
                .trap_on_grow_failure(true)
                .build(),
            max_memory_bytes: limits.max_memory_bytes,
            soft_memory: false,
            memory_refused: false,
        }
    }

    /// Makes memory growth beyond the limit fail instead of trapping.
    pub(crate) fn refuse_memory_growth(&mut self) {
        self.soft_memory = true;
    }

    /// Returns whether memory growth was refused.
    pub(crate) const fn memory_refused(&self) -> bool {
        self.memory_refused
    }
}

impl ResourceLimiter for GuestLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if !self.soft_memory {
            return self.limits.memory_growing(current, desired, maximum);
        }
        if desired > self.max_memory_bytes || maximum.is_some_and(|maximum| desired > maximum) {
            self.memory_refused = true;
            return Ok(false);
        }
        Ok(true)
    }

    fn memory_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        if self.soft_memory {
            self.memory_refused = true;
            return Ok(());
        }
        self.limits.memory_grow_failed(error)
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.limits.table_growing(current, desired, maximum)
    }

    fn table_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        self.limits.table_grow_failed(error)
    }

    fn instances(&self) -> usize {
        self.limits.instances()
    }

    fn tables(&self) -> usize {
        self.limits.tables()
    }

    fn memories(&self) -> usize {
        self.limits.memories()
    }
}
