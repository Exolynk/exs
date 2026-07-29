//! Command-line interface for the Phase-1 `ExS` toolchain.

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use exs_compiler::{CompileOptions, ModuleDebugInfo, SourceInput, compile, read_debug_info};

/// Runs the `ExS` command-line interface.
fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("exs: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Dispatches a CLI command.
fn run(arguments: Vec<String>) -> Result<(), String> {
    let Some(command) = arguments.first().map(String::as_str) else {
        return Err(usage());
    };
    match command {
        "check" if arguments.len() == 2 => {
            let _module = compile_source(&arguments[1], CompileOptions::default())?;
            Ok(())
        }
        "compile" if arguments.len() == 4 && arguments[2] == "-o" => {
            let module = compile_source(&arguments[1], CompileOptions::default())?;
            fs::write(&arguments[3], module.wasm)
                .map_err(|error| format!("could not write {}: {error}", arguments[3]))?;
            Ok(())
        }
        "run" if arguments.len() == 2 => run_program(&arguments[1]),
        _ => Err(usage()),
    }
}

/// Compiles one source file using the runtime template embedded in the compiler dependency.
fn compile_source(
    path: &str,
    options: CompileOptions,
) -> Result<exs_compiler::CompiledModule, String> {
    let source =
        fs::read_to_string(path).map_err(|error| format!("could not read {path}: {error}"))?;
    compile(
        SourceInput {
            source_id: path,
            text: &source,
        },
        options,
    )
    .map_err(|error| error.to_string())
}

/// Executes a source file or linked WebAssembly module.
fn run_program(path: &str) -> Result<(), String> {
    let wasm = if Path::new(path)
        .extension()
        .is_some_and(|extension| extension == "wasm")
    {
        fs::read(path).map_err(|error| format!("could not read {path}: {error}"))?
    } else {
        compile_source(
            path,
            CompileOptions {
                embed_sources: true,
            },
        )?
        .wasm
    };
    let debug_info = read_debug_info(&wasm).ok();
    let result = exs_runner::execute(&wasm, exs_runner::ExsValue::None)
        .map_err(|error| error.to_string())?;
    match result {
        exs_runner::ExsValue::Error(error) => Err(format_error(&error, debug_info.as_ref())),
        result => {
            print_result(result);
            Ok(())
        }
    }
}

/// Prints a completed Phase-1 program result in ExS source notation.
fn print_result(result: exs_runner::ExsValue) {
    println!("{}", format_result(&result));
}

/// Formats one host-safe ExS value using source-like syntax.
fn format_result(result: &exs_runner::ExsValue) -> String {
    match result {
        exs_runner::ExsValue::None => "None".to_owned(),
        exs_runner::ExsValue::Ok(value) => format!("Ok({})", format_result(value)),
        exs_runner::ExsValue::Error(error) => {
            format!("Error({:?}, {:?})", error.kind, error.message)
        }
        exs_runner::ExsValue::Bool(value) => value.to_string(),
        exs_runner::ExsValue::Int(value) => value.to_string(),
        exs_runner::ExsValue::Float(value) if value.is_finite() && value.fract() == 0.0 => {
            format!("{value:.1}")
        }
        exs_runner::ExsValue::Float(value) => value.to_string(),
        exs_runner::ExsValue::String(value) => format!("{value:?}"),
        exs_runner::ExsValue::List(values) => {
            let values = values
                .iter()
                .map(format_result)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{values}]")
        }
        exs_runner::ExsValue::Object(entries) => {
            let entries = entries
                .iter()
                .map(|(key, value)| format!("{key:?}: {}", format_result(value)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{{entries}}}")
        }
    }
}

/// Formats a complete top-level language Error for the command-line interface.
fn format_error(error: &exs_runner::ExsError, debug_info: Option<&ModuleDebugInfo>) -> String {
    let mut output = String::new();
    format_error_into(&mut output, error, debug_info, "");
    output
}

/// Appends one structured language Error, including nested causes, to a report buffer.
fn format_error_into(
    output: &mut String,
    error: &exs_runner::ExsError,
    debug_info: Option<&ModuleDebugInfo>,
    indent: &str,
) {
    output.push_str(indent);
    output.push_str("error: ");
    output.push_str(&error.kind);
    output.push_str(" (");
    output.push_str(match error.severity {
        exs_runner::ErrorSeverity::Recoverable => "recoverable",
        exs_runner::ErrorSeverity::Fatal => "fatal",
    });
    output.push_str(")\n");
    output.push_str(indent);
    output.push_str("  message: ");
    output.push_str(&error.message);
    output.push('\n');
    output.push_str(indent);
    output.push_str("  data: ");
    output.push_str(&format_result(&error.data));
    output.push('\n');
    if let Some(origin) = error.origin {
        output.push_str(indent);
        output.push_str("  origin: ");
        output.push_str(&format_position(debug_info, origin));
        output.push('\n');
        append_source_excerpt(output, debug_info, origin, indent);
    }
    if !error.trace.is_empty() {
        output.push_str(indent);
        output.push_str("  trace:\n");
        for frame in &error.trace {
            output.push_str(indent);
            output.push_str("    ");
            output.push_str(
                debug_info
                    .and_then(|info| info.function_name(frame.function_id))
                    .map_or_else(
                        || format!("function #{}", frame.function_id),
                        ToOwned::to_owned,
                    )
                    .as_str(),
            );
            if frame.call_site.0 != 0 {
                output.push_str(" called at ");
                output.push_str(&format_position(debug_info, frame.call_site));
            }
            output.push('\n');
        }
    }
    if let Some(cause) = &error.cause {
        output.push_str(indent);
        output.push_str("  cause:\n");
        match cause.as_ref() {
            exs_runner::ExsValue::Error(cause) => {
                let nested_indent = format!("{indent}    ");
                format_error_into(output, cause, debug_info, &nested_indent);
            }
            cause => {
                output.push_str(indent);
                output.push_str("    ");
                output.push_str(&format_result(cause));
                output.push('\n');
            }
        }
    }
}

/// Formats one source position with line and column when source text is available.
fn format_position(
    debug_info: Option<&ModuleDebugInfo>,
    identifier: exs_runner::SourcePositionId,
) -> String {
    let Some(position) = debug_info.and_then(|info| info.position(identifier)) else {
        return format!("position #{}", identifier.0);
    };
    let Some(source) = debug_info.and_then(|info| info.source.as_deref()) else {
        return format!(
            "{}:{}-{}",
            position.source_id, position.start_byte, position.end_byte
        );
    };
    let (line, column) = line_and_column(source, position.start_byte);
    format!("{}:{line}:{column}", position.source_id)
}

/// Appends the source line and a caret for one resolved position when embedded source is present.
fn append_source_excerpt(
    output: &mut String,
    debug_info: Option<&ModuleDebugInfo>,
    identifier: exs_runner::SourcePositionId,
    indent: &str,
) {
    let Some(position) = debug_info.and_then(|info| info.position(identifier)) else {
        return;
    };
    let Some(source) = debug_info.and_then(|info| info.source.as_deref()) else {
        return;
    };
    let (line, column) = line_and_column(source, position.start_byte);
    let Some(source_line) = source.lines().nth(line.saturating_sub(1)) else {
        return;
    };
    output.push_str(indent);
    output.push_str("    ");
    output.push_str(&line.to_string());
    output.push_str(" | ");
    output.push_str(source_line);
    output.push('\n');
    output.push_str(indent);
    output.push_str("      | ");
    output.push_str(&" ".repeat(column.saturating_sub(1)));
    output.push('^');
    output.push('\n');
}

/// Converts a UTF-8 byte offset into one-based source line and Unicode-scalar column numbers.
fn line_and_column(source: &str, offset: u32) -> (usize, usize) {
    let offset = usize::try_from(offset)
        .unwrap_or(source.len())
        .min(source.len());
    let prefix = source.get(..offset).unwrap_or(source);
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .map_or(1, |line| line.chars().count() + 1);
    (line, column)
}

/// Returns CLI usage text.
fn usage() -> String {
    "usage: exs check <file.exs> | exs compile <file.exs> -o <file.wasm> | exs run <file.exs|file.wasm>".to_owned()
}
