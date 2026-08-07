# ExS Browser Playground

This Trunk example uses Leptos and Birei's code editor to compile source and execute the resulting ExS WebAssembly module in the browser. Both compilation and host callbacks run in the Rust-Wasm application; no server is involved.

Install Trunk and the Rust `wasm32-unknown-unknown` target, then run this command from this directory:

```sh
trunk serve
```

`print` and `println` are registered as browser host functions and append their arguments to the Output panel. The final `main` result is appended after execution completes. Leaving the editor formats valid ExS source automatically. The documentation pane exposes generated built-in `std` pages and declarations from the current valid editor source.
