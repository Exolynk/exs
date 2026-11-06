//! Execution measurements reported alongside a successful root result.

use exs_abi::ExsValue;

/// The result of one successful root execution together with its resource usage.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ExecutionReport {
    /// The value returned by the executed public function.
    pub value: ExsValue,
    /// Deterministic Wasmtime fuel units consumed by guest instructions across all resumptions;
    /// 0 in the browser runner, which meters no fuel.
    pub fuel_consumed: u64,
    /// Wall-clock time spent compiling and instantiating the module before the entry call.
    pub compile_time: std::time::Duration,
    /// Wall-clock time from writing the inputs to the decoded result.
    ///
    /// Covers every resumption, including time spent waiting for host futures, but no module
    /// compilation or instantiation.
    pub execution_time: std::time::Duration,
}
