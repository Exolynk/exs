**Findings**

- **P1:** Closures capturing a `for` loop variable trap at runtime. The continuation lowerer marks the loop binding as a cell but never emits `CellNew` after loading the iteration item ([graph builder](/Users/roba/Code/exs/crates/exs-compiler/src/codegen/continuation/graph_builder.rs:259)). Closure lowering then passes that non-cell as a capture ([capture lowering](/Users/roba/Code/exs/crates/exs-compiler/src/codegen/continuation/graph_expression.rs:95)), causing a Wasm trap in `__exs_rt_closure_new`. This also fails the requirement that each iteration has a fresh closure binding.

- **P2:** The browser playground executes editor-provided source on the UI thread ([playground execution](/Users/roba/Code/exs/examples/playground/src/lib.rs:93)). `BrowserRunner` has no execution limits and the project documentation explicitly requires a dedicated Worker for untrusted code ([README](/Users/roba/Code/exs/README.md:47)). An infinite loop can freeze the playground UI.

- **P2:** Uninitialized bindings are rejected. The parser requires `=` after every `let` ([parser](/Users/roba/Code/exs/crates/exs-compiler/src/parser.rs:553)), but `let value;` is valid and must initialize to `None` per the [specification](/Users/roba/Code/exs/SPECIFICATION.md:152). Reproduced as syntax error `E0103`.

- **P2:** `else if` is unsupported. The parser accepts only a block after `else` ([parser](/Users/roba/Code/exs/crates/exs-compiler/src/parser.rs:585)); the AST likewise permits only `Option<Block>`. This contradicts the documented syntax ([specification](/Users/roba/Code/exs/SPECIFICATION.md:260) and [grammar](/Users/roba/Code/exs/SPECIFICATION.md:558)).

- **P2:** Standalone lexical blocks are unsupported. The grammar permits `block` as a statement ([specification](/Users/roba/Code/exs/SPECIFICATION.md:554)), but there is no corresponding AST statement variant ([AST](/Users/roba/Code/exs/crates/exs-compiler/src/ast.rs:215)) or parser branch. A valid `{ let value = 1; }` block fails parsing.
