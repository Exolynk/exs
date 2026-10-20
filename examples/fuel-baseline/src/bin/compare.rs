//! Compares the fuel of every ExS fuel scenario with its Rust baseline.
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
        let runner = ServerRunner::new(
            64 * 1024 * 1024,
            FUEL_BUDGET,
            Duration::from_secs(60),
            ProtectionLevel::Standard,
        )
        .with_compiler_strategy(CompilerStrategy::Winch);
        let mut measurements = Vec::new();
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
        }
        print_table(&measurements);
        Ok(())
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
