//! Writes the compiled modules of this version to `frozen/<version>`. Run it through
//! `freeze.nu`, which also supplies the Rust guest.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::{env, fs};

use exs_compatibility::{RUST_GUEST, SCRIPTS, compile_script};

/// Writes `bytes` to the module `name` of `directory`, refusing to replace a frozen module.
fn freeze(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    let path = directory.join(format!("{name}.wasm"));
    if path.exists() {
        return Err(format!("{} is frozen", path.display()));
    }
    fs::write(&path, bytes).map_err(|error| format!("could not write {}: {error}", path.display()))
}

/// Freezes the scripts and the Rust guest at the path given as the only argument.
fn run() -> Result<(), String> {
    let guest = env::args()
        .nth(1)
        .ok_or("usage: freeze <rust guest module>")?;
    let guest = fs::read(&guest).map_err(|error| format!("could not read {guest}: {error}"))?;
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("frozen")
        .join(env!("CARGO_PKG_VERSION"));
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    for (name, source) in SCRIPTS {
        freeze(&directory, name, &compile_script(name, source)?)?;
    }
    freeze(&directory, RUST_GUEST, &guest)
}

/// Runs the tool and reports its failure.
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("freeze: {message}");
            ExitCode::FAILURE
        }
    }
}
