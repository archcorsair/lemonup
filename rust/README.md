# LemonUp Rust v2

Rust rewrite scaffold for LemonUp.

## Layout

- `crates/lemonup-core`: shared domain model, config/state storage, typed progress events
- `crates/lemonup-app`: Ratatui shell and narrow CLI entrypoint
- `docs/`: acceptance matrix, handoff notes, v1 gaps, and the single-surface shell plan

## Intended commands

```bash
cargo fmt --all
cargo test --workspace
cargo run -p lemonup-app --bin lemonup -- tui
cargo run -p lemonup-app --bin lemonup -- update --dry-run
```
