# Specification: Fix Config Schema Validation Errors in Tests

## Context
Running `bun run test` produces visible `ZodError` logs in the console, even though the tests pass. This is because the `ConfigManager.get()` method logs errors when validation fails, and the tests are initializing `ConfigManager` with empty configuration directories, causing `destDir` validation to fail (it is required in the schema but missing in empty config). While the application recovers gracefully, these errors clutter the test output and indicate that our tests aren't setting up valid mock configurations.

## Goals
1.  **Analyze Failure:** Confirm that `destDir` missing is the root cause of the Zod errors in `tests/core/config.test.ts`.
2.  **Fix Tests:** Update `tests/core/config.test.ts` to ensure `ConfigManager` is initialized with a valid `destDir` (e.g., using `overrides` or setting up a mock config file) or adjust the test expectations if the failure is intended.
3.  **Silence Expected Errors:** If a test *intentionally* checks failure behavior, verify that the error logging is suppressed or asserted against, rather than leaking to console.
4.  **Schema Refinement (Optional):** Consider if `destDir` should be optional with a default (e.g. "NOT_CONFIGURED") in the schema itself, rather than handled in `get()` fallback.

## Technical Design
-   **Test Update:** In `beforeEach` or individual tests in `config.test.ts`, ensure `ConfigManager` receives a valid `destDir` via `overrides` if we want to avoid the error.
-   **Schema Update:** Change `ConfigSchema.destDir` to `z.string().default("NOT_CONFIGURED")`. This would make empty configs valid by default, removing the error entirely.

## Constraints
-   Must not break existing functionality (e.g. `createDefaultConfig`).
-   Must eliminate the `ZodError` logs from clean test runs.
