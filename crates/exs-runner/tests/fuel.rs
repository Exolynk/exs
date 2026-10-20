//! Fuel baselines for typical small `ExS` scripts.
//!
//! Each test asserts an upper fuel ceiling. When an optimisation lowers the consumed fuel, lower
//! the matching ceiling by hand so later regressions are caught.
//!
//! Fuel accounting differs slightly per code generator, so these tests pin Winch. Instantiation
//! fuel also differs per platform: Linux maps data segments copy-on-write without fuel, while
//! other platforms charge their initialization, so the ceilings are measured on macOS.

mod support;

use exs_abi::ExsValue;
use exs_runner::{CompilerStrategy, ExecutionCancellation, ExecutionReport, ServerRunner};
use support::{block_on, compile_source};

/// Fuel ceiling for an empty function, which measures pure ABI and runtime overhead.
const EMPTY_MAIN_FUEL: u64 = 68_721;
/// Fuel ceiling for a show function that only returns `true`.
const SHOW_TRUE_FUEL: u64 = 68_739;
/// Fuel ceiling for a show function comparing fields of a host-supplied record object.
const SHOW_COMPARE_RECORD_FUEL: u64 = 76_209;
/// Fuel ceiling for concatenating one field of 10 host-supplied objects.
const CONCAT_10_OBJECTS_FUEL: u64 = 130_430;
/// Fuel ceiling for concatenating one field of 100 host-supplied objects.
const CONCAT_100_OBJECTS_FUEL: u64 = 603_319;
/// Fuel ceiling for 1000 iterations of an integer arithmetic loop.
const INT_LOOP_1000_FUEL: u64 = 306_636;
/// Fuel ceiling for 100 iterations of a resumable `for` loop with a comparison.
const ASYNC_INT_LOOP_100_FUEL: u64 = 126_557;
/// Fuel ceiling for 1000 iterations of an empty resumable `while` loop.
const ASYNC_EMPTY_WHILE_1000_FUEL: u64 = 204_697;
/// Fuel ceiling for 1000 iterations of a resumable integer arithmetic `while` loop.
const ASYNC_INT_WHILE_1000_FUEL: u64 = 367_716;
/// Fuel ceiling for 1000 iterations of an empty `for` loop over a Range.
const FOR_RANGE_1000_FUEL: u64 = 349_732;
/// Fuel ceiling for 1000 iterations of an empty `for` loop over a Range held in a variable.
const FOR_RANGE_VARIABLE_1000_FUEL: u64 = 813_832;
/// Fuel ceiling for 100 calls of a small resumable function.
const ASYNC_CALL_LOOP_100_FUEL: u64 = 196_539;
/// Fuel ceiling for 1000 iterations building one formatted line with two Ints.
const FORMAT_LOOP_1000_FUEL: u64 = 2_570_409;
/// Fuel ceiling for 6000 short-lived Lists, enough allocations to trigger collections.
const LIST_CHURN_6000_FUEL: u64 = 8_029_192;
/// Fuel ceiling for 1000 calls of a small typed function.
const SYNC_CALL_LOOP_1000_FUEL: u64 = 370_630;

/// Script whose leading host call selects resumable lowering for an otherwise empty `while` loop.
const ASYNC_EMPTY_WHILE_SOURCE: &str = r#"
    fn main(limit: Int) -> Int {
        Host::elapsed();
        let index = 0;
        while index < limit {
            index = index + 1;
        }
        ret index;
    }
"#;

/// Script whose leading host call selects resumable lowering for the integer arithmetic loop.
const ASYNC_INT_WHILE_SOURCE: &str = r#"
    fn main(limit: Int) -> Int {
        Host::elapsed();
        let total = 0;
        let index = 0;
        while index < limit {
            total = total + index * 3 + 1;
            index = index + 1;
        }
        ret total;
    }
"#;

/// Script iterating an empty-bodied `for` loop over a Range.
const FOR_RANGE_SOURCE: &str = r#"
    fn main(limit: Int) -> Int {
        let count = 0;
        for index in 0..limit {
        }
        ret count;
    }
"#;

/// Script iterating an empty-bodied `for` loop over a Range bound to a variable.
const FOR_RANGE_VARIABLE_SOURCE: &str = r#"
    fn main(limit: Int) -> Int {
        let range = 0..limit;
        for index in range {
        }
        ret range.current;
    }
"#;

/// Script calling a small function, resumable because of its `match`, in a loop.
const ASYNC_CALL_LOOP_SOURCE: &str = r#"
    fn bump(value: Int) -> Int {
        ret match value {
            _ => value + 1,
        };
    }

    fn main(limit: Int) -> Int {
        let total = 0;
        let index = 0;
        while index < limit {
            total = bump(total);
            index = index + 1;
        }
        ret total;
    }
"#;

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

/// Script summing integer arithmetic in a counted `while` loop.
const INT_LOOP_SOURCE: &str = r#"
    fn main(limit: Int) -> Int {
        let total = 0;
        let index = 0;
        while index < limit {
            total = total + index * 3 + 1;
            index = index + 1;
        }
        ret total;
    }
"#;

/// Script whose `for` loop selects resumable lowering, exercising async comparisons.
const ASYNC_INT_LOOP_SOURCE: &str = r#"
    fn main(limit: Int) -> Int {
        let total = 0;
        for index in 0..limit {
            if index < limit {
                total = total + index * 3 + 1;
            }
        }
        ret total;
    }
"#;

/// Script building one formatted line with two interpolated Ints per loop iteration.
const FORMAT_LOOP_SOURCE: &str = r#"
    fn main(limit: Int) -> Int {
        let length = 0;
        let index = 0;
        while index < limit {
            let line = f"row {index}: {index * 2};";
            length = length + line.length();
            index = index + 1;
        }
        ret length;
    }
"#;

/// Script allocating one short-lived List per iteration, enough to trigger collections.
const LIST_CHURN_SOURCE: &str = r#"
    fn main(limit: Int) -> Int {
        let total = 0;
        let index = 0;
        while index < limit {
            let pair = [index, index];
            total = total + pair[0];
            index = index + 1;
        }
        ret total;
    }
"#;

/// Script calling a small typed function once per iteration.
const SYNC_CALL_LOOP_SOURCE: &str = r#"
    fn bump(value: Int) -> Int {
        ret value + 1;
    }

    fn main(limit: Int) -> Int {
        let total = 0;
        let index = 0;
        while index < limit {
            total = bump(total);
            index = index + 1;
        }
        ret total;
    }
"#;

/// Compiles and executes `main` with a Winch runner and returns its execution report.
fn execute_report(source: &str, inputs: &[ExsValue]) -> ExecutionReport {
    let compiled = compile_source(source);
    let runner = ServerRunner::default().with_compiler_strategy(CompilerStrategy::Winch);
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

/// Measures 1000 iterations of a loop with a comparison, additions and a multiplication.
#[test]
fn fuel_int_loop_1000() {
    let report = execute_report(INT_LOOP_SOURCE, &[ExsValue::Int(1000)]);
    assert_fuel(
        "int_loop_1000",
        &report,
        &ExsValue::Int(1_499_500),
        INT_LOOP_1000_FUEL,
    );
}

/// Measures 100 iterations of a resumable `for` loop with a comparison and arithmetic.
#[test]
fn fuel_async_int_loop_100() {
    let report = execute_report(ASYNC_INT_LOOP_SOURCE, &[ExsValue::Int(100)]);
    assert_fuel(
        "async_int_loop_100",
        &report,
        &ExsValue::Int(14_950),
        ASYNC_INT_LOOP_100_FUEL,
    );
}

/// Measures 1000 iterations of an empty `while` loop in a resumable function.
#[test]
fn fuel_async_empty_while_1000() {
    let report = execute_report(ASYNC_EMPTY_WHILE_SOURCE, &[ExsValue::Int(1000)]);
    assert_fuel(
        "async_empty_while_1000",
        &report,
        &ExsValue::Int(1000),
        ASYNC_EMPTY_WHILE_1000_FUEL,
    );
}

/// Measures 1000 iterations of an integer arithmetic `while` loop in a resumable function.
#[test]
fn fuel_async_int_while_1000() {
    let report = execute_report(ASYNC_INT_WHILE_SOURCE, &[ExsValue::Int(1000)]);
    assert_fuel(
        "async_int_while_1000",
        &report,
        &ExsValue::Int(1_499_500),
        ASYNC_INT_WHILE_1000_FUEL,
    );
}

/// Measures 1000 iterations of an empty `for` loop over a Range.
#[test]
fn fuel_for_range_1000() {
    let report = execute_report(FOR_RANGE_SOURCE, &[ExsValue::Int(1000)]);
    assert_fuel(
        "for_range_1000",
        &report,
        &ExsValue::Int(0),
        FOR_RANGE_1000_FUEL,
    );
}

/// Measures 1000 iterations of an empty `for` loop over a Range that stays visible to the script.
#[test]
fn fuel_for_range_variable_1000() {
    let report = execute_report(FOR_RANGE_VARIABLE_SOURCE, &[ExsValue::Int(1000)]);
    assert_fuel(
        "for_range_variable_1000",
        &report,
        &ExsValue::Int(1000),
        FOR_RANGE_VARIABLE_1000_FUEL,
    );
}

/// Measures 100 calls of a small resumable function from a loop.
#[test]
fn fuel_async_call_loop_100() {
    let report = execute_report(ASYNC_CALL_LOOP_SOURCE, &[ExsValue::Int(100)]);
    assert_fuel(
        "async_call_loop_100",
        &report,
        &ExsValue::Int(100),
        ASYNC_CALL_LOOP_100_FUEL,
    );
}

/// Measures 1000 iterations building one formatted line with two interpolated Ints.
#[test]
fn fuel_format_loop_1000() {
    let report = execute_report(FORMAT_LOOP_SOURCE, &[ExsValue::Int(1000)]);
    let expected = (0..1000_i64)
        .map(|index| format!("row {index}: {};", index * 2).len() as i64)
        .sum::<i64>();
    assert_fuel(
        "format_loop_1000",
        &report,
        &ExsValue::Int(expected),
        FORMAT_LOOP_1000_FUEL,
    );
}

/// Measures 6000 iterations allocating short-lived Lists, which triggers garbage collection.
#[test]
fn fuel_list_churn_6000() {
    let report = execute_report(LIST_CHURN_SOURCE, &[ExsValue::Int(6000)]);
    assert_fuel(
        "list_churn_6000",
        &report,
        &ExsValue::Int(17_997_000),
        LIST_CHURN_6000_FUEL,
    );
}

/// Measures 1000 calls of a small typed function in a synchronous loop.
#[test]
fn fuel_sync_call_loop_1000() {
    let report = execute_report(SYNC_CALL_LOOP_SOURCE, &[ExsValue::Int(1000)]);
    assert_fuel(
        "sync_call_loop_1000",
        &report,
        &ExsValue::Int(1000),
        SYNC_CALL_LOOP_1000_FUEL,
    );
}
