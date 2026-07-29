#!/usr/bin/env nu

# Builds the committed Wasm runtime template used by the ExS compiler.
const crate_dir = (path self | path dirname)
const workspace_dir = ($crate_dir | path dirname | path dirname)

cd $workspace_dir
cargo rustc -p exs-runtime --target wasm32-unknown-unknown --release --crate-type cdylib -- -C link-arg=--export-memory

let template = ($crate_dir | path join "exs-runtime.wasm")
cp target/wasm32-unknown-unknown/release/exs_runtime.wasm $template
