# ExS Browser Playground

This Trunk example compiles the source entered in its editor and executes the resulting ExS WebAssembly module in the browser. Both compilation and host callbacks run in the Rust-Wasm application; no server is involved.

Install Trunk and the Rust `wasm32-unknown-unknown` target, then run this command from this directory:

```sh
trunk serve
```

`print` and `println` are registered as browser host functions and append their arguments to the Output panel. The final `main` result is appended after execution completes.
