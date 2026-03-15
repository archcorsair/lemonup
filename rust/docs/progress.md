# Rust Rewrite Progress

## Purpose

This is the current progress ledger for the LemonUp Rust rewrite on `codex/rusty-lemon`.

Use it as the quick status page before reading deeper docs.

## Current State

- Branch: `codex/rusty-lemon`
- Rust workspace: active
- Default manual test profile: `--profile dev`
- Production AddOns path: do not use for manual testing

## Completed

These chunks are implemented, user-verified, committed, and pushed.

### Foundation

- Rust workspace split into:
  - `crates/lemonup-core`
  - `crates/lemonup-app`
- typed domain/config/state setup
- SQLite-backed state storage
- OS-native path discovery
- panic-safe terminal restore
- typed event/action flow

### Onboarding and safety rails

- Location Finder onboarding flow
- auto-detect first
- editable root
- deep scan with progress and cancel
- strict `--addon-dir <path>` validation
- non-default profile guard against targeting the default/prod AddOns path
- profile-isolated sandbox mode via `--profile <name>`

### Single-surface shell

- one persistent shell instead of top-level multi-screen routing
- inline onboarding takeover inside the same shell
- left addon list pane
- right detail/action pane
- integrated detail modes:
  - overview
  - install
  - search
  - update
  - config
  - backup

### Scan and sync

- retail AddOns directory scan
- TOC selection and parsing
- color-code stripping
- embedded library detection
- conservative owned-folder collapse from scan only
- SQLite reconcile
- auto-scan on app boot when addon dir is valid
- auto-scan after onboarding save
- real DB-backed addon rows in the shell
- real selected-addon metadata in the detail pane

### Regression fixes already landed

- Windows path handling fix in core tests
- UTF-8-safe TOC field parsing for localized metadata such as `Notes-ruRU`

## In Progress

Current checkpoint state:

- no active half-finished slice should be considered stable until the next chunk starts
- latest completed work wired the first real managed ownership caller through the update command

## Newly Completed

These are implemented and manually verified in addition to the earlier chunks:

- relationship-safe reconcile rules for managed parents
- parent delete cascade through owned descendants
- first tree-ready shell expansion and collapse for owned child rows
- tightened MVP and drift-detection requirements in Rust docs
- explicit `ownership_source` tracking in core state:
  - `none`
  - `scan_inferred`
  - `managed`
- SQLite migration to persist ownership authority
- delete cascade now follows managed ownership only
- scan-inferred relationships no longer trigger child cascade delete
- added regression coverage for managed vs scan-inferred ownership behavior
- dedicated `record_managed_addon(...)` core API for install/update flows
- managed writes now remove represented standalone child rows
- managed updates now replace prior owned-folder sets while preserving install time
- non-interactive `update` now performs a real managed-state refresh from disk scan
- `update --dry-run` now computes refresh results without mutating state
- unmanaged/manual rows are skipped instead of guessed into managed ownership

## Next Up

Next phase:

1. shell multi-select groundwork
2. bulk action groundwork for update selected and delete selected
3. drift detection surfaced in the UI
4. relationship-safe update and delete behavior end-to-end
5. richer tree and management UX on top of the hardened model

### Goal

- stop relying on scan heuristics for important parent-child truth in managed flows
- make install and update flows write the authoritative managed folder set
- let shell bulk actions operate safely on that model

### Planned order

1. shell multi-select state
2. bulk delete and bulk update actions
3. drift indicators in list and detail views
4. expose update-state summaries and selection-aware actions in the shell
5. richer tree and relationship UX

## Remaining Major Work

Still missing or incomplete:

- richer tree view for parent/child relationships
- install flows
- update flows
- update all and update selected flows
- search flows
- delete selected flow
- multi-select
- config editing
- backup workflows
- broader source parity behavior
- CLI update path beyond foundation

## Minimum MVP

Current MVP target:

- install addon from supported sources
- update one addon
- update all addons
- update selected addons
- delete one addon
- delete selected addons
- search addons from Wago
- view addon metadata and details
- preserve parent-child ownership correctly during install, update, delete, and rescan

Supporting behavior expected for MVP quality:

- multi-select
- bulk actions
- status and progress feedback
- destructive-action confirmation
- update-check visibility
- rescan or reconcile command
- source and state indicators

## Relationship Rules

These are hard requirements going forward:

- parent update must preserve child ownership links
- parent delete must remove owned children too
- managed child folders must not split into standalone addons after update
- scan-only inference must stay conservative
- install/update metadata should become the authoritative ownership source where possible
- flat parent view and optional expanded tree view must both be supported

## Drift Detection Rules

The app should eventually detect:

- folder on disk with no tracked record
- tracked record with no folder on disk
- parent record with missing owned child on disk
- child folder on disk with no matching parent ownership record
- tracked ownership that no longer matches disk contents
- tracked source metadata that materially disagrees with disk state

## Testing Rules

- validate with `--profile dev`
- never manually test against the live production AddOns folder
- prefer focused Rust tests per slice
- keep adding regression coverage for ownership/relationship cases

Useful commands:

```powershell
cd C:\Users\archc\ghq\github.com\archcorsair\lemonup\rust
mise exec rust@latest -- cargo fmt --all
mise exec rust@latest -- cargo test --workspace
mise exec rust@latest -- cargo run -p lemonup-app --bin lemonup -- --profile dev tui
```

## Commit Workflow

- work in logical chunks
- user manually tests each chunk before commit
- commit only after user approval

## Related Docs

- `rust/README.md`
- `rust/docs/windows-thread-handoff.md`
- `rust/docs/acceptance-matrix.md`
- `rust/docs/known-v1-gaps.md`
- `rust/docs/single-surface-shell-plan.md`
