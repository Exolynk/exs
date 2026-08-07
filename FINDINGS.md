**Findings**

- **P2:** The browser playground executes editor-provided source on the UI thread ([playground execution](/Users/roba/Code/exs/examples/playground/src/lib.rs:93)). `BrowserRunner` has no execution limits and the project documentation explicitly requires a dedicated Worker for untrusted code ([README](/Users/roba/Code/exs/README.md:47)). An infinite loop can freeze the playground UI.

- **P2:** `else if` is unsupported. The parser accepts only a block after `else` ([parser](/Users/roba/Code/exs/crates/exs-compiler/src/parser.rs:585)); the AST likewise permits only `Option<Block>`. This contradicts the documented syntax ([specification](/Users/roba/Code/exs/SPECIFICATION.md:260) and [grammar](/Users/roba/Code/exs/SPECIFICATION.md:558)).

- **P2:** Standalone lexical blocks are unsupported. The grammar permits `block` as a statement ([specification](/Users/roba/Code/exs/SPECIFICATION.md:554)), but there is no corresponding AST statement variant ([AST](/Users/roba/Code/exs/crates/exs-compiler/src/ast.rs:215)) or parser branch. A valid `{ let value = 1; }` block fails parsing.
