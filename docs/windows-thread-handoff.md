# LemonUp Rust Handoff

## Start Here

This repo is now a single Rust workspace rooted here. The legacy Bun/Ink app has been retired.

Read in this order:
1. `README.md`
2. `docs/progress.md`
3. `docs/acceptance-matrix.md`
4. `docs/manual-testing.md`
5. `docs/v1-archive.md` only if historical context is explicitly needed

## Repo Root

- Windows (primary): `C:/Users/archc/ghq/github.com/archcorsair/lemonup`
- Other machines: `~/ghq/github.com/archcorsair/lemonup` (ghq layout)

## Workspace Layout

- `crates/lemonup-core`
- `crates/lemonup-app`
- `docs`
- `scripts`
- `testdata`

## Current Status

- default branch: `main`
- current branch: `feat/2.0-release-blockers`
- latest milestone: the 2.0 TUI redesign is complete (slices 1-20, one commit per wave, through `130d011`); each wave was smoke tested by the owner in a sandbox; `cargo test --workspace` passes (535 app, 100 core)
- slice 21 (find all my addons in one go) is deferred until after 2.0
- no 2.0 release published yet; see Before Release in `docs/progress.md`

## Current Product State

Implemented and validated:
- onboarding and addon-dir safety rails
- provider-backed install/check/update for GitHub, TukUI, WoWInterface, and Wago
- dashboard default sort now starts on Version and prioritizes update-available rows, then recent update/install activity
- unified install/search overlay for name search and direct Wago URL install
- overview sort, inspect, multi-select, bulk update/delete, soft-delete undo
- sheets for Details, Install, Settings, Backups, and Help; Help lists every working key per screen
- Backups sheet with create (`b`), restore, and delete flows and a short backup spam guard
- import/export addon list
- background auto-check / dashboard status
- WTF backup and restore
- Rust-only CI, release workflow, and repo root layout
- manual-smoke seed/reset no longer wipes saved profile config; failed Wago preflight should now surface the real credential error

## Validation Workflow

Use this ladder by default:

1. `cargo fmt --all`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. seeded manual smoke from `docs/manual-testing.md`

## Manual Smoke

Preferred safe path:

1. `pwsh -File scripts/seed-sandbox.ps1 -SandboxRoot D:/Sandbox/WoWDev`
2. `pwsh -File scripts/smoke-provider-matrix.ps1 -SandboxRoot D:/Sandbox/WoWDev`
3. `pwsh -Command "& 'target/debug/lemonup.exe' --profile manual-smoke --addon-dir 'D:/Sandbox/WoWDev/_retail_/Interface/AddOns' tui"`

## Windows Toolchain Fallback

If `cargo` is not on PATH or rustup has no default toolchain, use:

```powershell
$cargo = (Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe')
$toolchain = Get-ChildItem (Join-Path $env:USERPROFILE '.rustup/toolchains') -Directory | Sort-Object Name -Descending | Select-Object -First 1 -ExpandProperty Name
& $cargo "+$toolchain" fmt --all
& $cargo "+$toolchain" clippy --workspace --all-targets -- -D warnings
& $cargo "+$toolchain" test --workspace
```

## First Commands In A New Thread

```powershell
cd C:/Users/archc/ghq/github.com/archcorsair/lemonup
git status
git log --oneline -n 10
mise which cargo
```

## Recommended Next Work

Before release, in order (each step needs the owner's approval):

1. website screenshots and pages (in progress; another thread owns `scripts/screenshots`)
2. full smoke run in a sandbox
3. push the branch and open the PR
4. tag `v2.0.0`
5. update the Homebrew formula (still says version "0.0.8") and the Scoop manifest
6. deploy the website

After 2.0: slice 21, find all my addons in one go. Keep docs current when checkpointing.

## Safety Rules

- never let a Setup search start above the sandbox HOME: a search from the parent of a fake HOME (or from the real home folder, or "Search the whole computer") finds the real WoW install
- use `--profile manual-smoke` and a sandbox AddOns folder for every manual run
- in the TUI, `q` goes back in sheets and quits only from the list
- Ctrl+C quits from every screen and asks first during an update
- only `y` confirms a delete or restore; Enter does not

## Historical Design Context

- the old in-repo design-plan docs were removed after implementation landed
- use git history if you explicitly need the earlier planning artifacts
