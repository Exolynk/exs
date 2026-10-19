//! Fuel baselines for typical small `ExS` scripts.
//!
//! Each test asserts an upper fuel ceiling. When an optimisation lowers the consumed fuel, lower
//! the matching ceiling by hand so later regressions are caught.

mod support;

use exs_abi::ExsValue;
use exs_runner::{ExecutionCancellation, ExecutionReport, ServerRunner};
use support::{block_on, compile_source};

/// Fuel ceiling for an empty function, which measures pure ABI and runtime overhead.
const EMPTY_MAIN_FUEL: u64 = 69_863;
/// Fuel ceiling for a show function that only returns `true`.
const SHOW_TRUE_FUEL: u64 = 69_918;
/// Fuel ceiling for a show function comparing fields of a host-supplied record object.
const SHOW_COMPARE_RECORD_FUEL: u64 = 85_337;
/// Fuel ceiling for concatenating one field of 10 host-supplied objects.
const CONCAT_10_OBJECTS_FUEL: u64 = 273_067;
/// Fuel ceiling for concatenating one field of 100 host-supplied objects.
const CONCAT_100_OBJECTS_FUEL: u64 = 2_843_878;

/// Script concatenating the `name` field of every supplied object.
const CONCAT_SOURCE: &str = r#"
    fn main(items: List) -> String {
        let output = "";
        for item in items {
            output = output + item.name + ";";
        }
        ret output;
    }
"#;

/// Compiles and executes `main` with the default runner and returns its execution report.
fn execute_report(source: &str, inputs: &[ExsValue]) -> ExecutionReport {
    let compiled = compile_source(source);
    let runner = ServerRunner::default();
    let cancellation = ExecutionCancellation::new();
    match block_on(runner.execute_with_report(&compiled.wasm, "main", inputs, &cancellation)) {
        Ok(report) => report,
        Err(error) => panic!("execution failed: {error}"),
    }
}

/// Asserts the expected result and that consumed fuel stays within the ceiling.
fn assert_fuel(name: &str, report: &ExecutionReport, expected: &ExsValue, ceiling: u64) {
    println!("{name}: {} fuel", report.fuel_consumed);
    assert_eq!(
        &report.value, expected,
        "{name} returned an unexpected value"
    );
    assert!(
        report.fuel_consumed <= ceiling,
        "{name} consumed {} fuel, ceiling is {ceiling}",
        report.fuel_consumed
    );
}

/// Builds `count` host objects with a `name` field, mimicking records loaded outside the script.
fn named_objects(count: usize) -> ExsValue {
    ExsValue::List(
        (0..count)
            .map(|index| {
                ExsValue::Object(vec![
                    ("name".to_owned(), ExsValue::String(format!("item{index}"))),
                    ("count".to_owned(), ExsValue::Int(index as i64)),
                ])
            })
            .collect(),
    )
}

/// Builds the string the concatenation script returns for `count` objects.
fn expected_concat(count: usize) -> ExsValue {
    ExsValue::String((0..count).map(|index| format!("item{index};")).collect())
}

/// Measures the fixed overhead of entering and leaving an empty function.
#[test]
fn fuel_empty_main() {
    let report = execute_report("fn main() {}", &[]);
    assert_fuel("empty_main", &report, &ExsValue::None, EMPTY_MAIN_FUEL);
}

/// Measures a show function that unconditionally returns `true`.
#[test]
fn fuel_show_true() {
    let report = execute_report("fn main() -> Bool { ret true; }", &[]);
    assert_fuel("show_true", &report, &ExsValue::Bool(true), SHOW_TRUE_FUEL);
}

/// Measures a show function comparing a string and an int of a host-supplied record object.
///
/// The ABI carries records as untyped objects, so the parameter is untyped like in real scripts.
#[test]
fn fuel_show_compare_record() {
    let report = execute_report(
        r#"
        fn main(record) -> Bool {
            ret record.name == "active" && record.count > 10;
        }
        "#,
        &[ExsValue::Object(vec![
            ("name".to_owned(), ExsValue::String("active".to_owned())),
            ("count".to_owned(), ExsValue::Int(42)),
        ])],
    );
    assert_fuel(
        "show_compare_record",
        &report,
        &ExsValue::Bool(true),
        SHOW_COMPARE_RECORD_FUEL,
    );
}

/// Measures concatenating one field of 10 host-supplied objects.
#[test]
fn fuel_concat_10_objects() {
    let report = execute_report(CONCAT_SOURCE, &[named_objects(10)]);
    assert_fuel(
        "concat_10_objects",
        &report,
        &expected_concat(10),
        CONCAT_10_OBJECTS_FUEL,
    );
}

/// Measures concatenating one field of 100 host-supplied objects.
#[test]
fn fuel_concat_100_objects() {
    let report = execute_report(CONCAT_SOURCE, &[named_objects(100)]);
    assert_fuel(
        "concat_100_objects",
        &report,
        &expected_concat(100),
        CONCAT_100_OBJECTS_FUEL,
    );
}
