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
