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
        ExsValue::List(vec![ExsValue::Int(1), ExsValue::Unit]),
        ExsValue::Int(10),
        ExsValue::List(vec![ExsValue::Int(7), ExsValue::Unit]),
        ExsValue::List(vec![ExsValue::Int(8)]),
        ExsValue::Bool(false),
        ExsValue::None,
        ExsValue::List(vec![ExsValue::Int(1), ExsValue::Unit]),
        ExsValue::Int(2),
    ]);
    // Any closure makes every function resumable, which selects the continuation lowering.
    let resumable = format!("{source} fn force_resumable() {{ ret () => {{ ret 1; }}; }}");
    for source in [source, resumable.as_str()] {
        assert_eq!(execute_source(source, ExsValue::None), expected);
    }
}

/// Matches literal patterns of every literal kind, in direct and resumable functions.
#[test]
fn matches_literal_patterns() {
    let source = r#"
        fn describe(value) -> String {
            ret match value {
                0 => "zero",
                -1 => "minus one",
                1.5 => "one and a half",
                "admin" => "administrator",
                true => "yes",
                None => "nothing",
                b"x" => "bytes",
                _ => "other",
            };
        }

        fn strict(value) {
            ret match value { 1 => "one", 2 => "two" };
        }

        fn main(input) -> List {
            ret [
                describe(0), describe(-1), describe(1.5), describe("admin"), describe(true),
                describe(None), describe(b"x"), describe(7), describe(0.0), strict(2),
                strict(3) is Error,
            ];
        }
    "#;
    let expected = ExsValue::List(vec![
        ExsValue::String("zero".to_owned()),
        ExsValue::String("minus one".to_owned()),
        ExsValue::String("one and a half".to_owned()),
        ExsValue::String("administrator".to_owned()),
        ExsValue::String("yes".to_owned()),
        ExsValue::String("nothing".to_owned()),
        ExsValue::String("bytes".to_owned()),
        ExsValue::String("other".to_owned()),
        ExsValue::String("zero".to_owned()),
        ExsValue::String("two".to_owned()),
        ExsValue::Bool(true),
    ]);
    let resumable = source.replace(
        "ret match value {\n                0",
        "Host::sleep(Duration::milliseconds(0));\n            ret match value {\n                0",
    );
    assert_ne!(resumable, source);
    for source in [source, resumable.as_str()] {
        assert_eq!(execute_source(source, ExsValue::None), expected);
    }
}

/// Reads `Name {` as a typed Object only when fields or `}` follow, so a match on a variant
/// literal parses.
#[test]
fn matches_variant_literals_and_typed_objects() {
    assert_eq!(
        execute_source(
            r#"
            enum Choice { First, Second(value: Int) }
            type Point { x: Int }
            type Empty {}

            fn main(input) -> List {
                let first = match Choice::First { Choice::First => 1, Choice::Second(value) => value };
                let second = match Choice::Second(5) { Choice::Second(value) => value, _ => 0 };
                let flag = 0;
                if Point { x: 1 } == Point { x: 1 } { flag = 1; }
                ret [first, second, flag, Empty {} == Empty {}];
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![
            ExsValue::Int(1),
            ExsValue::Int(5),
            ExsValue::Int(1),
            ExsValue::Bool(true),
        ]),
    );
}
