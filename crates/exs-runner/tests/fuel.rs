//! Fuel baselines for typical small `ExS` scripts.
//!
//! Each test asserts an upper fuel ceiling. When an optimisation lowers the consumed fuel, lower
//! the matching ceiling by hand so later regressions are caught. The scenarios live in
//! `fuel/scenarios.rs`, which the Rust baseline comparison in `examples/fuel-baseline` shares.
//!
//! Fuel accounting differs slightly per code generator, so these tests pin Winch. Instantiation
//! fuel also differs per platform: Linux maps data segments copy-on-write without fuel, while
//! other platforms charge their initialization, so the ceilings are measured on macOS.

#[path = "fuel/scenarios.rs"]
#[allow(dead_code)] // The comparison binary also uses the scenario list.
mod scenarios;
mod support;

use std::time::Duration;

use exs_runner::{
    CompilerStrategy, ExecutionCancellation, ExecutionReport, ProtectionLevel, ServerRunner,
};
use scenarios::Scenario;
use support::{block_on, compile_source};

/// Fuel budget of the measuring runner, far above every ceiling.
const FUEL_BUDGET: u64 = 1_000_000_000;
/// Fuel ceiling for an empty function, which measures pure ABI and runtime overhead.
const EMPTY_MAIN_FUEL: u64 = 68_721;
/// Fuel ceiling for a show function that only returns `true`.
const SHOW_TRUE_FUEL: u64 = 68_739;
/// Fuel ceiling for a show function comparing fields of a host-supplied record object.
const SHOW_COMPARE_RECORD_FUEL: u64 = 76_209;
/// Fuel ceiling for concatenating one field of 10 host-supplied objects.
const CONCAT_10_OBJECTS_FUEL: u64 = 130_349;
/// Fuel ceiling for concatenating one field of 100 host-supplied objects.
const CONCAT_100_OBJECTS_FUEL: u64 = 603_244;
/// Fuel ceiling for 1000 iterations of an integer arithmetic loop.
const INT_LOOP_1000_FUEL: u64 = 306_636;
/// Fuel ceiling for 100 iterations of a resumable `for` loop with a comparison.
const ASYNC_INT_LOOP_100_FUEL: u64 = 92_129;
/// Fuel ceiling for 1000 iterations of an empty resumable `while` loop.
const ASYNC_EMPTY_WHILE_1000_FUEL: u64 = 204_706;
/// Fuel ceiling for 1000 iterations of a resumable integer arithmetic `while` loop.
const ASYNC_INT_WHILE_1000_FUEL: u64 = 367_725;
/// Fuel ceiling for 1000 iterations of an empty `for` loop over a Range.
const FOR_RANGE_1000_FUEL: u64 = 109_602;
/// Fuel ceiling for 1000 iterations of an empty `for` loop over a Range held in a variable.
const FOR_RANGE_VARIABLE_1000_FUEL: u64 = 108_219;
/// Fuel ceiling for 100 calls of a small function with a `match`.
const ASYNC_CALL_LOOP_100_FUEL: u64 = 101_363;
/// Fuel ceiling for 100 calls of a small resumable function.
const RESUMABLE_CALL_LOOP_100_FUEL: u64 = 165_912;
/// Fuel ceiling for 1000 iterations building one formatted line with two Ints.
const FORMAT_LOOP_1000_FUEL: u64 = 2_270_864;
/// Fuel ceiling for 6000 short-lived Lists, enough allocations to trigger collections.
const LIST_CHURN_6000_FUEL: u64 = 8_029_192;
/// Fuel ceiling for 1000 calls of a small typed function.
const SYNC_CALL_LOOP_1000_FUEL: u64 = 370_630;
/// Fuel ceiling for parsing and formatting 100 RFC 3339 DateTimes.
const DATETIME_ROUNDTRIP_100_FUEL: u64 = 6_576_853;
/// Fuel ceiling for the Exolynk-style round trip of 20 records.
const RECORD_ROUNDTRIP_20_FUEL: u64 = 4_601_988;

/// Compiles and executes one scenario with a Winch runner and returns its execution report.
fn execute_report(scenario: &Scenario) -> ExecutionReport {
    let compiled = compile_source(scenario.source);
    // A large budget lets ceilings, not the default budget, catch regressions.
    let runner = ServerRunner::new(
        64 * 1024 * 1024,
        FUEL_BUDGET,
        Duration::from_secs(60),
        ProtectionLevel::Standard,
    )
    .with_compiler_strategy(CompilerStrategy::Winch);
    let cancellation = ExecutionCancellation::new();
    let inputs = (scenario.inputs)();
    match block_on(runner.execute_with_report(&compiled.wasm, "main", &inputs, &cancellation)) {
        Ok(report) => report,
        Err(error) => panic!("{} execution failed: {error}", scenario.name),
    }
}

/// Runs one scenario and asserts its expected result and that fuel stays within the ceiling.
fn assert_fuel(scenario: &Scenario, ceiling: u64) {
    let report = execute_report(scenario);
    println!("{}: {} fuel", scenario.name, report.fuel_consumed);
    assert_eq!(
        report.value,
        (scenario.expected)(),
        "{} returned an unexpected value",
        scenario.name
    );
    assert!(
        report.fuel_consumed <= ceiling,
        "{} consumed {} fuel, ceiling is {ceiling}",
        scenario.name,
        report.fuel_consumed
    );
}

/// Measures the `empty_main` scenario.
#[test]
fn fuel_empty_main() {
    assert_fuel(&scenarios::EMPTY_MAIN, EMPTY_MAIN_FUEL);
}

/// Measures the `show_true` scenario.
#[test]
fn fuel_show_true() {
    assert_fuel(&scenarios::SHOW_TRUE, SHOW_TRUE_FUEL);
}

/// Measures the `show_compare_record` scenario.
#[test]
fn fuel_show_compare_record() {
    assert_fuel(&scenarios::SHOW_COMPARE_RECORD, SHOW_COMPARE_RECORD_FUEL);
}

/// Measures the `concat_10_objects` scenario.
#[test]
fn fuel_concat_10_objects() {
    assert_fuel(&scenarios::CONCAT_10_OBJECTS, CONCAT_10_OBJECTS_FUEL);
}

/// Measures the `concat_100_objects` scenario.
#[test]
fn fuel_concat_100_objects() {
    assert_fuel(&scenarios::CONCAT_100_OBJECTS, CONCAT_100_OBJECTS_FUEL);
}

/// Measures the `int_loop_1000` scenario.
#[test]
fn fuel_int_loop_1000() {
    assert_fuel(&scenarios::INT_LOOP_1000, INT_LOOP_1000_FUEL);
}

/// Measures the `async_int_loop_100` scenario.
#[test]
fn fuel_async_int_loop_100() {
    assert_fuel(&scenarios::ASYNC_INT_LOOP_100, ASYNC_INT_LOOP_100_FUEL);
}

/// Measures the `async_empty_while_1000` scenario.
#[test]
fn fuel_async_empty_while_1000() {
    assert_fuel(
        &scenarios::ASYNC_EMPTY_WHILE_1000,
        ASYNC_EMPTY_WHILE_1000_FUEL,
    );
}

/// Measures the `async_int_while_1000` scenario.
#[test]
fn fuel_async_int_while_1000() {
    assert_fuel(&scenarios::ASYNC_INT_WHILE_1000, ASYNC_INT_WHILE_1000_FUEL);
}

/// Measures the `for_range_1000` scenario.
#[test]
fn fuel_for_range_1000() {
    assert_fuel(&scenarios::FOR_RANGE_1000, FOR_RANGE_1000_FUEL);
}

/// Measures the `for_range_variable_1000` scenario.
#[test]
fn fuel_for_range_variable_1000() {
    assert_fuel(
        &scenarios::FOR_RANGE_VARIABLE_1000,
        FOR_RANGE_VARIABLE_1000_FUEL,
    );
}

/// Measures the `async_call_loop_100` scenario.
#[test]
fn fuel_async_call_loop_100() {
    assert_fuel(&scenarios::ASYNC_CALL_LOOP_100, ASYNC_CALL_LOOP_100_FUEL);
}

/// Measures the `resumable_call_loop_100` scenario.
#[test]
fn fuel_resumable_call_loop_100() {
    assert_fuel(
        &scenarios::RESUMABLE_CALL_LOOP_100,
        RESUMABLE_CALL_LOOP_100_FUEL,
    );
}

/// Measures the `format_loop_1000` scenario.
#[test]
fn fuel_format_loop_1000() {
    assert_fuel(&scenarios::FORMAT_LOOP_1000, FORMAT_LOOP_1000_FUEL);
}

/// Measures the `list_churn_6000` scenario.
#[test]
fn fuel_list_churn_6000() {
    assert_fuel(&scenarios::LIST_CHURN_6000, LIST_CHURN_6000_FUEL);
}

/// Measures the `sync_call_loop_1000` scenario.
#[test]
fn fuel_sync_call_loop_1000() {
    assert_fuel(&scenarios::SYNC_CALL_LOOP_1000, SYNC_CALL_LOOP_1000_FUEL);
}

/// Measures the `datetime_roundtrip_100` scenario.
#[test]
fn fuel_datetime_roundtrip_100() {
    assert_fuel(
        &scenarios::DATETIME_ROUNDTRIP_100,
        DATETIME_ROUNDTRIP_100_FUEL,
    );
}

/// Measures the `record_roundtrip_20` scenario.
#[test]
fn fuel_record_roundtrip_20() {
    assert_fuel(&scenarios::RECORD_ROUNDTRIP_20, RECORD_ROUNDTRIP_20_FUEL);
}
