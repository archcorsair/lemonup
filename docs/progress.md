# Rewrite Progress

## Purpose

Current progress ledger for the Rust LemonUp workspace on `codex/rusty-lemon`.

## Current State

- Branch: `codex/rusty-lemon`
- Workspace root: repo root
- Default seeded manual-smoke profile: `--profile manual-smoke`
- Production AddOns path: do not use for manual testing
- Legacy v1 app: retired and removed from the runnable tree

## Newly Completed

- import/export addon list is implemented in Rust:
  - portable JSON transfer format in `crates/lemonup-core/src/transfer.rs`
  - CLI commands: `export-addons`, `import-addons`
  - TUI config entry path for export/import
  - import analysis is non-destructive by default and reports duplicates/collisions
- background auto-check is implemented:
  - freshness-aware dashboard refresh on tick
  - status is surfaced through the dashboard toast path without modal spam
  - respects provider capability and Wago API-key constraints
- backup restore is implemented:
  - CLI `restore-backup [archive]`
  - backup overlay restore flow with confirmation
  - rollback protection if restore fails mid-flight
- repo retirement cutover is complete:
  - root CI, release workflow, hooks, and README are Rust-only
  - v1 Bun/Ink code and tests are removed from the repo
  - Rust workspace contents now live at the repo root
  - v1 is archived briefly in `docs/v1-archive.md`
- manual-smoke seeding is hardened:
  - sandbox reset preserves profile config
  - partial profile configs load with Rust defaults instead of failing deserialization
  - seed preflight now fails honestly for invalid Wago credentials instead of wiping the profile first

## Completed Foundation

- Rust workspace split into:
  - `crates/lemonup-core`
  - `crates/lemonup-app`
- typed core domain/config/state setup
- SQLite-backed state storage
- OS-native path discovery
- panic-safe terminal restore
- typed event/action flow
- onboarding stepper wizard with strict addon-dir safety checks
- single-surface shell with overview, inspect, install/search, config, and backup overlays
- provider-backed install/check/update flows for GitHub, TukUI, WoWInterface, and Wago
- unified install/search overlay opened by both `i` and `/`
- overview sort, multi-select, bulk update/delete, soft-delete undo, and inspect action chips
- floating timed dashboard toast with semantic `ℹ` / `✓` / `×` feedback
- render-first `app.rs` cleanup into `crates/lemonup-app/src/app/` submodules

## Validation Ladder

Default validation remains:

1. `cargo fmt --all`
2. `cargo clippy -p lemonup-app -- -D warnings`
3. `cargo test --workspace`
4. seeded sandbox verification via `docs/manual-testing.md`

## Next Up

1. continue product polish and post-parity UX work
2. add the future `?` help overlay / action discovery work
3. keep expanding provider and workflow parity only when product scope demands it

## Related Docs

- `README.md`
- `docs/windows-thread-handoff.md`
- `docs/acceptance-matrix.md`
- `docs/manual-testing.md`
- `docs/v1-archive.md`
- `docs/single-surface-shell-plan.md`
- `docs/production-tui-design-plan.md`
