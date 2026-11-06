#!/usr/bin/env nu

# Freezes the compiled modules of the current version into frozen/<version>. Run it once per
# release, right before publishing; until then, delete the directory of the unpublished version to
# freeze it again. A module of a published version is never replaced: a test that fails on one
# means that a change broke stored modules.
const crate_dir = (path self | path dirname)
const workspace_dir = ($crate_dir | path dirname | path dirname)

cd ($workspace_dir | path join "examples" "rust-guest")
cargo build -q --release --target wasm32-unknown-unknown
cd $workspace_dir
cargo run -q -p exs-compatibility --bin freeze -- target/wasm32-unknown-unknown/release/exs_rust_guest.wasm
