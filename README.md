# LemonUp

<div align="center">
  <img src="./lemonup.png" alt="LemonUp Logo" width="150" />
  <h3>World of Warcraft Addon Manager</h3>
  <p>Rust-first terminal UI and CLI for installing, checking, updating, backing up, and restoring WoW addons.</p>
</div>

---

[![Test](https://github.com/archcorsair/lemonup/actions/workflows/test.yml/badge.svg?branch=main)](https://github.com/archcorsair/lemonup/actions/workflows/test.yml)

LemonUp is now a single Rust workspace at the repo root. The legacy Bun/Ink app has been retired and archived in `docs/v1-archive.md`.

## Compatibility

| Platform Support               | Content Sources      |
| :----------------------------- | :------------------- |
| ✅ **Retail**                  | ✅ **GitHub**        |
| 📋 **Classic / Era** (Planned) | ✅ **TukUI**         |
| 📋 **Cata** (Planned)          | ✅ **WoWInterface**  |
|                                | ✅ **Wago.io**       |
|                                | 📋 **WeakAuras** (Planned) |

### Supported Architectures

- **Windows:** x64
- **Linux:** x64
- **macOS:** Apple Silicon (arm64), Intel (x64)

## Features

- Terminal-first overview table with inspect, sort, multi-select, bulk update, and bulk delete
- Unified install/search overlay for Wago name search or direct Wago URL install
- Provider-backed install/check/update flows for GitHub, TukUI, WoWInterface, and Wago
- Import and export tracked addon lists using a portable JSON format
- Background auto-check using the same freshness policy as manual checks
- WTF backup and restore flows in both CLI and TUI
- Seeded manual-smoke scripts for safe sandbox validation

## Workspace Layout

- `crates/lemonup-core`: domain model, config/state storage, transfer format
- `crates/lemonup-app`: ratatui shell and CLI entrypoint
- `docs/`: handoff, progress, acceptance, manual testing, archive notes
- `scripts/`: PowerShell helpers for local runs and manual smoke
- `testdata/`: manual-smoke fixtures

## Start Here

1. `docs/windows-thread-handoff.md`
2. `docs/progress.md`
3. `docs/acceptance-matrix.md`
4. `docs/manual-testing.md`

## Common Commands

```powershell
cargo fmt --all
cargo clippy -p lemonup-app -- -D warnings
cargo test --workspace
cargo run -p lemonup-app --bin lemonup -- --profile dev tui
cargo run -p lemonup-app --bin lemonup -- export-addons
cargo run -p lemonup-app --bin lemonup -- import-addons --dry-run
cargo run -p lemonup-app --bin lemonup -- restore-backup
pwsh -File scripts/seed-sandbox.ps1 -SandboxRoot D:/Sandbox/WoWDev
pwsh -File scripts/smoke-provider-matrix.ps1 -SandboxRoot D:/Sandbox/WoWDev
```

## Releases

GitHub releases are built from the Rust workspace only. Download the latest binaries from [Releases](https://github.com/archcorsair/lemonup/releases).

## License

MIT © [ArchCorsair](https://github.com/archcorsair)
