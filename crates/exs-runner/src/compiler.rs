//! Native code generator selection for the Wasmtime server runner.

/// The Wasmtime code generator used to compile a module before execution.
///
/// Every root execution compiles its module, so compile time is part of every start. Winch
/// compiles several times faster, while Cranelift produces faster code for long-running scripts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CompilerStrategy {
    /// Wasmtime's single-pass baseline compiler with fast compilation and slower generated code.
    #[default]
    Winch,
    /// Wasmtime's optimizing compiler with slow compilation and faster generated code.
    Cranelift,
}

impl CompilerStrategy {
    /// Returns the matching Wasmtime compilation strategy.
    pub(crate) fn wasmtime_strategy(self) -> wasmtime::Strategy {
        match self {
            Self::Winch => wasmtime::Strategy::Winch,
            Self::Cranelift => wasmtime::Strategy::Cranelift,
        }
    }
}
