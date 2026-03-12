# LemonUp Rust v2

Rust rewrite scaffold for LemonUp.

## Layout

- `crates/lemonup-core`: shared domain model, config/state storage, typed progress events
- `crates/lemonup-app`: Ratatui shell and narrow CLI entrypoint
- `docs/`: acceptance matrix and captured v1 gaps to avoid porting blindly

## Intended commands

```bash
cargo fmt --all
cargo test --workspace
cargo run -p lemonup-app --bin lemonup -- tui
cargo run -p lemonup-app --bin lemonup -- update --dry-run
```
