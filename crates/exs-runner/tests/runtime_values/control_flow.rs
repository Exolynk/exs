use super::*;

/// Selects the first matching branch of an else-if conditional chain.
#[test]
fn executes_else_if_conditional_chains() {
    let source = r#"
        fn main(value: Int) -> Int {
            if value > 0 {
                ret 1;
            } else if value < 0 {
                ret -1;
            } else {
                ret 0;
            }
        }
    "#;
    assert_eq!(execute_source(source, ExsValue::Int(7)), ExsValue::Int(1));
    assert_eq!(execute_source(source, ExsValue::Int(-3)), ExsValue::Int(-1));
    assert_eq!(execute_source(source, ExsValue::Int(0)), ExsValue::Int(0));
}

/// Keeps bindings declared in a standalone block local to that block.
#[test]
fn executes_standalone_lexical_blocks() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let value = 1;
                {
                    let value = 2;
                    value = value + 1;
                }
                ret value;
            }
            "#,
            ExsValue::None,
        ),
        ExsValue::Int(1)
    );
}

/// Initializes an omitted local binding value to None.
#[test]
fn initializes_omitted_local_bindings_to_none() {
    assert_eq!(
        execute_source(
            "fn main(input) -> None { let later; ret later; }",
            ExsValue::None,
        ),
        ExsValue::None
    );
}

/// Runs resumable recursion deeper than the direct child-call limit through the dispatcher.
#[test]
fn recurses_beyond_the_direct_child_call_depth() {
    let source = r#"
        fn depth(remaining: Int) -> Int {
            if remaining == 0 { ret 0; }
            ret depth(remaining - 1) + 1;
        }
        fn main(input: Int) -> List {
            ret [depth(input), depth(3), depth(input)];
        }
    "#;
    let expected = ExsValue::List(vec![
        ExsValue::Int(500),
        ExsValue::Int(3),
        ExsValue::Int(500),
    ]);
    // Any closure makes every function resumable, so every call creates a child frame.
    let resumable = format!("{source} fn force_resumable() {{ ret () => {{ ret 1; }}; }}");
    for source in [source, resumable.as_str()] {
        assert_eq!(execute_source(source, ExsValue::Int(500)), expected);
    }
}

/// Evaluates variable operands before later operands rebind the variable, in direct functions
/// and in resumable continuations.
#[test]
fn reads_variable_operands_before_later_rebinding() {
    let source = r#"
        fn pair(left, right) { ret [left, right]; }
        fn main(input) -> List {
            let a = 1;
            let first = pair(a, match input { _ => { a = 10; } });
            let items = [7];
            let original = items;
            items.push(match input { _ => { items = [8]; } });
            let c = "x";
            let third = c == match input { _ => { c = None; } };
            let e = 1;
            let listed = [e, match input { _ => { e = 2; } }];
            ret [first, a, original, items, third, c, listed, e];
        }
    "#;
    let expected = ExsValue::List(vec![
        ExsValue::List(vec![ExsValue::Int(1), ExsValue::None]),
        ExsValue::Int(10),
        ExsValue::List(vec![ExsValue::Int(7), ExsValue::None]),
        ExsValue::List(vec![ExsValue::Int(8)]),
        ExsValue::Bool(false),
        ExsValue::None,
        ExsValue::List(vec![ExsValue::Int(1), ExsValue::None]),
        ExsValue::Int(2),
    ]);
    // Any closure makes every function resumable, which selects the continuation lowering.
    let resumable = format!("{source} fn force_resumable() {{ ret () => {{ ret 1; }}; }}");
    for source in [source, resumable.as_str()] {
        assert_eq!(execute_source(source, ExsValue::None), expected);
    }
}
