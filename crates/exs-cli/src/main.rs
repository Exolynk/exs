//! Command-line interface for the Phase-1 `ExS` toolchain.

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use exs_compiler::{CompileOptions, SourceInput, compile};

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
            let _module = compile_source(&arguments[1])?;
            Ok(())
        }
        "compile" if arguments.len() == 4 && arguments[2] == "-o" => {
            let module = compile_source(&arguments[1])?;
            fs::write(&arguments[3], module.wasm)
                .map_err(|error| format!("could not write {}: {error}", arguments[3]))?;
            Ok(())
        }
        "run" if arguments.len() == 2 => run_program(&arguments[1]),
        _ => Err(usage()),
    }
}

/// Compiles one source file using the runtime template embedded in the compiler dependency.
fn compile_source(path: &str) -> Result<exs_compiler::CompiledModule, String> {
    let source =
        fs::read_to_string(path).map_err(|error| format!("could not read {path}: {error}"))?;
    compile(
        SourceInput {
            source_id: path,
            text: &source,
        },
        CompileOptions,
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
        compile_source(path)?.wasm
    };
    let result = exs_runner::execute(&wasm, exs_runner::ExsValue::None)
        .map_err(|error| error.to_string())?;
    print_result(result);
    Ok(())
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

/// Returns CLI usage text.
fn usage() -> String {
    "usage: exs check <file.exs> | exs compile <file.exs> -o <file.wasm> | exs run <file.exs|file.wasm>".to_owned()
}
