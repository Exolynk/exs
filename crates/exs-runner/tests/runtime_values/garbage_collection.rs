use super::*;

/// Preserves live aliases through allocation-triggered collection.
#[test]
fn preserves_live_aliases_across_allocation_triggered_collection() {
    assert_eq!(
        execute_source(
            r#"
            fn churn(value) {
                let discarded = [value, { value: value }, [value, value]];
                ret 0;
            }
            fn main(input) {
                let object = { value: input };
                let alias = object;
                let count = 0;
                while count < 512 {
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
        execute_source(
            r#"
            fn churn(value) {
                let discarded = [value, value, { value: value }];
                ret 0;
            }
            fn main(input) {
                let cycle = [];
                cycle.push(cycle);
                let count = 0;
                while count < 512 {
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
        execute_source(
            r#"
            fn build(depth) {
                let kept = [depth, { depth: depth }];
                if depth == 0 {
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
