This file provides guidance to Codex when working in this repository.

## Repo Truth

This repo is now a single Rust workspace rooted here.

Default assumption:
- all new product work targets Rust
- the legacy TypeScript/Bun app has been retired from the runnable tree
- historical v1 context lives only in `docs/v1-archive.md` and git history

## Rust Continuations

For LemonUp continuation work, start here in this order:
1. `docs/windows-thread-handoff.md`
2. `docs/progress.md`
3. `README.md`

Use the docs as the source of truth for:
- current branch/status
- completed slices
- next phase
- safety rules
- manual verification status

## Product Context

If product context is needed, read only:
- `conductor/product.md`
- `conductor/product-guidelines.md`

Ignore everything else under `conductor/` unless explicitly asked.

## Runtime / Tooling

- use native Windows/PowerShell for LemonUp work
- if toolchain binaries are missing, prefer `mise which <tool>`
- if `cargo` still is not usable, follow the fallback in `docs/windows-thread-handoff.md`

## Safety / Workflow

- keep changes minimal and surgical
- commit only after user verification when working in the Rust rewrite flow
- when the user says `checkpoint`, update relevant docs under `docs/` before commit

## Legacy v1

Only use `docs/v1-archive.md` or git history if the user explicitly asks about legacy behavior or migration history.
