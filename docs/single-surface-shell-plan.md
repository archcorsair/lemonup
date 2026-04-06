# Single-Surface Shell Plan

## Purpose

Freeze the next Rust UI direction before deeper implementation.

This is the working contract for the current rewrite branch. It preserves the spirit of LemonUp v1 without copying the old React/Ink layout literally.

## Product Direction

LemonUp v2 should be a single persistent terminal surface.

The core interaction model is:
- left: addon list
- right: contextual detail and actions
- top: profile, path, scan status
- bottom: key hints and transient status

Do not reintroduce a multi-screen mental model as the primary UX.

## MVP Requirements

Minimum product scope should cover:

- install addon from supported sources
- update one addon
- update all addons
- update selected addons
- delete one addon
- delete selected addons
- search addons from Wago
- view addon metadata and details
- maintain parent-child ownership correctly during install, update, delete, and rescan

### MVP-adjacent support behavior

These are not optional if the core actions above are to behave safely:

- multi-select in the main list
- bulk actions on selection
- clear progress and result status for install, update, and delete
- destructive-action confirmation
- update-check status visibility
- rescan or reconcile command inside the shell
- source and state indicators

### Drift detection requirements

The app should eventually detect and surface both directions of drift:

- folder exists on disk with no tracked record
- tracked record exists with no folder on disk
- parent record exists but an owned child folder is missing on disk
- child folder exists on disk but parent or ownership record is missing
- tracked ownership does not match actual disk contents
- tracked source metadata and disk state disagree in a meaningful way

## v1 Spirit To Preserve

The old app got several things right. Preserve these qualities in the Rust TUI:

- data-dense first, not dashboard-first
- one dominant operational list/table surface
- low chrome, thin borders, terminal-native feel
- strong LemonUp branding in the header, but data owns the screen
- high-signal color, not decorative color
- persistent footer legend for commands and status
- hierarchy from alignment, spacing, and color rather than many nested boxes

### Visual direction

Use the v1 screenshot as aesthetic guidance, not as a literal layout spec.

Preserve this overall look:
- deep navy or indigo base
- warm LemonUp brand accent in the title area
- cyan for affordances and focus hints
- green for healthy or up-to-date status
- amber and purple for source badges or metadata accents

Avoid:
- separate routed screens
- oversized empty panels
- generic dashboard styling
- porting old component structure into Ratatui concepts

## Current Slice Plan

### Summary

Shift the Rust app from multi-screen navigation to one single-surface TUI.

This slice builds:
- the single-surface shell foundation
- inline onboarding takeover inside that shell
- real addon scan plus SQLite reconcile
- real scanned addon rows in the primary list
- visible but mostly stubbed action areas for install, update, search, config, and backup

### Defaults locked

- single-screen interaction model is `list + detail`
- onboarding stays inside the same shell
- full DB reconcile runs on scan
- owned-folder inference is conservative only
- retail remains the active flavor for this slice
- parent addon rows only are rendered in the main list for now

## Shell Contract

### Header

Show:
- active profile
- target AddOns path
- scan status

### Main area

Left pane:
- parent addon rows only
- sorted by addon name ascending
- `j` and `k` navigation

Right pane:
- selected addon metadata
- contextual action mode
- explicit "not wired yet" states where features are placeholders

### Footer

Show:
- key hints
- transient status messages
- no numbered screen routing hints

## Onboarding Contract

If addon dir is missing or invalid, the shell remains mounted and the main content becomes the Location Finder takeover.

Preserve the existing onboarding UX contract:
- auto-detect first
- editable search root
- deep scan with progress and cancel
- success actions:
  - `Use this path`
  - `Scan another location`
  - `Edit path manually`

After a path is saved:
- persist config
- transition in-place to addon management
- trigger scan and sync automatically

## Scan And Reconcile Contract

Add scan support in `lemonup-core` for direct children of `Interface/AddOns` only.

A top-level folder qualifies as an addon candidate only if it contains one or more `.toc` files at the folder root.

### TOC rules

Support retail TOC selection priority:
- `Folder-Retail.toc`
- `Folder_Mainline.toc`
- `Folder-Mainline.toc`
- `Folder.toc`
- else first alphabetical `.toc` as ambiguous fallback

Parse:
- `Title` with WoW color-code stripping
- `Version`
- `Author`
- `Interface`
- `Dependencies`
- `RequiredDeps`
- `OptionalDeps`
- `X-Library`

### Embedded library detection

Detect embedded libraries under:
- `Libs`
- `libs`
- `Lib`
- `lib`
- `Libraries`

A child counts as an embedded library only if that child folder contains a `.toc` file.

### Conservative owned-folder inference

Collapse only when an exact base folder exists.

Allowed examples:
- `Details` + `Details_DataStorage` -> parent owns child
- `ElvUI` + `ElvUI_Options` -> parent owns child
- `ElvUI` + `ElvUI_Libraries` -> parent owns child

Disallowed examples:
- `DBM-Core` + `DBM-Naxx` stay separate
- dependency name similarity alone does not collapse folders
- no exact base folder means no collapse

### Library classification

Classify as library only from strong local evidence:
- `X-Library: true`
- known library naming patterns such as `Lib*`, `Ace*`, `*-1.0`, `CallbackHandler*`

### Git detection

If a folder contains `.git`, capture the current commit hash as disk metadata.

## SQLite Reconcile Contract

No schema migration in this slice.

Use transactional reconcile against the current `addons` table.

After each scan:
- upsert scanned parent rows
- delete stale rows for folders missing on disk
- delete child rows now represented by a parent's `owned_folders`
- preserve `installed_at`
- preserve `kind` when `kind_override` is true
- preserve existing tracked source metadata unless a later install or update flow changes it

Keep one visible row per parent addon after reconcile.

## App Wiring Contract

### On app boot

- valid addon dir -> start background scan -> populate shell
- missing or invalid addon dir -> show onboarding takeover in the same shell

### After onboarding save

- persist config
- start background scan
- reload DB-backed addon rows into the shell

### Detail modes

Keep visible action modes inside the right pane:
- overview
- install
- search
- update
- config
- backup

These are integrated feature areas, not separate routed screens.

## Relationship Integrity Contract

These are hard invariants for future work:

- parent update must preserve child ownership links
- parent delete must remove owned children too
- managed child folders must not split into standalone addons after update
- scan-only inference must stay conservative
- install or update metadata should become the authoritative ownership source where possible
- flat parent view and optional expanded tree view must both be supported

## Testing Contract

Use the dev profile only for manual app testing.

Do not test against a live production AddOns folder.

Preferred manual run command:

```powershell
cargo run -p lemonup-app --bin lemonup -- --profile dev tui
```

If an explicit sandbox addon directory is needed, pass a dev-safe path via `--addon-dir`.

## Status On This Branch

Already implemented and user-verified:
- addon scan plus state reconcile core
- single-surface shell foundation
- background scan plus DB-backed shell dashboard
- UTF-8-safe TOC parsing for localized metadata
- relationship-safe reconcile rules for managed parents
- tree-ready owned-child expansion and collapse in the shell

Next implementation chunk:
- authoritative install and update ownership model
- multi-select and bulk action groundwork
- update or delete behavior that preserves ownership links end-to-end
- drift detection surfaced in the shell
- richer tree rendering beyond owned-child expansion

## Related Docs

Read alongside:
- `docs/windows-thread-handoff.md`
- `docs/acceptance-matrix.md`
- `docs/v1-archive.md`
