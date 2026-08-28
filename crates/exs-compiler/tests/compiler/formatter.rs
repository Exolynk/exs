use super::*;

/// Formats valid source into a stable, reparsable canonical layout.
#[test]
fn formats_source_into_canonical_layout() {
    let source = "import \"./math.exs\" as math;use math::{add as plus,Point};fn main(value:Int)->Int{let point=Point{value:value};if value>0{ret plus(point.value,1);}else{ret 0;}}";
    let formatted = match format(SourceInput {
        source_id: "format.exs",
        text: source,
    }) {
        Ok(formatted) => formatted,
        Err(error) => panic!("formatting failed: {error}"),
    };
    assert_eq!(
        formatted,
        "import \"./math.exs\" as math;\nuse math::{add as plus, Point};\n\nfn main(value: Int) -> Int {\n    let point = Point {value: value};\n    if value > 0 {\n        ret plus(point.value, 1);\n    }\n    else {\n        ret 0;\n    }\n}\n"
    );
    let reformatted = match format(SourceInput {
        source_id: "format.exs",
        text: &formatted,
    }) {
        Ok(formatted) => formatted,
        Err(error) => panic!("reformatting failed: {error}"),
    };
    assert_eq!(reformatted, formatted);
}

/// Retains comments and empty source lines while canonicalizing ExS syntax.
#[test]
fn formatter_preserves_comments_and_blank_lines() {
    let source = "//file\n\nfn main(){\n//before\nlet value=1; //trailing\n\n//between\nret value;\n//last\n}";
    let formatted = match format(SourceInput {
        source_id: "format-trivia.exs",
        text: source,
    }) {
        Ok(formatted) => formatted,
        Err(error) => panic!("formatting failed: {error}"),
    };
    assert_eq!(
        formatted,
        "// file\n\nfn main() {\n    // before\n    let value = 1;\n    // trailing\n\n    // between\n    ret value;\n    // last\n}\n"
    );
    let reformatted = match format(SourceInput {
        source_id: "format-trivia.exs",
        text: &formatted,
    }) {
        Ok(formatted) => formatted,
        Err(error) => panic!("formatting failed: {error}"),
    };
    assert_eq!(reformatted, formatted);
}

/// Canonicalizes empty functions and documentation-declaration boundaries on reformatting.
#[test]
fn formatter_normalizes_empty_blocks_and_preserves_declaration_spacing() {
    let source = "\n\n///asdasdasd\nfn main() {\n    let value = 21 * 2;\n    Host::call(\"println\", \"The result is\", value);\n    //lala\n    ret value;\n}\n\n\n///Hallo\n\nfn test() {\n\n\n}\n";
    let formatted = match format(SourceInput {
        source_id: "format-empty-block.exs",
        text: source,
    }) {
        Ok(formatted) => formatted,
        Err(error) => panic!("formatting failed: {error}"),
    };
    assert_eq!(
        formatted,
        "/// asdasdasd\nfn main() {\n    let value = 21 * 2;\n    Host::call(\"println\", \"The result is\", value);\n    // lala\n    ret value;\n}\n\n/// Hallo\nfn test() {}\n"
    );
    let reformatted = match format(SourceInput {
        source_id: "format-empty-block.exs",
        text: &formatted,
    }) {
        Ok(formatted) => formatted,
        Err(error) => panic!("reformatting failed: {error}"),
    };
    assert_eq!(reformatted, formatted);
}

/// Preserves conditional chains instead of converting them into nested else blocks.
#[test]
fn formats_else_if_chains() {
    let source = "fn main(value:Int)->Int{if value>0{ret 1;}else if value<0{ret -1;}else{ret 0;}}";
    let formatted = match format(SourceInput {
        source_id: "format-else-if.exs",
        text: source,
    }) {
        Ok(formatted) => formatted,
        Err(error) => panic!("formatting failed: {error}"),
    };
    assert_eq!(
        formatted,
        "fn main(value: Int) -> Int {\n    if value > 0 {\n        ret 1;\n    }\n    else if value < 0 {\n        ret -1;\n    }\n    else {\n        ret 0;\n    }\n}\n"
    );
    assert!(
        compile(
            SourceInput {
                source_id: "format-else-if.exs",
                text: &formatted,
            },
            CompileOptions::default(),
        )
        .is_ok()
    );
}

/// Formats standalone lexical blocks without treating them as object expressions.
#[test]
fn formats_standalone_lexical_blocks() {
    let source = "fn main(input)->Int{let value=1;{let value=2;}ret value;}";
    let formatted = match format(SourceInput {
        source_id: "format-block.exs",
        text: source,
    }) {
        Ok(formatted) => formatted,
        Err(error) => panic!("formatting failed: {error}"),
    };
    assert_eq!(
        formatted,
        "fn main(input) -> Int {\n    let value = 1;\n    {\n        let value = 2;\n    }\n    ret value;\n}\n"
    );
    assert!(
        compile(
            SourceInput {
                source_id: "format-block.exs",
                text: &formatted,
            },
            CompileOptions::default(),
        )
        .is_ok()
    );
}

/// Rejects malformed source instead of attempting a best-effort rewrite.
#[test]
fn formatter_returns_syntax_diagnostics() {
    let error = match format(SourceInput {
        source_id: "format-error.exs",
        text: "fn main( {",
    }) {
        Ok(_) => panic!("malformed source was formatted"),
        Err(error) => error,
    };
    assert!(!error.diagnostics.is_empty());
}

/// Formats statement-block match arms in their compact expression position.
#[test]
fn formats_match_block_arms() {
    let formatted = match format(SourceInput {
        source_id: "format-match.exs",
        text: "enum Color{Transparent,}fn main(value:Color)->Int{ret match value{Color::Transparent=>{ret -1;}};}",
    }) {
        Ok(formatted) => formatted,
        Err(error) => panic!("formatting failed: {error}"),
    };
    assert_eq!(
        formatted,
        "enum Color {\n    Transparent,\n}\n\nfn main(value: Color) -> Int {\n    ret match value { Color::Transparent => { ret -1; } };\n}\n"
    );
}
