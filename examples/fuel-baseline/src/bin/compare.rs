//! Compares the fuel and wall-clock time of every ExS fuel scenario with its Rust baseline.
//!
//! Run through `compare.nu`, which first builds the Rust guest for `wasm32-unknown-unknown`.

#[cfg(not(target_arch = "wasm32"))]
#[path = "../../../../crates/exs-runner/tests/fuel/scenarios.rs"]
mod scenarios;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::fs;
    use std::future::Future;
    use std::path::PathBuf;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};
    use std::time::Duration;

    use exs_abi::ExsValue;
    use exs_compiler::{CompileOptions, SourceInput, compile};
    use exs_runner::{CompilerStrategy, ExecutionCancellation, ProtectionLevel, ServerRunner};

    use super::scenarios::{ALL, Scenario};

    /// Fuel budget of the measuring runner, far above every scenario.
    const FUEL_BUDGET: u64 = 1_000_000_000;

    /// Executions per scenario, implementation and compiler whose median time is reported.
    const TIMING_RUNS: usize = 7;

    /// Median wall-clock times of one scenario implementation under one compiler.
    #[derive(Clone, Copy)]
    struct Timing {
        /// Module compilation and instantiation.
        compile: Duration,
        /// Execution from the entry call to the decoded result.
        execution: Duration,
    }

    /// Wall-clock times measured for one scenario in both implementations and compilers.
    struct TimingRow {
        /// Scenario name.
        name: &'static str,
        /// ExS and Rust timings under Winch.
        winch: (Timing, Timing),
        /// ExS and Rust timings under Cranelift.
        cranelift: (Timing, Timing),
    }

    /// Fuel measured for one scenario in both implementations.
    struct Measurement {
        /// Scenario name.
        name: &'static str,
        /// Fuel consumed by the ExS script.
        exs: u64,
        /// Fuel consumed by the Rust guest.
        rust: u64,
    }

    /// Measures every scenario and prints the comparison table.
    pub fn run() -> Result<(), String> {
        let path = guest_wasm_path();
        let guest = fs::read(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        let runner = measuring_runner(CompilerStrategy::Winch);
        let cranelift = measuring_runner(CompilerStrategy::Cranelift);
        let mut measurements = Vec::new();
        let mut timings = Vec::new();
        for scenario in ALL {
            let compiled = compile(
                SourceInput {
                    source_id: "test.exs",
                    text: scenario.source,
                },
                CompileOptions::default(),
            )
            .map_err(|error| format!("{} does not compile: {error}", scenario.name))?;
            let exs = measure(&runner, &compiled.wasm, "main", scenario, "ExS")?;
            let rust = measure(&runner, &guest, scenario.name, scenario, "Rust")?;
            measurements.push(Measurement {
                name: scenario.name,
                exs,
                rust,
            });
            timings.push(TimingRow {
                name: scenario.name,
                winch: (
                    time(&runner, &compiled.wasm, "main", scenario)?,
                    time(&runner, &guest, scenario.name, scenario)?,
                ),
                cranelift: (
                    time(&cranelift, &compiled.wasm, "main", scenario)?,
                    time(&cranelift, &guest, scenario.name, scenario)?,
                ),
            });
        }
        print_table(&measurements);
        println!();
        print_timing_table(&timings);
        Ok(())
    }

    /// Creates the runner used for every measurement with one code generator.
    fn measuring_runner(compiler: CompilerStrategy) -> ServerRunner {
        ServerRunner::new(
            64 * 1024 * 1024,
            FUEL_BUDGET,
            Duration::from_secs(60),
            ProtectionLevel::Standard,
        )
        .with_compiler_strategy(compiler)
    }

    /// Executes one implementation `TIMING_RUNS` times and returns its median timings.
    fn time(
        runner: &ServerRunner,
        wasm: &[u8],
        function: &str,
        scenario: &Scenario,
    ) -> Result<Timing, String> {
        let inputs = (scenario.inputs)();
        let mut compile = Vec::with_capacity(TIMING_RUNS);
        let mut execution = Vec::with_capacity(TIMING_RUNS);
        for _ in 0..TIMING_RUNS {
            let report = block_on(runner.execute_with_report(
                wasm,
                function,
                &inputs,
                &ExecutionCancellation::new(),
            ))
            .map_err(|error| format!("timing {} failed: {error}", scenario.name))?;
            compile.push(report.compile_time);
            execution.push(report.execution_time);
        }
        Ok(Timing {
            compile: median(&mut compile),
            execution: median(&mut execution),
        })
    }

    /// Returns the median of a non-empty list of durations.
    fn median(durations: &mut [Duration]) -> Duration {
        durations.sort_unstable();
        durations
            .get(durations.len() / 2)
            .copied()
            .unwrap_or_default()
    }

    /// Prints median execution times in microseconds and ExS compile times in milliseconds.
    fn print_timing_table(rows: &[TimingRow]) {
        println!(
            "median of {TIMING_RUNS} runs; execution in us, compile in ms (ExS / Rust module)"
        );
        println!(
            "{:<26} {:>10} {:>10} {:>7} {:>10} {:>10} {:>7} {:>13} {:>13}",
            "scenario",
            "ExS winch",
            "Rust wnch",
            "factor",
            "ExS crane",
            "Rust crn",
            "factor",
            "compile wnch",
            "compile crn"
        );
        let micros = |duration: Duration| duration.as_secs_f64() * 1e6;
        let millis = |duration: Duration| duration.as_secs_f64() * 1e3;
        for row in rows {
            let (exs_winch, rust_winch) = row.winch;
            let (exs_crane, rust_crane) = row.cranelift;
            println!(
                "{:<26} {:>10.1} {:>10.1} {:>7} {:>10.1} {:>10.1} {:>7} {:>6.1}/{:<6.1} {:>6.1}/{:<6.1}",
                row.name,
                micros(exs_winch.execution),
                micros(rust_winch.execution),
                format_ratio(time_ratio(exs_winch.execution, rust_winch.execution)),
                micros(exs_crane.execution),
                micros(rust_crane.execution),
                format_ratio(time_ratio(exs_crane.execution, rust_crane.execution)),
                millis(exs_winch.compile),
                millis(rust_winch.compile),
                millis(exs_crane.compile),
                millis(rust_crane.compile),
            );
        }
    }

    /// Returns `left / right` for two durations, or `None` when `right` is zero.
    fn time_ratio(left: Duration, right: Duration) -> Option<f64> {
        (!right.is_zero()).then(|| left.as_secs_f64() / right.as_secs_f64())
    }

    /// Executes one implementation of a scenario and returns its fuel after checking the result.
    fn measure(
        runner: &ServerRunner,
        wasm: &[u8],
        function: &str,
        scenario: &Scenario,
        label: &str,
    ) -> Result<u64, String> {
        let inputs = (scenario.inputs)();
        let report = block_on(runner.execute_with_report(
            wasm,
            function,
            &inputs,
            &ExecutionCancellation::new(),
        ))
        .map_err(|error| format!("{label} {} failed: {error}", scenario.name))?;
        let expected: ExsValue = (scenario.expected)();
        if report.value != expected {
            return Err(format!(
                "{label} {} returned {:?}, expected {expected:?}",
                scenario.name, report.value
            ));
        }
        Ok(report.fuel_consumed)
    }

    /// Prints raw and net fuel, net meaning above each implementation's `empty_main`.
    fn print_table(measurements: &[Measurement]) {
        let (exs_base, rust_base) = measurements
            .iter()
            .find(|measurement| measurement.name == "empty_main")
            .map_or((0, 0), |measurement| (measurement.exs, measurement.rust));
        println!(
            "{:<26} {:>12} {:>12} {:>8} {:>12} {:>12} {:>9}",
            "scenario", "ExS fuel", "Rust fuel", "factor", "ExS net", "Rust net", "net fact."
        );
        let mut outliers = Vec::new();
        for measurement in measurements {
            let exs_net = measurement.exs.saturating_sub(exs_base);
            let rust_net = measurement.rust.saturating_sub(rust_base);
            let net_factor = ratio(exs_net, rust_net);
            println!(
                "{:<26} {:>12} {:>12} {:>8} {:>12} {:>12} {:>9}",
                measurement.name,
                measurement.exs,
                measurement.rust,
                format_ratio(ratio(measurement.exs, measurement.rust)),
                exs_net,
                rust_net,
                format_ratio(net_factor)
            );
            if let Some(factor) = net_factor {
                outliers.push((factor, measurement.name));
            }
        }
        outliers.sort_by(|left, right| right.0.total_cmp(&left.0));
        println!();
        println!("largest net factors (ExS work per unit of Rust work):");
        for (factor, name) in outliers.iter().take(5) {
            println!("  {name:<26} {factor:>8.1}x");
        }
    }

    /// Returns `left / right`, or `None` when `right` is zero.
    fn ratio(left: u64, right: u64) -> Option<f64> {
        (right != 0).then(|| left as f64 / right as f64)
    }

    /// Formats one optional ratio.
    fn format_ratio(ratio: Option<f64>) -> String {
        ratio.map_or_else(|| "-".to_owned(), |ratio| format!("{ratio:.1}x"))
    }

    /// Returns the release guest Wasm path built by `compare.nu`.
    fn guest_wasm_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/wasm32-unknown-unknown/release/exs_fuel_baseline.wasm")
    }

    /// Polls one runner future without adding an executor dependency.
    fn block_on<Output>(future: impl Future<Output = Output>) -> Output {
        let mut future = pin!(future);
        let mut context = Context::from_waker(Waker::noop());
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), String> {
    native::run()
}

#[cfg(target_arch = "wasm32")]
fn main() {}
