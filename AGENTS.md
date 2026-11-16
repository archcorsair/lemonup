# Contributing to LemonUp

Guidance for anyone changing this repository, human or coding agent. It describes the
architecture and the rules every change has to keep, so LemonUp stays safe for people's addons
and consistent to use.

## Repository

LemonUp is a single Rust workspace. The legacy TypeScript/Bun app is retired and exists only in
git history; all product work targets Rust.

- `crates/lemonup-core`: everything that needs no network. Domain types, `config.toml`, per-profile
  paths, the SQLite state (`storage`), AddOns scanning, WoW install detection and validation
  (`wow`), the profile to AddOns folder binding (`binding`), v1 migration, and addon list
  import and export.
- `crates/lemonup-app`: the `lemonup` binary.
  - `cli.rs`: the CLI commands. `tui.rs`, `app.rs` and `app/`: the TUI.
  - `github.rs`, `wago.rs`, `tukui.rs`, `wowinterface.rs`: one client per addon source.
    `http.rs` builds every HTTP client; `source_error.rs` classifies their failures.
  - `update.rs` and `update_targets/`: planning installs and updates.
    `update_targets/folder_swap.rs` is the only code that replaces addon folders.
  - `backup.rs`: backups of the game's WTF folder. `archive.rs`: zip extraction.
- `scripts`: manual smoke scripts (`*.ps1`) and the website screenshot generator
  (`scripts/screenshots`). `testdata`: fixtures for manual testing.

Start with `README.md`, then `docs/manual-testing.md` for testing against a sandbox.

## Tooling and checks

- The Rust toolchain is pinned in `mise.toml`; CI uses the same version. If a tool is missing,
  try `mise which <tool>`.
- Every change must pass, in this order, the same checks CI runs:
  1. `cargo fmt --all`
  2. `cargo clippy --workspace --all-targets -- -D warnings`
  3. `cargo test --workspace`
- Then check the change in a sandbox as described in `docs/manual-testing.md`. TUI changes need a
  look at 80x24 and at a wide window.
- `scripts/*.ps1` need PowerShell (`pwsh`), on any OS.

## Invariants

### Your addons and settings are never at risk

- Addon folders change only through `FolderSwap`: new folders are staged first, old ones are moved
  aside, and everything is rolled back if any step or the database write fails.
- Never replace a folder that belongs to another linked addon. Compare folder names without case,
  because Windows and macOS ignore it.
- Replacing a folder LemonUp didn't install (a "Not linked" addon) needs the user's explicit yes,
  and the question names every folder it replaces.
- Never touch the WTF folder except through `backup.rs` (backup and restore). Updates back it up
  first when the setting is on.
- A profile is bound to one AddOns folder. Refuse to act on a different folder rather than
  rebinding silently.
- A failed read (scan, metadata, migration) must keep existing state. Never treat "couldn't read"
  as "deleted".

### Network

- Every HTTP client comes from `http::client_builder()`, which sets the connect and read
  timeouts. Don't build clients elsewhere.
- Source failures return a `SourceError` with a `SourceErrorKind`. Code decides what to show from
  the kind, never by matching error text.
- Never log, print or show API keys or tokens; key boxes are masked.

### TUI

- The screen only claims what is true right now. No key hint for a key that does nothing, and no
  success message that wasn't confirmed.
- Every key lives in `app/keymap.rs`; the key line and Help are built from it, and tests check
  that Help matches the real key handling. Add or change keys there.
- Destructive actions ask through the shared question in `app/dialog.rs`, and only `y` confirms.
  Enter never confirms a destructive question. Ctrl+C always quits; `q` goes back, and quits only
  from the addon list.
- Every screen works at 80x24 without clipping.
- Colors come only from the color roles in `theme.rs`, and symbols go through `Symbols`, so the
  Terminal theme, `NO_COLOR` and Simple symbols keep working. No emoji.
- Use the existing words: Update ready, Up to date, Newer than source, Not linked, Not checked
  yet, Check failed, Details, Settings, "backup of your game settings".

### Code and tests

- Keep changes small and focused. No refactors, renames or cleanups a change doesn't need.
- Every behavior change comes with a test that fails without it.
- Don't add dependencies without discussing it first.
- Put logic that needs no network in `lemonup-core`.

## Writing

- American English spelling (color, center) in code, comments, docs and user-facing text.
- No em dashes or en dashes in user-facing text, docs or commit messages.
- Plain, short sentences in user-facing text. Say what happened and what to do next.

## Commits and pull requests

- Conventional commit subjects with a scope where it helps: `feat(tui): ...`, `fix(backup): ...`,
  `docs: ...`, `chore: ...`. The body lists what changed for the user.
- One topic per pull request. Its description says what changed, why, and how it was checked.
- Releases are cut by maintainers: pushing a `v*` tag runs `.github/workflows/release.yml`.
