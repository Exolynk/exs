use std::time::Duration;

use exs_runner::{ExecutionCancellation, ProtectionLevel, ServerRunner};

use super::*;

/// Executes `main` with a fuel budget large enough to run several collections.
fn execute_collecting(source: &str, input: ExsValue) -> ExsValue {
    let compiled = super::support::compile_source(source);
    let runner = ServerRunner::new(
        64 * 1024 * 1024,
        1_000_000_000,
        Duration::from_secs(60),
        ProtectionLevel::Standard,
    );
    match super::support::block_on(runner.execute(
        &compiled.wasm,
        "main",
        &[input],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    }
}

/// Preserves live aliases through allocation-triggered collection.
#[test]
fn preserves_live_aliases_across_allocation_triggered_collection() {
    assert_eq!(
        execute_collecting(
            r#"
            fn churn(value) {
                let discarded = [value, { value: value }, [value, value]];
                ret 0;
            }
            fn main(input) {
                let object = { value: input };
                let alias = object;
                let count = 0;
                while count < 4096 {
                    churn(count);
                    count = count + 1;
                }
                if alias == object && alias.value == input {
                    ret object;
                }
                ret {};
            }
        "#,
            ExsValue::Int(42),
        ),
        ExsValue::Object(vec![("value".to_owned(), ExsValue::Int(42))]),
    );
}

/// Traces a self-referential List without losing its identity during threshold-triggered collection.
#[test]
fn traces_cycles_during_allocation_triggered_collection() {
    assert_eq!(
        execute_collecting(
            r#"
            fn churn(value) {
                let discarded = [value, value, { value: value }];
                ret 0;
            }
            fn main(input) {
                let cycle = [];
                cycle.push(cycle);
                let count = 0;
                while count < 4096 {
                    churn(count);
                    count = count + 1;
                }
                ret cycle[0] == cycle;
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::Bool(true),
    );
}

/// Keeps every frame's locals rooted while deep recursion spans several shadow root stack chunks.
#[test]
fn preserves_roots_across_root_stack_chunks_during_collection() {
    assert_eq!(
        execute_collecting(
            r#"
            fn build(depth) {
                let kept = [depth, { depth: depth }];
                if depth == 0 {
                    // Churn past the collection threshold while every frame's locals are live.
                    let count = 0;
                    while count < 6000 {
                        let discarded = [count, { count: count }];
                        count = count + 1;
                    }
                    ret kept[0];
                }
                let below = build(depth - 1);
                ret below + kept[1].depth;
            }
            fn main(input) {
                ret [build(input), build(input)];
            }
        "#,
            ExsValue::Int(100),
        ),
        ExsValue::List(vec![ExsValue::Int(5_050), ExsValue::Int(5_050)]),
    );
}

/// Keeps long-lived values intact across many collections that free short-lived garbage.
#[test]
fn keeps_survivors_across_repeated_collections() {
    assert_eq!(
        execute_collecting(
            r#"
            fn main(input) {
                let kept = [];
                let index = 0;
                while index < 3000 {
                    kept.push({ value: index, label: f"item {index}" });
                    index = index + 1;
                }
                let round = 0;
                while round < 20000 {
                    let discarded = [round, { round: round }, f"temp {round}"];
                    round = round + 1;
                }
                let total = 0;
                for entry in kept {
                    total = total + entry.value;
                }
                ret [total, kept[0].label, kept[2999].label, kept.length()];
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![
            ExsValue::Int(4_498_500),
            ExsValue::String("item 0".to_owned()),
            ExsValue::String("item 2999".to_owned()),
            ExsValue::Int(3000),
        ]),
    );
}
