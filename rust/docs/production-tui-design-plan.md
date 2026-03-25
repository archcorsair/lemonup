# Production TUI Design Plan

## Purpose

This is the dedicated design/build plan for the real production LemonUp TUI.

It is not a polish checklist.

The current TUI is a functional wireframe for proving workflows. This doc exists for the later phase where that wireframe is replaced with a deliberate information architecture, branded visual system, and production interaction model.

Do not front-load this doc into normal feature slices.

Use it only when the rewrite explicitly enters the production TUI design/build phase.

## Locked Direction

- keep the single-surface shell product model
- move from persistent split-pane prose UI to a list-first shell with overlays
- make the main surface a dense compact table
- keep branding moderate, with the v1 ASCII `LEMONUP` logo explicitly carried forward

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

## Phase Deliverables

When this phase starts, the implementation should define:
- final shell regions
- canonical table columns
- overlay taxonomy
- keybinding ownership between base shell and overlays
- status badge vocabulary
- responsive behavior for narrow terminal widths
- ASCII logo/header variants

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

- intentional planning doc
- not active implementation context until the project explicitly enters this phase

