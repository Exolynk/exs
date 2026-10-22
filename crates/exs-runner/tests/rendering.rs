//! Rendering values in ExS literal syntax through `to_string`, `debug` and formatted strings.

mod support;

use exs_abi::ExsValue;
use exs_runner::{ExecutionCancellation, ServerRunner};
use support::{block_on, compile_source, execute_source_with_inputs};

/// Returns the Strings of a List result.
fn strings(value: ExsValue) -> Vec<String> {
    match value {
        ExsValue::List(items) => items
            .into_iter()
            .map(|item| match item {
                ExsValue::String(text) => text,
                other => panic!("expected a String, got {other:?}"),
            })
            .collect(),
        other => panic!("expected a List, got {other:?}"),
    }
}

/// Renders composites, scalars and resources in literal syntax.
#[test]
fn renders_values_in_literal_syntax() {
    let result = execute_source_with_inputs(
        r#"
            type Point { x: Int, name: String }
            type Empty {}
            enum Color { Red, Rgb(r: Int, g: Int) }

            fn main() -> List {
                let cycle = [1];
                cycle.push(cycle);
                ret [
                    [1, "a", [2.5, None, true]].to_string(),
                    f"{ {a: 1, "display-name": "x\ny"} }",
                    f"{Point { x: 1, name: "Ada" }}",
                    Empty {}.to_string(),
                    f"{Color::Red} {Color::Rgb(1, 2)}",
                    f"{Error("Kind", "message", [1])}",
                    f"{0..3} {1..=5} {Range::step(0, 10, 2)}",
                    f"{Duration::seconds(1)}",
                    f"{cycle}",
                    "a\"b".debug(),
                    "plain".to_string(),
                    b"hi\n".debug(),
                    Bytes::from_list([0, 255]).debug(),
                    (() => { ret 1; }).to_string(),
                    {}.to_string(),
                    None.debug(),
                    [].debug(),
                ];
            }
        "#,
        &[],
    );
    assert_eq!(
        strings(result),
        [
            r#"[1, "a", [2.5, None, true]]"#,
            r#"{ a: 1, "display-name": "x\ny" }"#,
            r#"Point { x: 1, name: "Ada" }"#,
            "Empty {}",
            "Color::Red Color::Rgb(1, 2)",
            r#"Error("Kind", "message", [1])"#,
            "0..3 1..=5 Range::step(0, 10, 2)",
            "Duration { seconds: 1, nanoseconds: 0 }",
            "[1, ...]",
            r#""a\"b""#,
            "plain",
            r#"b"hi\n""#,
            r#"Bytes::from_hex("00ff")"#,
            "Fn",
            "{}",
            "None",
            "[]",
        ]
    );
}

/// Uses `Debug`, else `ToString`, implementations for parts and at the top level.
#[test]
fn renders_parts_through_implementations() {
    let result = execute_source_with_inputs(
        r#"
            type Money { cents: Int }
            impl ToString for Money {
                fn to_string(self) -> String { ret f"{self.cents} ct"; }
            }

            type Secret { value: String }
            impl Debug for Secret {
                fn debug(self) -> String { ret "Secret(***)"; }
            }

            fn main() -> List {
                let money = Money { cents: 5 };
                let secret = Secret { value: "x" };
                ret [
                    f"{money}",
                    f"{[money]}",
                    { secret: secret }.to_string(),
                    secret.debug(),
                    f"{secret}",
                ];
            }
        "#,
        &[],
    );
    assert_eq!(
        strings(result),
        [
            "5 ct",
            "[5 ct]",
            "{ secret: Secret(***) }",
            "Secret(***)",
            r#"Secret { value: "x" }"#,
        ]
    );
}

/// Resumes a `ToString` implementation that suspends while a composite renders.
#[test]
fn resumes_suspending_render_implementations() {
    let compiled = compile_source(
        r#"
            type Tag { name: String }

            impl ToString for Tag {
                fn to_string(self) -> String {
                    ret Host::call("echo", self.name);
                }
            }

            fn main() -> String {
                ret f"tags: {[Tag { name: "a" }, Tag { name: "b" }]}";
            }
        "#,
    );
    let mut runner = ServerRunner::default();
    assert!(
        runner
            .registry_mut()
            .fn_async_raw("echo", |arguments: Vec<ExsValue>| async move {
                arguments.into_iter().next().unwrap_or(ExsValue::None)
            })
            .is_ok()
    );
    let result = match block_on(runner.execute(
        &compiled.wasm,
        "main",
        &[],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::String("tags: [a, b]".to_owned()));
}
