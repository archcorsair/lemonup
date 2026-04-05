# Acceptance Matrix

This is the current product contract for the Rust workspace now that v1 is retired.

| Capability | Status | Notes |
| --- | --- | --- |
| Onboarding / first-run setup | Done | Stepper wizard with safe addon-dir validation |
| Install from GitHub URL | Done | CLI + tracked update/check |
| Install from TukUI / ElvUI | Done | Canonical TukUI coverage |
| Install from WoWInterface | Done | CLI + tracked update/check |
| Search + install from Wago | Done | Unified install/search overlay plus CLI |
| Scan installed addons | Done | Retail scan plus reconcile |
| Update one / selected / all addons | Done | CLI + TUI overview actions |
| Remove addon | Done | Soft-delete plus undo |
| Owned-folder / subfolder tracking | Done | Managed ownership persisted in state |
| Dependency metadata | Done | Inspect overlay and core record support |
| Import / export addon list | Done | Portable JSON format, CLI + TUI entry path |
| Backup WTF | Done | Create + retention in CLI/TUI |
| Restore WTF backup | Done | CLI + TUI restore with rollback |
| Auto-check / background status | Done | Background freshness-aware dashboard refresh |
| CLI updater | Done | `check`, `update`, `update-all`, import/export, restore |
| WeakAuras | Out of scope | Separate product work |
| CurseForge | Out of scope | Not supported |

## Release Gates

- Rust is the only supported runtime in this repo.
- CI, release artifacts, and hooks run against Cargo only.
- Manual smoke coverage exists for seeded provider flows.
- The legacy Bun/Ink app is archived in `docs/v1-archive.md` and no longer lives in-tree.
