# v1 -> v2 Acceptance Matrix

This freezes the current product contract before deeper Rust implementation.

| Capability | v1 status | v2 target |
| --- | --- | --- |
| Onboarding / first-run setup | Implemented in Ink wizard | Required before v2 beta |
| Install from GitHub URL | Implemented | Required |
| Install from TukUI / ElvUI | Implemented | Required |
| Install from WoWInterface | Implemented | Required |
| Search + install from Wago | Implemented | Required |
| Scan installed addons | Implemented | Required |
| Update all addons | Implemented | Required |
| Remove addon | Implemented | Required |
| Owned-folder / subfolder tracking | Implemented | Required |
| Dependency metadata | Implemented | Required |
| Import / export addon list | Implemented | Required |
| Backup WTF | Implemented | Required |
| Auto-check / background status | Implemented | Required |
| CLI updater | Basic only | Required, still basic |
| WeakAuras | Planned only | Out of scope |
| CurseForge | Not supported | Out of scope |

## v2 release gates

- Core domain/state layer can represent every current addon/source type.
- TUI can reach onboarding, install, manage, config, and Wago search screens.
- CLI can run a non-interactive update path with `--force` and `--dry-run`.
- Cross-platform config/data/cache paths are OS-native.
- Terminal state is restored on normal exit, error, and ctrl-c.
