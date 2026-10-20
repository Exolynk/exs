//! Runs programs the runtime v2 prototype supports with both backends and compares them.

use std::time::Duration;

use exs_compiler::{Backend, CompileOptions, SourceInput, compile_with_backend};
use exs_runner::{ExecutionCancellation, ExsValue, ProtectionLevel, ServerRunner};

/// Polls one runner future to completion.
fn block_on<Output>(future: impl std::future::Future<Output = Output>) -> Output {
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

/// Compiles and runs `source` with one backend and returns its result and fuel.
fn run(source: &str, backend: Backend, inputs: &[ExsValue]) -> (ExsValue, u64) {
    let compiled = match compile_with_backend(
        SourceInput {
            source_id: "test.exs",
            text: source,
        },
        CompileOptions::default(),
        backend,
    ) {
        Ok(module) => module,
        Err(error) => panic!("{backend:?} compilation failed: {error}"),
    };
    let runner = ServerRunner::new(
        64 << 20,
        1_000_000_000,
        Duration::from_secs(60),
        ProtectionLevel::Standard,
    );
    match block_on(runner.execute_with_report(
        &compiled.wasm,
        "main",
        inputs,
        &ExecutionCancellation::new(),
    )) {
        Ok(report) => (report.value, report.fuel_consumed),
        Err(error) => panic!("{backend:?} execution failed: {error}"),
    }
}

/// Runs `source` with both backends, asserts equal results and returns both fuel values.
fn compare(source: &str, inputs: &[ExsValue]) -> (u64, u64) {
    let (v1, v1_fuel) = run(source, Backend::V1, inputs);
    let (v2, v2_fuel) = run(source, Backend::V2, inputs);
    assert_eq!(v2, v1, "backends disagree on:\n{source}");
    (v1_fuel, v2_fuel)
}

/// Returns scalar results identical to v1 for literals, arithmetic, comparisons and control flow.
#[test]
fn matches_v1_for_scalar_programs() {
    let programs = [
        "fn main() { ret 42; }",
        "fn main() {}",
        "fn main() { ret true; }",
        "fn main() { ret 1.5 + 2; }",
        "fn main() { ret 7 / 2; }",
        "fn main() { ret 3000000000 * 3; }",
        "fn main() { ret -(2147483647 + 1); }",
        "fn main() { ret 2147483647 + 1 == 2147483648; }",
        "fn main() { ret 1 == 1.0 && true != false; }",
        "fn main() { ret !(2 < 1) || false; }",
        "fn main() { let total = 0; let index = 0; while index < 10 { if index == 3 { index = index + 1; continue; } if index == 8 { break; } total = total + index; index = index + 1; } ret total; }",
        "fn main() { let total = 0; for value in 0..10 { total = total + value; } for value in 5..=1 { total = total * 2 + value; } ret total; }",
        "fn square(value: Int) -> Int { ret value * value; } fn main() { ret square(square(3)) + square(-2); }",
        "fn fib(n: Int) -> Int { if n < 2 { ret n; } ret fib(n - 1) + fib(n - 2); } fn main() { ret fib(15); }",
    ];
    for program in programs {
        compare(program, &[]);
    }
}

/// Decodes scalar arguments and encodes scalar results like v1.
#[test]
fn matches_v1_for_scalar_arguments() {
    let source = "fn main(a, b, c, d) { if c { ret a * 2 + b; } ret d; }";
    for inputs in [
        [
            ExsValue::Int(20),
            ExsValue::Int(2),
            ExsValue::Bool(true),
            ExsValue::None,
        ],
        [
            ExsValue::Int(i64::MAX / 4),
            ExsValue::Float(0.25),
            ExsValue::Bool(true),
            ExsValue::None,
        ],
        [
            ExsValue::Int(0),
            ExsValue::Int(0),
            ExsValue::Bool(false),
            ExsValue::Float(-1.5),
        ],
    ] {
        compare(source, &inputs);
    }
    compare(
        "fn main(limit: Int) -> Int { ret limit; }",
        &[ExsValue::Int(-3_000_000_000)],
    );
}

/// Keeps heap Ints and Floats alive across collections triggered by many allocations.
#[test]
fn matches_v1_while_collecting() {
    let source = "fn main(limit: Int) -> Float { let total = 0.0; let index = 0; while index < limit { total = total + 0.5 + (index + 3000000000) * 0; index = index + 1; } ret total; }";
    compare(source, &[ExsValue::Int(200_000)]);
}

/// Prints the fuel of both backends for scalar loops and calls.
#[test]
fn reports_scalar_fuel() {
    let int_loop = "fn main(limit: Int) -> Int { let total = 0; let index = 0; while index < limit { total = total + index; index = index + 1; } ret total; }";
    let for_range = "fn main(limit: Int) -> Int { let total = 0; for index in 0..limit { total = total + index; } ret total; }";
    let calls = "fn bump(value: Int) -> Int { ret value + 1; } fn main(limit: Int) -> Int { let total = 0; let index = 0; while index < limit { total = bump(total); index = index + 1; } ret total; }";
    let floats = "fn main(limit: Int) -> Float { let total = 0.0; let index = 0; while index < limit { total = total + 1.5; index = index + 1; } ret total; }";
    for (name, source) in [
        ("int_loop", int_loop),
        ("for_range", for_range),
        ("sync_calls", calls),
        ("float_loop", floats),
    ] {
        let (v1, v2) = compare(source, &[ExsValue::Int(1000)]);
        println!("v2 fuel {name}: v1 {v1} v2 {v2}");
    }
}

/// Builds, compares and returns Strings exactly like v1.
#[test]
fn matches_v1_for_strings() {
    let programs = [
        r#"fn main() { ret "hello"; }"#,
        r#"fn main() { ret ""; }"#,
        r#"fn main() { ret "äöü " + "€ " + "𝄞"; }"#,
        r#"fn main() { ret "n=" + 7 + ", big=" + 3000000000 + ", f=" + 0.5 + ", whole=" + 2.0 + ", b=" + true; }"#,
        r#"fn main() { let name = "ExS"; let count = 3; ret f"Hallo {name}, {count} mal, {count * 1.5}!"; }"#,
        r#"fn main() { ret 1 + 2 + 0.5; }"#,
        r#"fn main() { ret "a" == "a" && "a" != "b" && "apple" < "banana" && "b" >= "b"; }"#,
        r#"fn main() { let text = "x"; let index = 0; while index < 5 { text = text + index; index = index + 1; } ret text; }"#,
        r#"fn pad(value: Int) -> String { if value < 10 { ret "0" + value; } ret "" + value; } fn main() { ret pad(4) + ":" + pad(12); }"#,
    ];
    for program in programs {
        compare(program, &[]);
    }
    compare(
        r#"fn main(name: String, suffix) { ret name + suffix + name; }"#,
        &[ExsValue::String("Grüße".to_owned()), ExsValue::Int(-5)],
    );
}

/// Keeps Strings alive across collections while building many temporary ones.
#[test]
fn matches_v1_for_strings_while_collecting() {
    let source = r#"fn main(limit: Int) -> String { let kept = "start"; let index = 0; while index < limit { let temporary = "row " + index + ": " + (index * 2) + ";"; if index == 1000 || index == 20000 { kept = kept + temporary; } index = index + 1; } ret kept; }"#;
    compare(source, &[ExsValue::Int(30_000)]);
}

/// Prints the fuel of both backends for String building.
#[test]
fn reports_string_fuel() {
    let format_loop = r#"fn main(limit: Int) -> Int { let length = 0; let index = 0; while index < limit { let line = f"row {index}: {index * 2};"; length = length + 1; index = index + 1; } ret length; }"#;
    let concat_loop = r#"fn main(limit: Int) -> Int { let count = 0; let index = 0; while index < limit { let line = "row " + index + ": " + (index * 2) + ";"; if line != "" { count = count + 1; } index = index + 1; } ret count; }"#;
    for (name, source) in [("format_loop", format_loop), ("concat_loop", concat_loop)] {
        let (v1, v2) = compare(source, &[ExsValue::Int(1000)]);
        println!("v2 fuel {name}: v1 {v1} v2 {v2}");
    }
}
