# LemonUp Rust Rewrite Handoff

## Start Here

This repo contains the existing TypeScript/Bun LemonUp app **and** an in-progress Rust rewrite under `rust/`.

Before making changes:
1. Follow the root `AGENTS.md`.
2. Treat this as a **clean-break Rust rewrite**, not a direct React/Ink port.
3. Read these files first:
   - `rust/README.md`
   - `rust/docs/acceptance-matrix.md`
   - `rust/docs/known-v1-gaps.md`
   - `rust/docs/single-surface-shell-plan.md`
   - `rust/docs/progress.md`
4. If product context is needed, root `AGENTS.md` says only read:
   - `conductor/product.md`
   - `conductor/product-guidelines.md`
   Ignore the rest of `conductor/`.

## Repo Root

- `C:\Users\archc\ghq\github.com\archcorsair\lemonup`

## Rust Workspace

- `rust\crates\lemonup-core`
- `rust\crates\lemonup-app`
- `rust\docs`

## Current State

Implemented and committed:
- Rust workspace foundation
- typed core domain/config/state setup
- SQLite-backed app state
- OS-native path discovery
- WoW path detection / validation / deep scan
- Ratatui app shell
- panic-safe terminal restore
- typed event/action flow
- onboarding Location Finder
- profile-isolated sandbox mode via `--profile <name>`
- strict `--addon-dir <path>` precondition
- non-default profile guard against using the default/prod AddOns path
- addon scan plus state reconcile core
- single-surface shell foundation
- background scan plus DB-backed shell dashboard

Recent commits:
- `01c6abe` `feat(rust): scaffold v2 foundation and onboarding flow`
- `7c97cb2` `feat(rust): add profile-isolated sandbox mode`
- `91d9227` `feat(rust): add addon scan and state reconcile core`
- `f2ac4a3` `feat(rust): add single-surface shell foundation`
- `c9af6f1` `docs(rust): capture single-surface shell plan`
- `42eb7ab` `feat(rust): wire scan sync into shell dashboard`

## Product / UX Decisions Already Made

Keep these stable unless the user asks to revisit them.

### Rewrite direction
- TUI-first product in Rust using Ratatui.
- Narrow CLI surface only.
- Shared core crate owns logic; app crate owns TUI/CLI shell.

### Scan/onboarding UX contract
Preserve the v1 scan experience, with the refined single-screen Location Finder flow:
- auto-detect first
- editable search root
- deep scan with live progress and cancel
- success actions:
  - `Use this path`
  - `Scan another location`
  - `Edit path manually`

### CLI override rule
- `--addon-dir` is **strict**, not a hint.
- If invalid, fail before starting the TUI.
- If a non-default profile points at the default profile AddOns path, fail.

### Commit policy
- Do **not** commit while iterating.
- Commit only after the user has manually verified the slice.
- Keep commits in logical chunks.

## Existing Docs

### `rust/docs/acceptance-matrix.md`
Use this as the high-level parity target.

### `rust/docs/known-v1-gaps.md`
Use this to avoid porting known v1 problems forward.

### `rust/docs/progress.md`
Use this for current branch status, completed slices, next work, and testing rules.

## Environment Notes

Preferred environment going forward:
- **native Windows / PowerShell**, not WSL

Toolchain lookup rule:
- if a binary is missing from PATH, use:
  - `mise which cargo`
  - `mise which rustc`
  - `mise which node`

Important note:
- a prior Codex thread was rooted in WSL and produced a stale cwd warning
- the correct repo path for new threads is:
  - `C:\Users\archc\ghq\github.com\archcorsair\lemonup`

## Current Worktree Caveat

There may still be one unrelated dirty change in the repo:
- `.node-version` deleted

Do not revert unrelated changes unless the user asks.

## What Has Been Manually Verified By The User

- onboarding scan flow worked
- found-action navigation bug was fixed
- `dev` profile isolation worked
- invalid `--addon-dir` now fails before opening the TUI
- dashboard auto-scan populated real addon rows
- detail pane updated with real addon metadata
- owned-child tree expansion and collapse worked in the shell

## Next Recommended Slice

Harden authoritative ownership and bulk-management foundations:
1. make install and update metadata the authoritative ownership source
2. preserve parent-child links across update and delete flows
3. add multi-select groundwork for bulk update and bulk delete
4. surface drift detection for broken disk or state relationships
5. keep testing on `--profile dev` and avoid live AddOns paths
6. use `rust/docs/progress.md` and `rust/docs/single-surface-shell-plan.md` as active contracts

## Suggested First Commands In A New Windows-Rooted Thread

```powershell
cd C:\Users\archc\ghq\github.com\archcorsair\lemonup
git status
mise which node
mise which cargo
```

If Rust is active:

```powershell
cd C:\Users\archc\ghq\github.com\archcorsair\lemonup\rust
cargo test --workspace
cargo run -p lemonup-app --bin lemonup -- --profile dev tui
```

## Suggested First Prompt In The New Thread

`Continue LemonUp Rust rewrite. Repo root is C:\Users\archc\ghq\github.com\archcorsair\lemonup. Read rust/README.md and rust/docs/*. Next slice: addon scan + state sync, test-first. Respect existing UX decisions and commit only after my manual verification.`
