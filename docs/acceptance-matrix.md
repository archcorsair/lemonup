# Acceptance Matrix

This is the current product contract for the Rust workspace now that v1 is retired.

| Capability | Status | Notes |
| --- | --- | --- |
| Onboarding / first-run setup | Done | Stepper wizard with safe addon-dir validation |
| Install from GitHub URL | Done | CLI + tracked update/check |
| Install from TukUI / ElvUI | Done | Canonical TukUI coverage |
| Install from WoWInterface | Done | CLI + tracked update/check |
| Search + install from Wago | Done | Unified install/search overlay plus CLI |
| Scan installed addons | Done | Retail and WoW Forever scan plus reconcile |
| Update one / selected / all addons | Done | CLI + TUI overview actions |
| Remove addon | Done | Soft-delete plus undo |
| Owned-folder / subfolder tracking | Done | Managed ownership persisted in state |
| Dependency metadata | Done | Inspect overlay and core record support |
| Import / export addon list | Done | Portable JSON format, CLI + TUI entry path |
| Backup WTF | Done | Create + retention in the TUI backup overlay (no CLI create command) |
| Restore WTF backup | Done | CLI + TUI restore with rollback |
| Auto-check / background status | Done | Background freshness-aware dashboard refresh |
| CLI updater | Done | `check`, `update`, `update-all`, import/export, restore |
| WoW Forever | Done | `_forever_` AddOns folders: flavor detected from the path, Forever/Camelot then Mainline TOC selection, Wago `game_version=forever`, TukUI requires a 1.60.x patch, WoWInterface installs warn; one folder per profile. Not yet verified against a live Forever install |
| WeakAuras | Out of scope | Separate product work |
| CurseForge | Out of scope | Not supported |

## Release Gates

- Rust is the only supported runtime in this repo.
- CI, release artifacts, and hooks run against Cargo only.
- Manual smoke coverage exists for seeded provider flows.
- The legacy Bun/Ink app is archived in `docs/v1-archive.md` and no longer lives in-tree.
