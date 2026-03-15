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

- bulk delete groundwork is the active slice
- confirmation flow is being added on top of parent-only multi-select
- no active half-finished slice should be considered stable until user verifies it

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
- CLI-only `check` companion command now exists
- `check` supports:
  - global check with no addon arguments
  - exact-match targeted checks for one addon or a list of addons
  - sanitized selector parsing with path-like and shell-like input rejected
- current `check` behavior uses tracked metadata only:
  - `up_to_date`
  - `update_available`
  - `unknown`
  - no remote refresh yet
- shell multi-select groundwork now exists
- parent rows are the only selectable bulk-action target
- child rows remain visual-only relationship context
- multi-select keys now support:
  - `space` toggle selected parent
  - `a` select all parent rows
  - `esc` clear selection
- selected-count and bulk-target context are now visible in the shell
- bulk delete groundwork now exists in the shell:
  - `x` requests delete for selected parent rows
  - `y` confirms a pending delete
  - `n` or `esc` cancels a pending delete
  - delete operates on selected parent rows only
  - owned child folders are removed through authoritative ownership rules

## Next Up

Next phase:

1. bulk action groundwork for update selected and delete selected
2. drift detection surfaced in the UI
3. relationship-safe update and delete behavior end-to-end
4. expose update-state summaries and selection-aware actions in the shell
5. richer tree and management UX on top of the hardened model

### Goal

- stop relying on scan heuristics for important parent-child truth in managed flows
- make install and update flows write the authoritative managed folder set
- let shell bulk actions operate safely on that model

### Planned order

1. bulk delete and bulk update actions
2. drift indicators in list and detail views
3. expose update-state summaries and selection-aware actions in the shell
4. richer tree and relationship UX
5. soft-delete trash model with undo for safer destructive operations

## Remaining Major Work

Still missing or incomplete:

- richer tree view for parent/child relationships
- install flows
- update flows
- update all and update selected flows
- remote-backed update checks beyond tracked metadata
- search flows
- delete selected flow
- config editing
- backup workflows
- broader source parity behavior
- CLI update path beyond foundation
- soft-delete trash/undo flow for delete operations

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
- safer destructive UX, ideally via soft-delete undo after MVP groundwork

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

## Delete Safety Plan

- current groundwork uses hard delete with explicit confirmation
- follow-up target is soft-delete, not fake undo after hard delete
- desired undo model:
  - move removed addon folders into LemonUp-managed trash
  - keep authoritative parent-child relationships intact in trash metadata
  - offer short-lived undo from the shell after delete completes
  - purge trash on an explicit future action or retention policy

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
