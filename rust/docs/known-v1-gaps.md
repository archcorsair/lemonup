# Known v1 Gaps To Avoid Porting

Captured from current repository code and tests, not speculation.

- `src/core/commands/UpdateAddonCommand.ts` still has unresolved loose-zip handling FIXME comments.
- `tests/core/commands/scan_relationships.test.ts` documents incomplete multi-folder scan consolidation.
- `src/core/manager.ts` still has a TODO around owner lookup during update checks.
- `src/core/config.ts` has an unfinished safe-mode branch.
- Current CLI surface is extremely narrow and piggybacks on the TUI-oriented manager model.

## v2 implication

- Model folder ownership and extraction rules explicitly in the core crate.
- Make scan/install/update planner deterministic and test-first.
- Keep CLI over the shared core, not over UI state or UI-only orchestration.
