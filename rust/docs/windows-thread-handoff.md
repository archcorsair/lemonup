# LemonUp Rust Rewrite Handoff

## Start Here

This repo contains two codepaths:
- the original TypeScript/Bun LemonUp app under `src/`
- the in-progress Rust v2 rewrite under `rust/`

Critical framing:
- Rust v2 is intended to **fully replace** the v1 TypeScript/Bun app.
- The v1 app still exists only as a reference point for feature parity, UX spirit, source logic comparisons, and known-gap avoidance.
- Do **not** continue building new product work in the v1 app unless the user explicitly asks.
- Do **not** treat the v1 implementation as authoritative architecture; use it as a product/reference baseline only.

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

## Current Branch / Status

- active branch: `codex/rusty-lemon`
- latest pushed commit at handoff time: `64e4ec8` `feat(rust): add live Wago update flow`
- current worktree status at handoff pass: clean

## Current State

Implemented, committed, and pushed:
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
- relationship-safe reconcile rules for managed parents
- explicit ownership authority persisted in state:
  - `none`
  - `scan_inferred`
  - `managed`
- parent delete cascade through authoritative owned descendants
- first tree-ready shell with parent-child expansion and collapse
- parent-only multi-select groundwork
- bulk delete flow in shell
- selected-update groundwork in shell
- drift indicators in shell
- post-action resync through the same dashboard scan path
- richer update pane summaries and refreshable-selection helpers
- richer tree UX:
  - expand/collapse all
  - sibling-aware tree connectors
  - mouse wheel navigation
  - click-to-highlight
  - drag-to-highlight
  - stable logical selection across tree expansion/collapse
- soft-delete to LemonUp-managed trash with shell undo
- first real source-backed install flow via Wago CLI
- first real live provider-backed update checks via Wago CLI
- first real live provider-backed update flow via Wago CLI
- update-all CLI contract
- targeted CLI update selectors with per-addon result reporting
- explicit TUI update-pane refresh wording to distinguish tracked-state refresh from live apply/update

## Recent Important Commits

Most relevant recent milestones:
- `64e4ec8` `feat(rust): add live Wago update flow`
- `7c3ffaa` `feat(rust): add live Wago update checks`
- `be3320d` `feat(rust): add Wago install flow`
- `c5c68fb` `feat(rust): add soft-delete undo flow`
- `11faa2c` `fix(rust): preserve selection across tree expansion`
- `a7c8a9f` `feat(rust): improve tree navigation and mouse input`
- `472b784` `feat(rust): enrich update selection workflow`
- `e32018d` `feat(rust): add drift indicators to shell`
- `65813ef` `feat(rust): add selected update shell groundwork`
- `234de04` `feat(rust): add bulk delete shell groundwork`
- `6fec626` `feat(rust): add shell multi-select groundwork`
- `d009a48` `feat(rust): add cli check command`
- `3f56236` `feat(rust): add managed update refresh path`
- `25ac841` `feat(rust): add managed ownership write path`

## Product / UX Decisions Already Made

Keep these stable unless the user asks to revisit them.

### Rewrite direction
- TUI-first product in Rust using Ratatui.
- Narrow CLI surface only.
- Shared core crate owns logic; app crate owns TUI/CLI shell.
- Single-surface shell, not routed multi-screen navigation.
- Rust v2 should fully replace the v1 TS/Bun app when feature-complete.

### v1 reference rule
- v1 TS/Bun code remains in the repo as a feature-parity and UX reference only.
- Use v1 to understand:
  - intended features
  - desired UX spirit
  - API/schema edge cases already encountered
- Do **not** treat v1 code structure as the template for Rust v2.
- Do **not** spend time re-evaluating the whole v1 codebase unless the user explicitly asks.

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

### Commit / docs workflow
- Do **not** commit while iterating.
- Commit only after the user has manually verified the slice.
- Keep commits in logical chunks.
- When the user says `checkpoint`, update relevant docs under `rust/docs` before commit.
- At minimum, refresh `rust/docs/progress.md` on checkpoint when capabilities or next-step order changed.

### Relationship integrity rule
Treat this as a hard invariant, not UI polish:
- parent update must preserve child ownership links
- parent delete must remove owned children too
- managed child folders must not split into standalone addons after update
- scan-only inference must stay conservative
- install/update metadata should be the authoritative ownership source where possible
- both flat parent view and expandable tree view must be supported

## Existing Docs

### `rust/docs/acceptance-matrix.md`
Use this as the high-level parity target.

### `rust/docs/known-v1-gaps.md`
Use this to avoid porting known v1 problems forward.

### `rust/docs/single-surface-shell-plan.md`
Use this as the shell/design contract.

### `rust/docs/progress.md`
Use this as the active ledger for:
- completed slices
- current branch status
- next work
- testing rules
- current MVP target

## Environment Notes

Preferred environment going forward:
- **native Windows / PowerShell**, not WSL

Toolchain lookup rule:
- if a binary is missing from PATH, use:
  - `mise which cargo`
  - `mise which rustc`
  - `mise which node`

Important note:
- a prior Codex thread was rooted in WSL and produced stale cwd friction
- the correct repo path for new threads is:
  - `C:\Users\archc\ghq\github.com\archcorsair\lemonup`

## Provider / API Notes

### Wago
- Wago is the first real source-backed provider wired in Rust v2.
- Current live paths implemented:
  - `install-wago`
  - `check` for tracked Wago addons
  - `update` for managed Wago addons
- Wago detail parsing already handles v1-observed API drift:
  - wrapped or direct addon response
  - `releases` or `recent_release`
  - `download_link` or `link`
- API key resolution order is:
  1. config
  2. process env `WAGO_API_KEY`
  3. repo-root `.env` key `WAGO_API_KEY`
- Do **not** manually inspect `.env` just to read the key in a new session if runtime resolution already works.

### Skill/tooling note
- Do **not** use the `native-data-fetching` skill for LemonUp Rust networking work.
- That skill is Expo/React Native specific and is the wrong fit for this repo.

## What Has Been Manually Verified By The User

User-verified so far:
- onboarding scan flow worked
- found-action navigation bug was fixed
- `dev` profile isolation worked
- invalid `--addon-dir` now fails before opening the TUI
- dashboard auto-scan populated real addon rows
- detail pane updated with real addon metadata
- owned-child tree expansion and collapse worked in the shell
- mouse wheel navigation works one row at a time
- left-click row highlight works
- left-button drag-to-highlight works
- parent-only selection model works; child rows are highlightable but not actionable
- bulk delete flow worked
- soft-delete undo flow worked
- Wago install worked on `--profile dev`
- TUI delete/undo worked on the installed Wago addon
- live Wago `check` worked for `WeakAuras`
- live Wago `update` worked
- `update --force --dry-run` and `update --force` behaved correctly
- TUI still rendered correct relationships after Wago update
- delete and undo still worked after Wago update

## Current MVP Boundary

Already proven in some form:
- install one addon from Wago via CLI
- check one addon against Wago live metadata via CLI
- update one managed Wago addon via CLI
- update all tracked addons via CLI
- delete one or many selected parents in the TUI
- undo delete
- relationship-safe parent/child handling in managed flows

Still missing or incomplete for MVP:
- update selected against real provider-backed updates in the TUI
- Wago search flow
- TUI install UX
- broader provider/source parity
- richer config/backup workflows

## Next Recommended Slice

Next real phase:
1. layer Wago search and TUI install UX on top
2. broaden provider/source coverage using the same managed ownership contract
3. replace tracked-state refresh in the TUI update pane with real provider-backed selected updates

Why this is next:
- Wago install, live check, update-all, and targeted update are now proven in the CLI
- biggest remaining MVP gap is search/install UX inside the shell plus broader provider coverage
- the ownership model is already hardened enough to build on

## Suggested First Commands In A New Windows-Rooted Thread

```powershell
cd C:\Users\archc\ghq\github.com\archcorsair\lemonup
git status
git log --oneline -n 10
mise which cargo
mise which node
```

If Rust is active:

```powershell
cd C:\Users\archc\ghq\github.com\archcorsair\lemonup\rust
mise exec rust@latest -- cargo test --workspace
mise exec rust@latest -- cargo run -q -p lemonup-app --bin lemonup -- --profile dev check WeakAuras
mise exec rust@latest -- cargo run -q -p lemonup-app --bin lemonup -- --profile dev tui
```

## Suggested First Prompt In A New Thread

`Continue LemonUp Rust rewrite on codex/rusty-lemon. Repo root is C:\Users\archc\ghq\github.com\archcorsair\lemonup. Read rust/README.md, rust/docs/windows-thread-handoff.md, and rust/docs/progress.md first. Treat the TS/Bun app as feature-parity reference only; do not extend it. CLI update-all and targeted update selectors are already landed; next slice is Wago search plus TUI install UX, then broader provider parity and real provider-backed selected updates in the TUI. Respect existing safety/relationship invariants and commit only after my manual verification.`
