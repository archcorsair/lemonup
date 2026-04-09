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

- `C:/Users/archc/ghq/github.com/archcorsair/lemonup`

## Workspace Layout

- `crates/lemonup-core`
- `crates/lemonup-app`
- `docs`
- `scripts`
- `testdata`

## Current Status

- default branch: `main`
- latest verified milestone: Rust-only cutover complete, v1 retired, blocker slice closed

## Current Product State

Implemented and validated:
- onboarding and addon-dir safety rails
- provider-backed install/check/update for GitHub, TukUI, WoWInterface, and Wago
- unified install/search overlay for name search and direct Wago URL install
- overview sort, inspect, multi-select, bulk update/delete, soft-delete undo
- contextual `?` help overlay for dashboard, inspect, install/search, config, and backup
- import/export addon list
- background auto-check / dashboard status
- WTF backup and restore
- Rust-only CI, release workflow, and repo root layout
- manual-smoke seed/reset no longer wipes saved profile config; failed Wago preflight should now surface the real credential error

## Validation Workflow

Use this ladder by default:

1. `cargo fmt --all`
2. `cargo clippy -p lemonup-app -- -D warnings`
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
& $cargo "+$toolchain" clippy -p lemonup-app -- -D warnings
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

- product polish and overlay/layout tightening
- future source/provider work only if product scope expands
- keep docs current when checkpointing

## Historical Design Context

- the old in-repo design-plan docs were removed after implementation landed
- use git history if you explicitly need the earlier planning artifacts
