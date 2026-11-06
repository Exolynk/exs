//! Records the hash of the sources of the core template. `build-template.nu` runs it after a build.

use std::fs;
use std::process::ExitCode;

use exs_compatibility::template::{STAMP, sources_hash, workspace};

/// Writes the hash to the stamp file.
fn run() -> Result<(), String> {
    let root = workspace();
    let hash = sources_hash(&root)?;
    let path = root.join(STAMP);
    fs::write(&path, format!("{hash}\n"))
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

/// Runs the tool and reports its failure.
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("stamp_template: {message}");
            ExitCode::FAILURE
        }
    }
}
