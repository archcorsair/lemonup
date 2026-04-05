# Rust Rewrite Progress

## Purpose

This is the current progress ledger for the LemonUp Rust rewrite on `codex/rusty-lemon`.

Use it as the quick status page before reading deeper docs.

## Current State

- Branch: `codex/rusty-lemon`
- Rust workspace: active
- Default seeded manual-smoke profile: `--profile manual-smoke`
- Ad hoc manual test profile: `--profile dev`
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

- stepper-style onboarding wizard
- auto-detect first
- editable root
- deep scan with progress and cancel
- steps:
  - Theme
  - Directory
  - Wago
  - Settings
  - Review
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
- narrow CLI `sync` command for deterministic sandbox/manual-fixture reconcile
- real DB-backed addon rows in the shell
- real selected-addon metadata in the detail pane

### Regression fixes already landed

- Windows path handling fix in core tests
- UTF-8-safe TOC field parsing for localized metadata such as `Notes-ruRU`

## In Progress

Current checkpoint state:

- no active unverified Rust slice in the worktree
- current focus:
  - continue overlay-by-overlay production redesign passes
  - keep converging footer/help/action language across dashboard and overlays
  - return later for backup restore and deeper help/menu affordances
## Newly Completed

These are implemented and manually verified in addition to the earlier chunks:

- relationship-safe reconcile rules for managed parents
- split-rail footer redesign:
  - footer is now a fixed two-line control dock instead of a raw status/debug panel
  - left rail stays quiet by default and only shows meaningful progress, results, or errors
  - right rail shows grouped command hints with sticky context emphasis
  - recognized keys now pulse the matching footer hint
  - footer copy is humanized; internal strings like `selected update complete...` no longer surface verbatim
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
- Wago single-folder managed installs now remain authoritative instead of degrading to unmanaged
- provider-managed ownership no longer depends on owned child folders being present
- long non-GitHub version labels now use middle truncation so differing suffixes remain visible
- Wago version normalization now ignores leading punctuation like #, preventing false immediate updates after install
- inspect overlay redesign is now checkpointed:
  - command-palette card presentation replaces the old raw metadata dump
  - child-row inspect resolves to the parent addon with child context
  - in-card actions use split key/action chips and live at the bottom of the card
  - `r`, `d`, and `t` reveal collapsible detail sections
  - update-available status in inspect now pulses gently and carries the target version
  - inspect details support scrolling with `j/k` or `↑/↓`
  - overflow affordances now sit below the detail content instead of overwriting it
- dashboard command dock cleanup is now checkpointed:
  - the footer command dock now shows only core table actions
  - dashboard footer actions disappear entirely while overlays are open
  - transient dashboard status moved into a single-line event rail above the table
  - core footer actions now render in a width-aware two-row grid instead of a loose flow
- dashboard toast and wording polish is now checkpointed:
  - dashboard event feedback now renders as a floating toast pill in the header instead of shifting table layout
  - only one toast is shown at a time; the latest meaningful toast wins and auto-dismisses after a short timer
  - toast symbols are semantic and color-tinted:
    - `ℹ` info
    - `✓` success
    - `×` error
  - inspect wording now uses player-facing labels:
    - `Included addons`
    - `Addon info`
  - inspect and dashboard selection wording now uses `deselect` instead of `clear`
  - dashboard result copy is more human:
    - single-addon update success shows the addon name
    - recently checked rows no longer surface raw `cached` wording
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
- update-selected groundwork now exists in the shell:
  - update panel summarizes tracked status for selected parent rows
  - `r` refreshes selected tracked addons from current disk state
  - refresh stays parent-only and ownership-safe
  - refresh summary remains visible in the update panel after completion
- drift indicator groundwork now exists:
  - last scan records imported disk-only folders
  - last scan records removed missing tracked records
  - last scan records orphaned owned children on disk
  - parent rows show drift markers when tracked owned children are missing on disk
  - overview detail panel surfaces selected-parent drift and last-scan drift summary
- post-delete and post-update now resync through the same dashboard scan path:
  - fresh addon rows after action completion
  - fresh drift report after action completion
  - fresh scan summary after action completion
  - less stale UI state between destructive and non-destructive actions
- refresh UX cleanup landed:
  - list viewport and selection stay stable across refresh-driven row replacement
  - zero-result update refresh status is human-readable
  - overview drift summary stays readable in one line
- richer update-state summaries and selection-aware actions now exist:
  - update pane shows inventory and selection readiness before refresh
  - `v` selects refreshable tracked parent addons only
- first richer tree UX slice now exists:
  - `]` expands all relationship rows
  - `[` collapses all relationship rows
  - list rows show clearer tree glyphs with sibling-aware connectors and indentation
  - expanded tree rows are now single-line only; list metadata noise was removed
  - overview detail shows relationship source and child-folder summaries
  - mouse wheel navigation is now explicitly captured and moves one row per step
  - left-click now highlights the clicked list row
  - left-button drag now moves highlight with the cursor
  - mouse interactions stay selection-only; parent-only action rules are unchanged
  - expand-all and collapse-all now preserve logical selection when tree rows are inserted or removed above the current highlight
- soft-delete undo flow now exists:
  - delete moves selected parent addons plus authoritative owned children into LemonUp-managed trash
  - `z` undoes the last delete batch
  - undo restores managed relationships and on-disk folders together
- first real source-backed install flow now exists via Wago CLI:
  - `install-wago` accepts a Wago slug or addon URL
  - Wago details parsing follows v1 API drift handling:
    - wrapped or direct addon response
    - `releases` or `recent_release`
    - `download_link` or `link`
  - API key resolution now checks:
    - config
    - process env `WAGO_API_KEY`
    - repo-root `.env` key `WAGO_API_KEY`
  - downloaded archives are extracted safely with path traversal protection
  - discovered folder sets are written as authoritative managed ownership
  - dry-run and real install were manually verified on `--profile dev`
  - installed Wago addons participate correctly in TUI delete and undo flows
- first real provider-backed update check now exists via Wago CLI:
  - `check <addon>` performs a live Wago lookup for tracked Wago addons
  - live checks refresh `remote_version` and `last_checked_at` in state
  - manual addons still report explicit `unknown`
  - non-Wago providers report explicit `unknown` until their live check paths are implemented
  - targeted live check was manually verified on `--profile dev`
- first real provider-backed update path now exists via Wago CLI:
  - `update` applies live Wago package updates for managed Wago addons
  - `update --force` re-applies the current remote package for safe dev verification
  - `update --force --dry-run` proves the update path without mutating disk
  - updates preserve authoritative owned-folder relationships
  - updates remove folders no longer shipped by the package
  - updates preserve install time and kind-override behavior through managed state writes
  - post-update `check`, TUI relationship rendering, delete, and undo were manually verified on `--profile dev`
- broader CLI update contract now exists:
  - `update` with no selectors is the update-all path
  - `update-all` is an explicit alias for update-all
  - `update <addon...>` accepts one or more exact addon or folder selectors
  - update selectors reuse the same sanitized exact-match rules as `check`
  - targeted update results now report per-addon outcome details
  - update-all output now highlights only non-success per-addon details by default
- TUI selected updates now use real provider-backed update paths:
  - `r` applies live selected updates for managed tracked parent addons
  - the same provider-backed update engine now serves both CLI and TUI
  - post-update runs resync dashboard rows and drift state through the normal scan path
  - update pane copy and summaries now reflect real apply/update behavior
- production TUI design/build is now active:
  - Phase 1 shell foundation is implemented:
    - extracted shell chrome helpers
    - full + compact ASCII `LEMONUP` header variants
    - motion tick/spinner primitives
    - adaptive tick now runs idle at `250ms` and animation/job states at `100ms`
  - overview now supports inline action-first workflow for the two highest-frequency operations:
    - `c` performs a live/cached freshness-aware check from overview
    - `u` performs update from overview without opening the update overlay
    - `,` opens config after `c` was reclaimed for check
  - live action feedback now exists in-shell:
    - shimmer is used for header/footer action text
    - row-local progress spinner/tint is used while update jobs run
    - the large `LEMONUP` logo remains static
  - false-positive same-version update regressions are fixed:
    - non-GitHub providers normalize leading `v` prefixes consistently
    - GitHub sync now preserves commit metadata instead of erasing it during scan reconcile
    - overlay host scaffold
  - Phase 2 dense addon table is implemented:
    - main dashboard now uses a compact table instead of prose rows
    - compact source/version/state/flag columns now exist
    - tree rows, multi-select, delete, and update workflows remain intact
  - Phase 2 follow-up layout pass is implemented:
    - table now dominates the shell width
    - footer is now grouped and readable
    - header metadata layout is tighter and more intentional
  - Phase 3 inspect overlay is implemented:
    - overview no longer uses a persistent right-side detail pane
    - `enter` opens inspect from overview
    - `esc` closes inspect first
    - inspect now renders as an explicit modal overlay
  - Phase 4 task overlays are implemented:
    - `Install`, `Search`, `Update`, `Config`, and `Backup` now render through the overlay host
    - overview no longer mixes modal inspect with side-panel task surfaces
    - `esc` closes idle task overlays back to overview
  - Phase 5 first-pass overlay UX/content cleanup is implemented:
    - task overlays now use clearer grouped sections instead of raw wireframe paragraphs
    - primary actions and current state are more explicit inside each overlay
    - search, install, update, config, and backup overlays are materially more readable
    - overlays are still not at the final production design bar and should get a deeper revamp later
  - search overlay compose-first revamp is implemented:
    - `/` opens search directly into query editing
    - source is explicit as `Wago`
    - the dominant search bar replaces the previous pre-search wall of text
    - search results now render in aligned table columns instead of freeform text rows
  - v1-inspired shell visual pass is implemented:
    - ASCII `LEMONUP` header now uses a smooth fruit-style left-to-right gradient instead of striped glyph coloring
    - shell chrome uses a stronger blue/purple border-title palette
    - the first striped logo attempt is intentionally preserved in code as a legacy fallback reference
  - overview table refinement is implemented:
    - overview table now uses `Name`, `Version`, `Author`, `Source`
    - update and attention state now live in the `Version` column instead of a standalone status column
    - selection uses a fixed-width gutter bar plus a persistent selected-row tint
    - current-row focus remains visually distinct from bulk-selected rows
  - overview table sorting is implemented:
    - `1` sorts by `Name`
    - `2` sorts by `Version`
    - `3` sorts by `Author`
    - `4` sorts by `Source`
    - pressing the same number again reverses the active sort direction
    - the active sort column shows `▲/▼` in the header
    - selection is preserved across re-sort and child rows remain attached to parents

## Implemented, Awaiting Manual Verification

- first real Wago search and install UX now exists in the TUI:
  - Search pane is now a Wago search surface instead of a placeholder
  - search is retail-only, stable-only, and runs on explicit `Enter`
  - search results are navigable in-pane and show selected-result details
  - Search pane can install the selected Wago result
  - Install pane now accepts a direct Wago slug or addon URL
  - both install entry points require explicit confirmation before replacing an existing tracked/on-disk addon
  - successful installs resync the dashboard through the normal scan path
- focused Rust coverage now exists for:
  - search state transitions
  - search result install dispatch
  - direct install dispatch
  - pending replace-confirmation routing
  - Wago search result parsing and install inspection
- Wago search plus TUI install UX was manually verified and checkpointed
- first real TukUI provider-backed CLI parity now exists for canonical `ElvUI` and `Tukui`:
  - TukUI install metadata resolves only from `https://api.tukui.org/v1/addons`
  - TukUI live `check` resolves only from the same TukUI API feed
  - TukUI live `update` resolves only from the same TukUI API feed
  - no GitHub fallback or inferred remote metadata is used for those packages
  - `install-tukui` supports only canonical `ElvUI` and `Tukui` targets in this slice
  - ElvUI managed ownership matches v1:
    - `ElvUI_Libraries`
    - `ElvUI_Options`
  - Tukui remains a managed single-folder package
- TukUI provider parity was manually verified and checkpointed
- first real WoWInterface provider-backed CLI parity now exists:
  - `install-wowinterface` accepts WoWInterface addon page URLs and parses the addon id internally
  - WoWInterface details resolve from `https://api.mmoui.com/v3/game/WOW/filedetails/<id>.json`
  - WoWInterface live `check` resolves only from that API and persists refreshed remote metadata
  - WoWInterface live `update` downloads from `UIDownload` and preserves managed ownership metadata
  - tracked WoWInterface source URLs are canonical public addon URLs that round-trip back to addon ids
- WoWInterface CLI parity was manually verified and checkpointed
- first real GitHub provider-backed CLI parity now exists:
  - `install-github` accepts canonical GitHub repo URLs only
  - GitHub install resolves the repo default branch dynamically and installs from the zipball at that branch HEAD commit
  - tracked GitHub `check` now resolves remote default-branch HEAD commits live
  - tracked GitHub `update` now reapplies managed folders from the current default-branch HEAD commit
  - tracked GitHub records persist canonical repo URL identity plus full commit metadata in `git_commit` and `remote_version`
- GitHub CLI parity was manually verified and checkpointed
- deterministic manual seed/smoke harness now exists:
  - `rust/scripts/manual-test-matrix.toml` is the single curated provider matrix
  - `rust/scripts/seed-sandbox.ps1` resets and seeds a safe WoW-like sandbox root
  - `rust/scripts/smoke-provider-matrix.ps1` runs repeatable provider-backed CLI smoke checks
  - `rust/docs/manual-testing.md` is the source of truth for how and when to use those scripts
  - use this harness before broad ad hoc TUI verification so manual coverage stays repeatable
  - seeded `manual-smoke` validation was manually verified end to end, including TUI check/update/delete/undo flows
- first real config/backup UX now exists in the TUI:
  - Config pane now edits a curated field set:
    - `wago_api_key`
    - `backup_wtf`
    - `backup_retention`
    - `theme`
    - `show_libs`
    - `default_screen`
  - config writes now use a safe read-modify-write path instead of onboarding-only save logic
  - Backup pane can now create a real WTF backup on demand
  - backup history is listed from a profile-scoped app-managed backup directory
  - backup retention is enforced after successful backup creation
  - restore remains intentionally deferred to a later slice
## Next Up

Next phase:

1. manually verify config save + backup-now flows on `manual-smoke`
2. checkpoint richer config/backup workflows
3. production TUI design/build phase
4. backup restore workflows
5. any remaining source-parity cleanup after those slices are proven

### Goal

- stop relying on scan heuristics for important parent-child truth in managed flows
- extend the proven Wago install, live-check, and update contract into broader managed update/install behavior
- let shell bulk actions operate safely on that model

### Planned order

1. continue filling remaining config/backup gaps
2. run the dedicated production TUI design/build phase
3. add backup restore workflows
4. clean up any remaining source-parity or workflow gaps after those slices land

## Remaining Major Work

Still missing or incomplete:

- richer tree view for parent/child relationships
- broader install flows beyond Wago, canonical TukUI, WoWInterface, and GitHub
- remote-backed update checks beyond Wago, canonical TukUI, WoWInterface, and GitHub
- backup restore workflows
- production TUI design/build phase

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
- current search overlay checkpoint:
  - compose-first `/` search retained as the permanent search entry
  - Wago results sorted by highest downloads first with compact DL labels
  - tracked-only installed detection via exact Wago source URL match
  - tracked results now show `📦 installed` plus inline check/install/success feedback

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

- current behavior uses soft-delete into LemonUp-managed trash with shell undo
- current undo scope:
  - one recent delete batch
  - shell/session driven undo
- future work:
  - retention or purge policy
  - richer trash/history UX

## Testing Rules

Default manual-validation ladder going forward:

1. run the Rust validation ladder:
   - `cargo fmt --all`
   - `cargo clippy -p lemonup-app -- -D warnings`
   - `cargo test -p lemonup-app`
2. seed a fresh sandbox with:
   - `pwsh -File rust/scripts/seed-sandbox.ps1 -SandboxRoot <sandbox-root>`
3. run the repeatable provider smoke pass with:
   - `pwsh -File rust/scripts/smoke-provider-matrix.ps1 -SandboxRoot <sandbox-root>`
4. only then do short targeted TUI verification against the seeded sandbox/profile

Future agents should prefer the scripted seed/smoke path over one-off manual installs whenever they need broad provider coverage quickly.

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
- `rust/docs/production-tui-design-plan.md`


## Latest Checkpoint

- inspect overlay redesigned into a command-palette card
- inspect now resolves child rows to the parent addon with child context
- inspect default view now shows compact source/status/version/folder/author/tracking lines instead of internal scan state
- inspect supports inline `space` select/clear, `c` check, `u` update, and `x` delete actions
- inspect details moved behind collapsible `r`/`d`/`t` sections
- overlay close affordance moved to the top-right of the shared modal shell
- dashboard now uses a floating timed toast pill for transient feedback instead of a layout-shifting event rail
- dashboard footer dock uses core actions only and stays stable while the toast appears/disappears
- inspect and dashboard wording now use player-facing labels like `Included addons`, `Addon info`, and `deselect`
