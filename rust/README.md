# LemonUp Rust v2

Rust rewrite for LemonUp.

Important:
- this is the v2 app intended to fully replace the existing TypeScript/Bun v1 app
- the v1 code still exists at the repo root only as a feature-parity and UX reference
- do not extend v1 unless the user explicitly asks

## Layout

- `crates/lemonup-core`: shared domain model, config/state storage, typed progress events
- `crates/lemonup-app`: Ratatui shell and narrow CLI entrypoint
- `docs/`: acceptance matrix, handoff notes, progress tracker, v1 gaps, and the single-surface shell plan

## Read Order

Start here before continuing the rewrite:

1. `docs/windows-thread-handoff.md`
2. `docs/progress.md`
3. `docs/acceptance-matrix.md`
4. `docs/known-v1-gaps.md`
5. `docs/single-surface-shell-plan.md`

## Intended commands

```bash
cargo fmt --all
cargo test --workspace
cargo run -p lemonup-app --bin lemonup -- tui
cargo run -p lemonup-app --bin lemonup -- update --dry-run
cargo run -p lemonup-app --bin lemonup -- update WeakAuras --dry-run
cargo run -p lemonup-app --bin lemonup -- update-all --dry-run
cargo run -p lemonup-app --bin lemonup -- install-tukui ElvUI --dry-run
cargo run -p lemonup-app --bin lemonup -- install-github https://github.com/WeakAuras/WeakAuras2 --dry-run
```

## Current Wago Shell Slice

- `Search` pane: Wago search plus selected-result install
- `Install` pane: direct Wago slug or URL install
- current scope is retail-only and stable-only

## Current Provider Coverage

- Wago:
  - CLI install
  - CLI live check
  - CLI live update
  - TUI search and install
- TukUI:
  - CLI install for canonical `ElvUI` and `Tukui`
  - CLI live check for tracked TukUI addons
  - CLI live update for tracked TukUI addons
- WoWInterface:
  - CLI install by addon page URL
  - CLI live check for tracked WoWInterface addons
  - CLI live update for tracked WoWInterface addons
- GitHub:
  - CLI install by canonical repo URL
  - CLI live check for tracked GitHub addons
  - CLI live update for tracked GitHub addons
