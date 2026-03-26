# Production TUI Design Plan

## Purpose

This is the dedicated design/build plan for the real production LemonUp TUI.

It is not a polish checklist.

The current TUI is a functional wireframe for proving workflows. This doc exists for the later phase where that wireframe is replaced with a deliberate information architecture, branded visual system, and production interaction model.

Do not front-load this doc into normal feature slices.

Use it when the rewrite is actively working through the production TUI design/build phases.

## Locked Direction

- keep the single-surface shell product model
- move from persistent split-pane prose UI to a list-first shell with overlays
- make the main surface a dense compact table
- keep branding moderate, with the v1 ASCII `LEMONUP` logo explicitly carried forward
- use tasteful microinteractions, not decorative animation
- keep the main addon table visually stable with only minimal motion

## Design Contract

### Primary shell

- one dominant addon table is the default operational surface
- the table stays visible as the main frame of reference during normal use
- the footer remains persistent and command-focused
- the header remains branded, but slim and operational

### Main table

The primary addon surface should be a compact table, not verbose prose rows.

Expected columns/signals:
- addon name
- source
- installed version / remote state
- relationship state
- drift / actionability markers

The table should optimize for scanning many addons quickly.

### Overlay model

Replace the persistent right-side detail pane with overlays or drawers for focused tasks:
- addon inspection
- search and install
- config editing
- backup history/actions
- confirmations and alerts

Overlays must preserve list context and restore logical selection cleanly on close.

### Branding

Carry forward the v1 ASCII `LEMONUP` branding/logo into v2.

Requirements:
- use it intentionally in the header/title treatment
- provide a compact fallback for narrow widths
- keep it readable and iconic
- do not let it crowd operational data

### Visual style

- terminal-native, dense, and operational first
- moderate brand presence
- restrained accent palette
- hierarchy from spacing, alignment, contrast, and badges
- replace verbose temporary text output with concise production copy

### Motion

Animation is supporting feedback, not decoration.

Allowed motion:
- one-shot header/logo reveal on startup
- loading spinners or shimmers for scan/search/install/update/backup work
- subtle row-selection pulse
- brief badge emphasis for update/error/drift state changes
- short overlay open/close transitions
- brief success/error status emphasis

Not allowed:
- constant idle logo animation
- animated table reflow
- bouncing cursors
- decorative background motion
- anything that obscures operational data

## Phased Build Plan

### Phase 1 — shell foundation

Goal:
- extract shell chrome from the monolithic render path without changing workflow behavior

Implement:
- shell layout helpers for header/body/footer
- theme and motion tokens
- full + compact ASCII `LEMONUP` header variants
- overlay host/controller scaffold
- centralized status/copy formatting helpers
- motion primitives for spinners, pulses, and transient feedback timing

Acceptance:
- current flows still work
- header/footer/layout are no longer hardcoded inline in a single render branch
- ASCII logo variants render
- overlay host exists structurally
- animation primitives exist and are safe, but restrained

### Phase 2 — dense addon table

Goal:
- replace the current list-first dashboard with the compact operational table

Implement:
- canonical columns for name, source, version/remote state, relationship state, drift/actionability
- preserve current tree, multi-select, delete, and update behaviors
- keep the old detail area temporarily while table behavior settles

Acceptance:
- table is faster to scan than the current list
- parent/child behavior still works
- common widths remain readable

### Phase 3 — inspect overlay

Goal:
- replace the persistent verbose detail pane

Implement:
- addon inspect overlay for metadata, relationship detail, drift detail, and action summary
- route inspect behavior through the overlay
- remove the persistent right-side detail pane after parity

Acceptance:
- list context is preserved while inspecting
- `q` and `esc` never get trapped
- no persistent paragraph-heavy detail pane remains

### Phase 4 — task overlays

Goal:
- move task-specific modes onto overlays while keeping the list as the main shell

Implement:
- search/install overlay
- config overlay
- backup overlay
- confirm/alert overlays
- footer legend becomes context-aware

Acceptance:
- task flows work through overlays
- destructive and replace flows are explicit and consistent
- provider logic remains unchanged underneath

### Phase 5 — production copy and interaction cleanup

Goal:
- remove wireframe verbosity and make the shell feel deliberate

Implement:
- concise badge and status vocabulary
- compact footer grouping
- better empty/loading/error states
- final microinteraction timing pass

Acceptance:
- operational data dominates the screen
- branding is visible but restrained
- repeated use feels crisp, not noisy

### Phase 6 — onboarding adaptation and responsive finish

Goal:
- bring setup into the same production shell language after the dashboard is proven

Implement:
- adapt onboarding/location finder to the new shell and overlay model
- finalize narrow-width behavior and compact logo handling

Acceptance:
- onboarding no longer feels like a separate app
- narrow terminals degrade intentionally, not accidentally

## Phase Deliverables

When this phase starts, the implementation should define:
- final shell regions
- canonical table columns
- overlay taxonomy
- keybinding ownership between base shell and overlays
- status badge vocabulary
- responsive behavior for narrow terminal widths
- ASCII logo/header variants
- motion tokens and approved microinteraction surfaces

## Acceptance Criteria

The design/build phase is done when:
- the v1 ASCII `LEMONUP` logo is integrated and readable at supported widths
- the main addon workflow is clearly faster than the current prose-detail wireframe
- overlays do not trap `q`, `esc`, or other global navigation unexpectedly
- selection survives overlay open/close cleanly
- the table remains usable at common terminal sizes
- provider-backed flows remain clear without paragraph-heavy detail text
- config, backup, search, install, and update remain discoverable from the shell

## Status

- active implementation context
- current checkpoint state:
  - Phase 1 implemented
  - Phase 2 implemented
  - Phase 2 layout follow-up implemented
  - Phase 3 implemented
  - Phase 4 is next
