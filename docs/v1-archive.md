# v1 Archive

The original LemonUp application was a Bun + Ink codebase at the repo root. It was retired on 2026-04-05 after the Rust workspace closed the required parity blockers and replaced root CI, release, and docs.

Removed from the runnable tree:
- `src/`
- `tests/`
- `index.tsx`
- `package.json`
- `bun.lock`
- Bun/TS-only config and sample files

Keep this note only for historical context. If legacy behavior must be studied, use git history rather than reviving v1 files in-tree.

## Migrating from v1

v1 kept its state in `~/.config/lemonup/` on every platform (on Windows, `%USERPROFILE%\.config\lemonup\`): `config.json` and the SQLite database `lemonup.db`. v2 uses different locations, so on first start v2 migrates automatically (`crates/lemonup-core/src/v1_migration.rs`).

When it runs:
- only for the default profile (no `--profile`)
- only when no v2 config file exists yet and v1's `config.json` exists
- for every command, TUI or CLI; once a v2 config exists it never runs again
- deleting the v2 `config.toml` while the v1 files are still there makes the migration run again on the next start

What is migrated:
- settings: AddOns path (`destDir`, only if it still exists and validates as a `_retail_` or `_forever_` `Interface/AddOns` folder), Wago API key, theme, auto-check on/off and interval, check freshness interval, WTF backup on/off, backup retention, show libraries. Intervals are converted from milliseconds to seconds and values are clamped to v2's ranges.
- tracked addons from `lemonup.db`, only when the AddOns path was migrated and only for addons whose folder still exists there: name, folder, owned folders (those still on disk), source (GitHub, TukUI, WoWInterface, Wago or manual), source URL, version, git commit, remote version, kind and kind override, dependencies, install/update/check timestamps. The game flavor comes from the migrated AddOns path (retail or forever), not from v1's stored value. A v1 row for a folder that another addon lists as one of its owned folders is folded into that parent rather than tracked separately. Addons and folders already tracked in v2 state are left as they are.

What is not migrated:
- addons whose folder is missing, or whose stored folder name is not a plain folder name (they are listed as skipped)
- all addons, when the AddOns path is unset or no longer valid; onboarding then asks for the path
- `maxConcurrent`, `nerdFonts`, `terminalProgress`, `debug`, `userAgent`, `defaultMenuOption`, `lastGlobalCheck` and the legacy `repositories` list

Nothing is downloaded or installed during migration, and v1 files are never modified, moved or deleted. The database is copied to a temporary folder in v2's cache directory and read from the copy, because opening a WAL database creates sidecar files next to it. On Linux, v2's config folder is the same `~/.config/lemonup` folder, so the only file added there is v2's own `config.toml`. On success, the TUI shows a short summary (addons migrated, skipped and already tracked, v1 files untouched) and CLI commands print it to stderr, with skipped folders listed.

If something goes wrong, LemonUp logs the error, shows a one-line notice and keeps starting normally:
- if `config.json` cannot be read, nothing is written; onboarding runs and the migration tries again on the next start, until a v2 config exists
- if `config.json` is fine but `lemonup.db` cannot be read (for example it is corrupt), the settings are still migrated and the notice says the addons were not imported, with the reason. Because a v2 config now exists, the migration does not run again; use the manual fallback below for the addons.

Manual fallback, if automatic migration did not bring over your addons:
1. In v1, export the addon list (it writes `~/lemonup-addons.json` by default).
2. In v2, configure the AddOns path, then run `lemonup import-addons` (reads `~/lemonup-addons.json`; use `--input <file>` for another path and `--dry-run` to preview). Unlike the automatic migration, this downloads and installs listed addons that are not already present.
