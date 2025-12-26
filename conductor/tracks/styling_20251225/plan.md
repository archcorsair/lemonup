# Plan: Tune Light Theme for Readability

## Phase 1: Iterative Tuning

- [x] Task: Analyze and Refine Light Theme Palette
  - **Description:** Review `src/tui/theme.ts` and modify `LIGHT_THEME` definitions to use higher contrast colors. Focus on replacing readable colors (Yellow, Cyan) with darker alternatives (Blue, Magenta, Black).
  - **Files:** `src/tui/theme.ts`
  - **Step 1:** Read current `src/tui/theme.ts`.
  - **Step 2:** Modify `LIGHT_THEME` object.
    -   `brand`: cyan -> blue (or magenta)
    -   `success`: green -> green (ensure not bright)
    -   `warning`: yellow -> magenta (or orange if supported, likely not)
    -   `info`: blue -> blue
    -   `key`: yellow -> magenta or black.bold
  - **Step 3:** Commit changes.
  - **Status:** Done. Updated colors.

- [~] Task: User Feedback Loop
  - **Description:** Ask user to verify readability.
  - **Status:** Pending user confirmation.

- [ ] Task: Conductor - User Manual Verification 'Iterative Tuning' (Protocol in workflow.md)
