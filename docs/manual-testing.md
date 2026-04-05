# Manual Testing Workflow

Use this when you want a fast, repeatable sandbox for LemonUp manual checks without touching a production AddOns path.

## Defaults

- canonical scripts live under `scripts`
- recommended profile is `manual-smoke`
- sandbox root is a WoW-like root, not a raw `AddOns` path
- CLI automation seeds and checks the sandbox; TUI remains a short manual pass

## Seed

```powershell
pwsh -File scripts/seed-sandbox.ps1 -SandboxRoot D:/Sandbox/WoWDev
```

Notes:
- default profile is `manual-smoke`
- pass `-Profile dev` only if you intentionally want to reuse the existing dev state DB
- Wago targets require a configured `WAGO_API_KEY`; the seed script fails before partial installs if that prerequisite is missing

## Smoke

```powershell
pwsh -File scripts/smoke-provider-matrix.ps1 -SandboxRoot D:/Sandbox/WoWDev
```

The scripts exit non-zero on the first failed prerequisite or contract mismatch.

## Matrix

The seeded target list lives in `scripts/manual-test-matrix.toml`.

Current curated targets:
- TukUI: `ElvUI`
- GitHub: `WeakAuras`
- WoWInterface: `TomTom`
- Wago: `BigWigs`
- manual fixture: `ManualSmokeAddon`

## TUI Pass

After a successful seed and smoke pass, run:

```powershell
pwsh -Command "& 'target/debug/lemonup.exe' --profile manual-smoke --addon-dir 'D:/Sandbox/WoWDev/_retail_/Interface/AddOns' tui"
```

Quick checklist:
- run one provider-backed selected update
- verify one manual addon reports a skip path cleanly
- confirm selection remains stable
- confirm list/detail text stays readable
- verify backup restore round-trips against the smoke profile when that flow changes
