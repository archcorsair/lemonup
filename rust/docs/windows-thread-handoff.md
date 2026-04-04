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
   - `rust/docs/manual-testing.md` when manual verification or seeded sandbox coverage is relevant
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
- latest local milestone at handoff pass: Wago managed-state and version-normalization fixes implemented and user-verified
- current worktree status at handoff pass: checkpoint-ready Wago managed-state/version-display slice pending commit

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
- onboarding stepper wizard
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
- real provider-backed selected updates in the TUI
- Wago search plus TUI install UX is implemented, manually verified, and checkpointed
- TukUI provider parity is implemented, manually verified, and checkpointed
- WoWInterface provider parity is implemented, manually verified, and checkpointed
- GitHub CLI parity is implemented, manually verified, and checkpointed:
  - `install-github` installs from canonical GitHub repo URLs only
  - tracked GitHub `check` now resolves remote default-branch HEAD commits live
  - tracked GitHub `update` now reapplies managed folders from the current default-branch HEAD commit
  - GitHub package parent selection follows Rust-local repo-name heuristics and managed ownership rules
- scripted manual-testing harness now exists and is manually verified:
  - `seed-sandbox.ps1`
  - `smoke-provider-matrix.ps1`
  - `manual-test-matrix.toml`
  - `manual-testing.md`
  - seeded `manual-smoke` profile covers TukUI, GitHub, WoWInterface, Wago, and one manual fixture
- first TUI config/backup slice is implemented and checkpointed:
  - Config pane edits curated settings with explicit save/reset
  - Backup pane creates real WTF backups into a profile-scoped app-managed backup directory
  - backup history and retention are wired
  - restore is intentionally deferred
- production TUI design/build phases 1-3 are implemented and user-verified:
  - Phase 1 shell foundation:
    - extracted shell layout/header/footer helpers
    - full + compact ASCII `LEMONUP` branding
    - motion primitives and overlay host scaffold
  - Phase 2 dense addon table:
    - compact operational table replaced prose-style addon rows
    - table/tree/multi-select/delete/update behaviors remain intact
    - header/footer/layout rebalanced so the table is the dominant surface
  - Phase 3 inspect overlay:
    - overview no longer uses a persistent right-side detail pane
    - `enter` opens inspect from overview
    - `esc` closes inspect first
    - inspect uses an explicit modal-style overlay
- search overlay polish and tracked-Wago install feedback:
  - compose-first `/` search retained
  - aligned results table sorted by highest DL count
  - tracked-only installed detection with `📦 installed` badge
  - inline shimmer during install/checking and green-check success state
- production TUI design/build Phase 4 is implemented and user-verified:
  - `Install`, `Search`, `Update`, `Config`, and `Backup` render through the overlay host
  - overview no longer mixes modal inspect with side-panel task surfaces
  - `esc` closes idle task overlays back to overview
- production TUI design/build Phase 5 first pass is implemented and user-verified:
  - task overlays now use clearer grouped sections and denser production-oriented copy
  - primary actions and current state are more explicit inside each overlay
  - overlays are more readable, but still below the final production design bar
  - expect a later deeper overlay revamp/polish pass
- search overlay compose-first redesign is implemented and user-verified:
  - `/` opens search directly in typing mode
  - source is explicit as `Wago`
  - the dominant search bar replaces the old pre-search wall of text
  - results now render in aligned columns; `?` is no longer used for search and is available for future help
- v1-inspired shell visual pass is implemented and user-verified:
  - ASCII `LEMONUP` header now uses a smooth fruit-style gradient
  - shell border/title chrome uses a stronger production palette
  - the original striped-logo first attempt is intentionally preserved in code as a legacy fallback reference
- production overview table refinement is implemented and user-verified:
  - overview table now uses `Name`, `Version`, `Author`, `Source`
  - update and attention state now live in the `Version` column instead of a standalone status column
  - selection uses a fixed-width gutter bar plus a persistent selected-row tint
  - current-row focus remains visually distinct from bulk-selected rows
- inline overview actions are implemented and user-verified:
  - `c` now performs freshness-aware check directly from overview
  - `u` now performs update directly from overview after stale-target preflight
  - `,` now opens config after reclaiming `c`
  - header/footer job rail plus row-local update feedback are active during inline actions
  - shimmer is used for action text only; the logo remains static
  - adaptive tick now drops to `100ms` only during active animation/job states and otherwise stays at `250ms`
  - false-positive same-version updates from leading `v` prefixes and lost GitHub commit metadata are fixed
- footer control dock is implemented and user-verified:
  - footer is now a fixed two-line split rail rather than a titled status panel
  - left rail stays quiet by default and only shows meaningful progress, results, errors, or explicit selection summaries
  - right rail shows grouped command hints with sticky context emphasis and keypress pulse feedback
  - footer copy is deliberately humanized; internal/debug-style summaries are suppressed
- overview table sorting is implemented and user-verified:
  - 1 sorts by Name
  - 2 sorts by Version
  - 3 sorts by Author
  - 4 sorts by Source
  - pressing the same number again reverses the sort direction for that column
  - the active sort shows ▲/▼ in the table header
  - selection survives re-sort and child rows remain attached to their parents
- Wago tracked-state/version regression fixes are implemented and user-verified:
  - single-folder Wago installs remain managed instead of rendering as unmanaged
  - state merges preserve provider-managed authority even with empty owned-folder lists
  - Wago immediate post-install false updates caused by leading # version labels are normalized away
  - overview version rendering now preserves suffixes so false same-version arrows are easier to avoid and diagnose

## Recent Important Commits

Most relevant recent milestones:
- `3663147` `fix(tests): close addon manager before temp cleanup`
- `7236ff5` `feat(rust): add targeted cli update selectors`
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
Preserve the v1 setup spirit, now through a dedicated fullscreen stepper wizard:
- top-left `LemonUp Setup` header with small lemon icon
- top-right `Step X of 5`
- slim steps:
  - `Theme`
  - `Directory`
  - `Wago`
  - `Settings`
  - `Review`
- directory step keeps the proven location-finder behavior:
  - auto-detect first
  - editable search root
  - deep scan with live progress and cancel
  - success actions:
    - `Use this path`
    - `Enter different path`

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

### `rust/docs/production-tui-design-plan.md`
Keep this out of normal feature context until the project explicitly enters the production TUI design/build phase.

Do not pull it into normal implementation context early.

### `rust/docs/manual-testing.md`
Use this when you need repeatable manual coverage:
- seeded sandbox setup
- provider smoke coverage
- manual fixture coverage
- short TUI spot checks on top of a known-good seeded matrix

Prefer this scripted workflow over one-off manual installs when validating broad provider behavior.

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
- seeded sandbox harness worked end to end:
  - `seed-sandbox.ps1`
  - `smoke-provider-matrix.ps1`
  - multi-provider CLI smoke coverage now exists for TukUI, GitHub, WoWInterface, Wago, and one manual fixture
- seeded TUI verification on `manual-smoke` worked for:
  - checks
  - provider-backed selected updates
  - delete
  - undo delete

## Scripted Manual Testing Workflow

Preferred repeatable validation path:

1. seed a fresh sandbox root with:
   - `pwsh -File rust/scripts/seed-sandbox.ps1 -SandboxRoot <sandbox-root>`
2. run provider smoke coverage with:
   - `pwsh -File rust/scripts/smoke-provider-matrix.ps1 -SandboxRoot <sandbox-root>`
3. use the printed TUI launch command for short visual/interaction verification

Notes for future agents:
- the scripts default to `--profile manual-smoke`, not `dev`
- they intentionally reset the sandbox root
- they are the fastest safe way to get broad source coverage without rebuilding ad hoc test installs
- the target list lives in `rust/scripts/manual-test-matrix.toml`

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
- production TUI design/build phase
- backup restore workflows
- any remaining non-MVP source parity cleanup after those slices land

## Next Recommended Slice

Next real phase:
1. enter the dedicated production TUI design/build phase
2. add backup restore workflows
3. clean up any remaining source-parity or workflow gaps after those slices are proven

Why this is next:
- Wago install/search/check/update is now proven
- canonical TukUI install/check/update is now proven
- WoWInterface install/check/update is now proven
- GitHub install/check/update is now proven
- real provider-backed selected updates in the TUI are now proven
- the new seeded manual-smoke workflow gives broad repeatable coverage for later slices
- the ownership model is already hardened enough to build on
- the visual/design pass is intentionally deferred into its own dedicated doc to preserve progressive disclosure

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

`Continue LemonUp Rust rewrite on codex/rusty-lemon. Repo root is C:\Users\archc\ghq\github.com\archcorsair\lemonup. Read rust/README.md, rust/docs/windows-thread-handoff.md, and rust/docs/progress.md first. Treat the TS/Bun app as feature-parity reference only; do not extend it. Wago CLI/TUI search-install plus TukUI, WoWInterface, GitHub, TUI-selected live updates, and config/backup-now flows are already landed and verified. Use rust/docs/manual-testing.md and the seeded manual-smoke scripts for broad manual coverage before ad hoc provider setup. The next phase is the dedicated production TUI design/build phase described in rust/docs/production-tui-design-plan.md. Respect existing safety/relationship invariants and commit only after my manual verification.`


## Latest Checkpoint

- inspect overlay is now a compact command-palette card instead of a raw metadata dump
- child-row inspect resolves to the parent addon and shows child context inline
- inspect supports inline `space` select/clear, `c` check, `u` update, and `x` delete actions
- modal close affordance now lives in the top-right of the shared overlay shell
