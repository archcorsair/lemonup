# Manual Testing Workflow

Use this when you want a fast, repeatable sandbox for Rust v2 manual checks without touching a production AddOns path.

## Defaults

- canonical scripts live under `rust/scripts`
- recommended profile is `manual-smoke`
- sandbox root is a WoW-like root, not a raw `AddOns` path
- CLI automation seeds and checks the sandbox; TUI remains a short manual pass

## Seed

This resets the sandbox root, rebuilds the WoW validator artifacts, clears the dedicated smoke profile state, installs the curated provider matrix, copies the manual fixture, and runs `sync`.

PowerShell:

```powershell
pwsh -File rust/scripts/seed-sandbox.ps1 -SandboxRoot D:\Sandbox\WoWDev
```

Nushell:

```nu
pwsh -File rust/scripts/seed-sandbox.ps1 -SandboxRoot D:\Sandbox\WoWDev
```

Notes:

- default profile is `manual-smoke`
- pass `-Profile dev` only if you intentionally want to reuse the existing dev state DB
- Wago targets require a configured `WAGO_API_KEY`; the seed script fails before partial installs if that prerequisite is missing

## Smoke

This re-syncs the sandbox state, then runs:

- targeted `check` for every seeded target
- targeted `update <selector> --dry-run` for every seeded target
- aggregate `update --dry-run`
- aggregate `update-all --dry-run`
- mixed-provider targeted `update <selectors...> --dry-run`

PowerShell:

```powershell
pwsh -File rust/scripts/smoke-provider-matrix.ps1 -SandboxRoot D:\Sandbox\WoWDev
```

Nushell:

```nu
pwsh -File rust/scripts/smoke-provider-matrix.ps1 -SandboxRoot D:\Sandbox\WoWDev
```

The scripts exit non-zero on the first failed prerequisite or contract mismatch.

## Matrix

The seeded target list lives in:

- `rust/scripts/manual-test-matrix.toml`

Current curated targets:

- TukUI: `ElvUI`
- GitHub: `WeakAuras`
- WoWInterface: `TomTom`
- Wago: `BigWigs`
- manual fixture: `ManualSmokeAddon`

## TUI Pass

After a successful seed and smoke pass, run:

```powershell
pwsh -Command "& 'rust\target\debug\lemonup.exe' --profile manual-smoke --addon-dir 'D:\Sandbox\WoWDev\_retail_\Interface\AddOns' tui"
```

Quick checklist:

- run one provider-backed selected update with `r`
- verify one manual addon reports a skip path cleanly
- confirm selection remains stable
- confirm list/detail text stays readable
