use super::*;

/// Validates the generated Wasm shape for one Cell-backed closure expression.
#[test]
fn validates_wasm_for_a_closure_expression() {
    let compiled = match compile(
        Source::new(
            "closure.exs",
            "fn main() -> Int { let count = 0; let increment = () => { count = count + 1; ret count; }; increment(); ret increment(); }",
        ),
        CompileOptions::default(),
    ) {
        Ok(compiled) => compiled,
        Err(error) => panic!("compilation failed: {error}"),
    };
    if let Err(error) = Validator::new().validate_all(&compiled.wasm) {
        panic!("generated Wasm is invalid: {error}");
    }
}

/// Compiles a frame-backed Host ABI call and its runner-facing control exports.
#[test]
fn compiles_a_resumable_host_call() {
    let compiled = match compile(
        Source::new(
            "host-call.exs",
            "fn main(input) { ret Host::call(\"echo\", input); }",
        ),
        CompileOptions::default(),
    ) {
        Ok(compiled) => compiled,
        Err(error) => panic!("compilation failed: {error}"),
    };
    if let Err(error) = Validator::new().validate_all(&compiled.wasm) {
        panic!("generated Wasm is invalid: {error}");
    }
    let exports = Parser::new(0)
        .parse_all(&compiled.wasm)
        .filter_map(Result::ok)
        .filter_map(|payload| match payload {
            Payload::ExportSection(section) => Some(section),
            _ => None,
        })
        .flat_map(|section| section.into_iter().filter_map(Result::ok))
        .map(|export| export.name.to_owned())
        .collect::<Vec<_>>();
    assert!(exports.iter().any(|name| name == "__exs_resume_host"));
    assert!(exports.iter().any(|name| name == "__exs_cancel"));
    assert!(exports.iter().any(|name| name == "__exs_start_main"));
}

/// Emits compact source positions by default and embeds source text only when requested.
#[test]
fn emits_source_map_and_optional_source_sections() {
    let source = "fn main(input) { ret input + 1; }";
    let compiled = match compile(Source::new("maps.exs", source), {
        let mut options = CompileOptions::default();
        options.embed_sources = true;
        options
    }) {
        Ok(compiled) => compiled,
        Err(error) => panic!("compilation failed: {error}"),
    };
    let sections = Parser::new(0)
        .parse_all(&compiled.wasm)
        .filter_map(Result::ok)
        .filter_map(|payload| match payload {
            Payload::CustomSection(section) => Some((section.name(), section.data().to_vec())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        sections
            .iter()
            .any(|(name, data)| { *name == "exs.source.map" && data.starts_with(b"EXSMAP3\0") })
    );
    assert!(sections.iter().any(|(name, data)| {
        *name == "exs.sources"
            && data.starts_with(b"EXSSRC2\0")
            && data.ends_with(source.as_bytes())
    }));

    let without_sources = match compile(Source::new("maps.exs", source), CompileOptions::default())
    {
        Ok(compiled) => compiled,
        Err(error) => panic!("compilation failed: {error}"),
    };
    let has_embedded_source = Parser::new(0)
        .parse_all(&without_sources.wasm)
        .filter_map(Result::ok)
        .any(|payload| {
            matches!(payload, Payload::CustomSection(section) if section.name() == "exs.sources")
        });
    assert!(!has_embedded_source);

    let debug_info = match read_debug_info(&compiled.wasm) {
        Ok(debug_info) => debug_info,
        Err(error) => panic!("could not read debug metadata: {error}"),
    };
    assert_eq!(debug_info.function_name(0), Some("main"));
    assert_eq!(debug_info.source_for("maps.exs"), Some(source));
}
