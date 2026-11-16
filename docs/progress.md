# Progress

## Purpose

Current progress ledger for the Rust LemonUp workspace.

## Current State

- Default branch: `main`
- Workspace root: repo root
- Default seeded manual-smoke profile: `--profile manual-smoke`
- Production AddOns path: do not use for manual testing
- Legacy v1 app: retired and removed from the runnable tree
- Branch: `feat/2.0-release-blockers`
- Activity: the 2.0 TUI redesign is complete (2026-11); before that, last product work landed 2026-04-09
- Releases: 2.0.0 is prepared (workspace version set to `2.0.0` in `Cargo.toml`, `lemonup
  --version` prints `lemonup 2.0.0`) but not yet tagged or published. The latest GitHub release
  (`v0.1.16`, 2026-01-10) and the Scoop/Homebrew manifests still ship the retired v1 Bun binary
  until the 2.0.0 release goes out

## 2.0 TUI Redesign (complete)

The approved redesign (slices 1-20) is implemented on `feat/2.0-release-blockers`. Slice 21, "find
all my addons in one go", is deferred until after 2.0.

- `3e23c2b` release candidate: version 2.0.0, install from a pasted URL, safer updates with
  rollback, per client WTF backups, plain header
- `26d0280` key safety: Ctrl+C quits anywhere, `q` goes back in sheets and quits only from the
  list, `y` is the only yes, PgUp/PgDn/Home/End in the list
- `86e0cdb` new frame: two row header, status line, key line that shows only working keys, `U`
  updates all, `C` checks all, plain status words
- `465242e` sheets: Details, Install, Settings, Backups and Help open as scrolling sheets with
  key lines that fit the width
- `ba08a68` Details: flat view, up and down change addon, docks beside the list at 120 columns
- `27198af` one centered question style, plain wording (Not linked, Newer than source), complete
  Help, welcome card and empty states, `z` undo delete
- `19b767d` filter box: `/` opens a box in the header, the list never moves
- `d372d56` one text box everywhere, Install without edit mode, `f` finds a Not linked addon
  online, clearer errors
- `47c28ee` Settings and Backups sheets: `s` or `,` opens Settings, `b` backs up now, game folder
  and addon list save/load are rows, the Wago.io key is never shown
- `b26866d` setup in three steps: Find your game, Wago.io, Preferences
- `c85c4ab` theme and symbols (Terminal theme, Simple symbols, Reduce motion), per row progress,
  click key hints, wheel scrolling, double click opens Details
- `130d011` Start with folders shown works and takes effect right away

## Verification

- Each wave was smoke tested by the owner in a sandbox.
- `cargo test --workspace` passes: 535 app tests and 100 core tests.

## Before Release

Each step needs the owner's approval.

1. Website screenshots and pages (in progress)
2. Full smoke run
3. Push the branch and open the PR
4. Tag `v2.0.0`
5. Update the Homebrew formula (still says version "0.0.8") and the Scoop manifest
6. Deploy the website

## Completed Through 2026-04-09

- dashboard default sort now starts on Version:
  - update-available addons rank first
  - then rows sort by most recently updated / installed activity
  - version sort toggle still works for reverse ordering
- backup overlay now matches the current modal language:
  - compact summary, recent backups list, selected archive card, centered actions
  - restore and delete both use in-context confirmation
  - backup creation is throttled briefly to avoid accidental spam
- contextual `?` help overlay is implemented:
  - opens from dashboard and active overlays
  - content is grouped by current context: overview, inspect, install/search, config, backup
  - uses the shared modal design language and contextual footer hints
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

- root Rust workspace:
  - `crates/lemonup-core`
  - `crates/lemonup-app`
- typed core domain/config/state + SQLite storage
- safe onboarding, path discovery, and terminal restore
- TUI shell with overview, inspect, install/search, config, and backup overlays
- provider-backed install/check/update for GitHub, TukUI, WoWInterface, and Wago
- overview sort, multi-select, bulk update/delete, undo, and toast/status feedback
- render-first `app.rs` cleanup into `crates/lemonup-app/src/app/` submodules

## Validation Ladder

Default validation remains:

1. `cargo fmt --all`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. seeded sandbox verification via `docs/manual-testing.md`

## Next Up

1. finish the steps in Before Release
2. after 2.0: slice 21, find all my addons in one go
3. keep expanding provider and workflow parity only when product scope demands it

## Related Docs

- `README.md`
- `docs/windows-thread-handoff.md`
- `docs/acceptance-matrix.md`
- `docs/manual-testing.md`
- `docs/v1-archive.md`
- git history if historical design-planning context is needed
