//! Command-line interface for the Phase-1 `ExS` toolchain.

mod input;

use std::env;
use std::fs;
use std::future::Future;
use std::path::Path;
use std::pin::pin;
use std::process::ExitCode;
use std::task::{Context, Poll, Waker};

use exs_compiler::{
    CompileOptions, ModuleDebugInfo, ModuleResolver, ResolvedSource, SourceInput,
    compile_with_resolver, format, read_debug_info,
};
use exs_runner::{ExecutionCancellation, ExsValue, ServerRunner};

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
        "fmt" if arguments.len() == 2 => format_file(&arguments[1]),
        "run" if arguments.len() >= 2 => {
            let inputs = match arguments.get(2) {
                None => Vec::new(),
                Some(separator) if separator == "--" => input::parse_arguments(&arguments[3..])?,
                Some(_) => return Err("expected `--` before run input values".to_owned()),
            };
            run_program(&arguments[1], &inputs)
        }
        _ => Err(usage()),
    }
}

/// Formats one source file in place using the compiler library formatter.
fn format_file(path: &str) -> Result<(), String> {
    let source =
        fs::read_to_string(path).map_err(|error| format!("could not read {path}: {error}"))?;
    let formatted = format(SourceInput {
        source_id: path,
        text: &source,
    })
    .map_err(|error| error.render(&source))?;
    fs::write(path, formatted).map_err(|error| format!("could not write {path}: {error}"))
}

/// Compiles one source file using the runtime template embedded in the compiler dependency.
fn compile_source(
    path: &str,
    options: CompileOptions,
) -> Result<exs_compiler::CompiledModule, String> {
    let source =
        fs::read_to_string(path).map_err(|error| format!("could not read {path}: {error}"))?;
    let canonical =
        fs::canonicalize(path).map_err(|error| format!("could not resolve {path}: {error}"))?;
    let source_id = canonical.to_string_lossy().into_owned();
    let mut resolver = FileResolver;
    compile_with_resolver(
        SourceInput {
            source_id: &source_id,
            text: &source,
        },
        options,
        &mut resolver,
    )
}

/// Resolves CLI imports through canonical local filesystem paths.
struct FileResolver;

impl ModuleResolver for FileResolver {
    /// Resolves one relative `.exs` import and reads its UTF-8 contents.
    fn resolve(&mut self, importer: &str, path: &str) -> Result<ResolvedSource, String> {
        if !path.starts_with("./") && !path.starts_with("../") {
            return Err("imports must use a relative `./` or `../` path".to_owned());
        }
        if Path::new(path)
            .extension()
            .is_none_or(|extension| extension != "exs")
        {
            return Err("imports must name an `.exs` file".to_owned());
        }
        let base = Path::new(importer)
            .parent()
            .ok_or_else(|| "importing file has no parent directory".to_owned())?;
        let path = fs::canonicalize(base.join(path))
            .map_err(|error| format!("could not resolve file: {error}"))?;
        let text =
            fs::read_to_string(&path).map_err(|error| format!("could not read file: {error}"))?;
        Ok(ResolvedSource {
            source_id: path.to_string_lossy().into_owned(),
            text,
        })
    }
}

/// Executes a source file or linked WebAssembly module.
fn run_program(path: &str, inputs: &[exs_runner::ExsValue]) -> Result<(), String> {
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
    let runner = cli_runner()?;
    let cancellation = ExecutionCancellation::new();
    let result = block_on(runner.execute(&wasm, inputs, &cancellation))
        .map_err(|error| error.to_string())?;
    match result {
        exs_runner::ExsValue::Error(error) => Err(format_error(&error, debug_info.as_ref())),
        result => {
            print_result(result);
            Ok(())
        }
    }
}

/// Creates the CLI runner with its standard synchronous output host functions.
fn cli_runner() -> Result<ServerRunner, String> {
    let mut runner = ServerRunner::new();
    runner
        .registry_mut()
        .register_sync("print", |arguments: Vec<ExsValue>| {
            write_host_output(&arguments, false);
            ExsValue::None
        })
        .map_err(|error| format!("could not register CLI print host function: {error}"))?;
    runner
        .registry_mut()
        .register_sync("println", |arguments: Vec<ExsValue>| {
            write_host_output(&arguments, true);
            ExsValue::None
        })
        .map_err(|error| format!("could not register CLI println host function: {error}"))?;
    Ok(runner)
}

/// Writes host-function arguments to standard output with optional line termination.
fn write_host_output(arguments: &[ExsValue], newline: bool) {
    let output = arguments
        .iter()
        .map(format_host_output)
        .collect::<Vec<_>>()
        .join(" ");
    if newline {
        println!("{output}");
    } else {
        print!("{output}");
    }
}

/// Formats one CLI output host-function argument.
fn format_host_output(value: &ExsValue) -> String {
    match value {
        ExsValue::String(value) => value.clone(),
        value => format_result(value),
    }
}

/// Polls one immediately-ready CLI runner future without adding an executor dependency.
fn block_on<Output>(future: impl Future<Output = Output>) -> Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
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
    let Some(source) = debug_info.and_then(|info| info.source_for(&position.source_id)) else {
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
    let Some(source) = debug_info.and_then(|info| info.source_for(&position.source_id)) else {
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
    "usage: exs check <file.exs> | exs fmt <file.exs> | exs compile <file.exs> -o <file.wasm> | exs run <file.exs|file.wasm> [-- <value> ...]".to_owned()
}
