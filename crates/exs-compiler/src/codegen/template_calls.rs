//! Function references inside the core template's bodies, read once per process.
//!
//! The linker copies the template's bodies unchanged, so every linked module starts with the
//! same bodies. Pruning only needs their function references: which functions each body keeps
//! alive and where the indices sit that renumbering rewrites. Each body is re-encoded once,
//! which also drops the padding of its LEB128 immediates, and later compilations copy those
//! bytes instead of decoding the template's code again.

use std::convert::Infallible;
use std::sync::OnceLock;

use wasm_encoder::reencode::{self, Reencode, RoundtripReencoder};
use wasmparser::{FunctionBody, Operator, Parser, Payload, TypeRef};

/// One function index inside a re-encoded template body.
pub(super) struct CallSite {
    /// Byte offset of the encoded index in `TemplateBody::bytes`.
    pub(super) start: usize,
    /// Byte offset just past the encoded index in `TemplateBody::bytes`.
    pub(super) end: usize,
    /// The referenced function.
    pub(super) target: u32,
}

/// One template body, re-encoded once, and its function references.
pub(super) struct TemplateBody {
    /// Length of the body in the template in bytes, locals included.
    pub(super) len: usize,
    /// The body as the re-encoder writes it, locals included, which is shorter than the
    /// template's padded encoding.
    pub(super) bytes: Vec<u8>,
    /// Every `call`, `return_call` and `ref.func` index, in body order.
    pub(super) calls: Vec<CallSite>,
}

/// Function references of every defined function of the core template.
pub(super) struct TemplateCalls {
    /// Number of imported functions of the template.
    pub(super) imports: u32,
    /// Bodies in definition order.
    pub(super) bodies: Vec<TemplateBody>,
}

/// Returns the function references of the core template, reading them on first use.
///
/// # Errors
///
/// Returns a message when the template cannot be parsed.
pub(super) fn template_calls() -> Result<&'static TemplateCalls, String> {
    static CALLS: OnceLock<Result<TemplateCalls, String>> = OnceLock::new();
    CALLS
        .get_or_init(|| read(exs_core_template::WASM_TEMPLATE))
        .as_ref()
        .map_err(Clone::clone)
}

/// Reads the function references of every body of `template`.
fn read(template: &[u8]) -> Result<TemplateCalls, String> {
    let error = |error: wasmparser::BinaryReaderError| error.to_string();
    let mut calls = TemplateCalls {
        imports: 0,
        bodies: Vec::new(),
    };
    for payload in Parser::new(0).parse_all(template) {
        match payload.map_err(error)? {
            Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    if matches!(
                        import.map_err(error)?.ty,
                        TypeRef::Func(_) | TypeRef::FuncExact(_)
                    ) {
                        calls.imports += 1;
                    }
                }
            }
            Payload::CodeSectionEntry(body) => calls.bodies.push(body_calls(&body)?),
            _ => {}
        }
    }
    Ok(calls)
}

/// Re-encodes one body and records the function references in the re-encoded bytes.
fn body_calls(body: &FunctionBody<'_>) -> Result<TemplateBody, String> {
    let error = |error: reencode::Error<Infallible>| format!("{error:?}");
    let mut reencoder = RoundtripReencoder;
    let mut function = reencoder
        .new_function_with_parsed_locals(body)
        .map_err(error)?;
    let mut calls = Vec::new();
    let mut reader = body
        .get_operators_reader()
        .map_err(|error| error.to_string())?;
    while !reader.eof() {
        let operator = reader.read().map_err(|error| error.to_string())?;
        let target = match operator {
            Operator::Call { function_index }
            | Operator::ReturnCall { function_index }
            | Operator::RefFunc { function_index } => Some(function_index),
            _ => None,
        };
        let start = function.byte_len();
        function.instruction(&reencoder.instruction(operator).map_err(error)?);
        if let Some(target) = target {
            // The three opcodes are one byte each, followed by the index.
            calls.push(CallSite {
                start: start + 1,
                end: function.byte_len(),
                target,
            });
        }
    }
    Ok(TemplateBody {
        len: body.range().len(),
        bytes: function.into_raw_body(),
        calls,
    })
}
