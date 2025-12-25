# Plan: Fix Config Schema Validation Errors in Tests

## Phase 1: Investigation & Fix

- [ ] Task: Reproduce and Analyze Zod Error
  - **Description:** Run tests and identify exactly which tests trigger the error and why.
  - **Files:** `tests/core/config.test.ts`, `src/core/config.ts`
  - **Step 1:** Run `bun test tests/core/config.test.ts` and capture output.
  - **Step 2:** Confirm `destDir` is the culprit.

- [ ] Task: Update Config Schema Defaults
  - **Description:** Make `destDir` optional with a default value in the Zod schema to align with the application's "NOT_CONFIGURED" fallback logic. This is the cleanest fix.
  - **Files:** `src/core/config.ts`
  - **Step 1:** Update `ConfigSchema` for `destDir` to use `.default("NOT_CONFIGURED")`.
  - **Step 2:** Verify that `ConfigManager` logic still handles this correctly.

- [ ] Task: Clean up ConfigManager Fallback Logic
  - **Description:** Since validation should now pass for empty objects, the `get()` fallback logic for `destDir` might be redundant or can be simplified.
  - **Files:** `src/core/config.ts`
  - **Step 1:** Review `get()` method.
  - **Step 2:** Ensure validation logs are still useful for *actual* errors (e.g. wrong types).

- [ ] Task: Verify Fix
  - **Description:** Run tests again to ensure no Zod errors appear.
  - **Files:** `tests/core/config.test.ts`
  - **Step 1:** Run `bun test`.

- [ ] Task: Conductor - User Manual Verification 'Investigation & Fix' (Protocol in workflow.md)
