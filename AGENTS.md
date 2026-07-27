General

- Ask clarifying questions whenever requirements are ambiguous or multiple reasonable implementations exist.
- Never make assumptions that change user-visible behavior, public APIs, or architecture without confirmation.
- Prefer the smallest, simplest, and most idiomatic solution.
- Do not perform unrelated refactoring.
- Seperate code in multiple files, when necessary to have clean structure and not too big files

Planning

Before making any code changes:

1. Inspect the relevant code.
2. Explain your understanding of the problem.
3. Present a short implementation plan including:
   - affected files
   - architectural impact
   - important design decisions
4. Wait for my explicit approval before modifying any files.

If new architectural decisions or unexpected complexity arise during implementation, stop, explain the situation, and ask for approval again.

Rust

- Prefer stable, idiomatic Rust.
- Follow the existing project style and architecture.
- Always make function, struct and enum comments
- Avoid unnecessary cloning and allocations.
- Avoid unwrap() and expect() outside tests.
- Do not introduce new dependencies without approval.

Validation

After implementing Rust code, run:

- cargo fmt
- cargo test
- cargo check
- cargo clippy

If a command cannot be run or fails, report it honestly instead of claiming success.

Safety

Do not start servers, watchers, background processes, containers, or other long-running commands unless I explicitly request it.

Examples include:

- cargo run
- cargo watch
- trunk serve
- dx serve
- docker compose up

Do not create commits, push changes, or modify generated files unless explicitly instructed.
