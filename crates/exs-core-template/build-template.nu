#!/usr/bin/env nu

# Builds the committed Wasm template of the runtime v2 core library.
const crate_dir = (path self | path dirname)
const workspace_dir = ($crate_dir | path dirname | path dirname)

cd $workspace_dir
let cargo_home = ($env.CARGO_HOME? | default $"($env.HOME)/.cargo")
let remap_flag = $"--remap-path-prefix=($cargo_home)=/cargo"
let rustflags = ($env.RUSTFLAGS? | default "")
$env.RUSTFLAGS = if ($rustflags | is-empty) {
  $remap_flag
} else {
  $"($rustflags) ($remap_flag)"
}
# One codegen unit lets the optimizer inline across the whole core, which lowers fuel.
with-env { CARGO_PROFILE_RELEASE_CODEGEN_UNITS: "1" } {
  cargo rustc -p exs-core-template --features template --target wasm32-unknown-unknown --release --crate-type cdylib -- -C link-arg=--export-memory -C link-arg=--export=__heap_base -C link-arg=-zstack-size=262144
}

let template = ($crate_dir | path join "exs-core-template.wasm")
cp target/wasm32-unknown-unknown/release/exs_core_template.wasm $template

# The test that the template matches its sources compares with this record.
hide-env RUSTFLAGS
cargo run -q -p exs-compatibility --bin stamp_template
