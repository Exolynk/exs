//! Module constants substituted into their references.

mod support;

use exs_abi::ExsValue;
use support::execute_source_with_inputs;

/// Substitutes constants everywhere no local binding shadows them.
#[test]
fn substitutes_constants() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                /// The greeted name.
                const NAME = "Ada";
                const NEGATIVE = -5;
                const RATIO = 0.5;
                const ENABLED = true;
                const NOTHING = None;
                const ALIAS = NAME;
                private const BYTES = b"hi";

                enum Wrapper { Value(NAME: Int) }

                fn shadowed(NAME: Int) -> Int { ret NAME; }

                fn main() -> List {
                    let closure = () => { ret NAME; };
                    let local = match Wrapper::Value(3) { Wrapper::Value(NAME) => NAME };
                    ret [
                        NAME, NEGATIVE, RATIO, ENABLED, NOTHING, ALIAS, BYTES.length(),
                        shadowed(3), closure(), local, f"{NAME}!",
                    ];
                }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::String("Ada".to_owned()),
            ExsValue::Int(-5),
            ExsValue::Float(0.5),
            ExsValue::Bool(true),
            ExsValue::None,
            ExsValue::String("Ada".to_owned()),
            ExsValue::Int(2),
            ExsValue::Int(3),
            ExsValue::String("Ada".to_owned()),
            ExsValue::Int(3),
            ExsValue::String("Ada!".to_owned()),
        ])
    );
}
