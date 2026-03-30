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
- `scripts/`: Windows-native manual seed/smoke helpers for sandbox testing
- `testdata/`: static manual-test fixtures used by the seed/smoke workflow

## Read Order

Start here before continuing the rewrite:

1. `docs/windows-thread-handoff.md`
2. `docs/progress.md`
3. `docs/acceptance-matrix.md`
4. `docs/known-v1-gaps.md`
5. `docs/single-surface-shell-plan.md`

Read `docs/production-tui-design-plan.md` only when the rewrite explicitly enters the production TUI design/build phase.
Read `docs/manual-testing.md` when you need the seeded sandbox manual-smoke workflow.
The current functional MVP work is ahead of the real production TUI design/build phase; keep that design doc out of normal implementation context until we explicitly switch to it.

For future agents: prefer the scripted manual-test harness before ad hoc provider installs when broad manual coverage is needed.

## Intended commands

```bash
cargo fmt --all
cargo test --workspace
cargo run -p lemonup-app --bin lemonup -- tui
cargo run -p lemonup-app --bin lemonup -- sync
cargo run -p lemonup-app --bin lemonup -- update --dry-run
cargo run -p lemonup-app --bin lemonup -- update WeakAuras --dry-run
cargo run -p lemonup-app --bin lemonup -- update-all --dry-run
cargo run -p lemonup-app --bin lemonup -- install-tukui ElvUI --dry-run
cargo run -p lemonup-app --bin lemonup -- install-github https://github.com/WeakAuras/WeakAuras2 --dry-run
pwsh -File scripts/seed-sandbox.ps1 -SandboxRoot D:\Sandbox\WoWDev
pwsh -File scripts/smoke-provider-matrix.ps1 -SandboxRoot D:\Sandbox\WoWDev
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

## Current Config / Backup Slice

- `Config` pane:
  - curated editable settings only
  - explicit save/reset workflow
- `Backup` pane:
  - real WTF backup-now action
  - profile-scoped backup history
  - retention pruning after successful backup creation
  - restore deferred to a later slice

## Current Production TUI Checkpoint

- production TUI design/build is now active
- current verified checkpoint includes:
  - Phase 1 shell foundation
  - Phase 2 dense addon table
  - Phase 2 layout/header/footer rebalance
  - Phase 3 inspect overlay
  - Phase 4 task overlays
  - Phase 5 first-pass overlay UX/content cleanup
  - v1-inspired shell visual/header pass
  - overview table refinement against v1 scanability feedback
- current shell shape:
  - overview uses a full-width addon table
  - overview table now uses `Name`, `Version`, `Author`, `Source`
  - update and attention state are folded into the `Version` column
  - selected rows use a fixed-width gutter bar plus a persistent tint
  - `enter` opens inspect as a modal overlay
  - `Install`, `Search`, `Update`, `Config`, and `Backup` now open through the overlay host
  - the header uses a smooth fruit-style gradient logo treatment inspired by v1
  - current gap is a deeper overlay revamp/polish pass, not overlay routing

## Current Onboarding Checkpoint

- onboarding is now a dedicated fullscreen stepper wizard
- current implemented steps:
  - Theme
  - Directory
  - Wago
  - Settings
  - Review
- the Directory step reuses the validated Rust location-finder logic:
  - quick-check
  - manual path edit
  - deep scan with progress and cancel
- config `Run onboarding again` re-enters the wizard with current values prefilled
