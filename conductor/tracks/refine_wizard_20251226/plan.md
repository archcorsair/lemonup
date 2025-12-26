# Plan: Refine First Run Wizard

## Phase 1: Theming and Progress Bar Visuals [checkpoint: eed2fbb]

- [x] Task: Audit and Refactor Theme Tokens for Wizard
  - **Description:** Review `src/tui/FirstRunWizard.tsx` for hardcoded colors. Update `src/tui/theme.ts` with any necessary semantic tokens for the wizard.
  - **Files:** `src/tui/FirstRunWizard.tsx`, `src/tui/theme.ts`
  - **Step 1:** Identify hardcoded colors in `FirstRunWizard.tsx`.
  - **Step 2:** Add missing semantic tokens to `src/tui/theme.ts` (e.g., specific colors for wizard elements).
  - **Step 3:** Replace hardcoded colors in `FirstRunWizard.tsx` with theme references.
  - **Commit:** 416869c

- [x] Task: Implement Spacing for Progress Bar Checkmark
  - **Description:** Add horizontal spacing around the "✓" symbol in `WizardProgress` component.
  - **Files:** `src/tui/FirstRunWizard.tsx`
  - **Step 1:** Modify `renderProgressBar` logic to add padding or spaces specifically for the checkmark marker.
  - **Commit:** ab140e4

- [ ] Task: Conductor - User Manual Verification 'Theming and Progress Bar Visuals' (Protocol in workflow.md)

## Phase 2: Directory Step Refinement and Bug Fixes

- [ ] Task: Relocate Validation Indicator and Improve Editing UI
  - **Description:** Move the validation icon to the left of the path. Replace `[EDITING]` label with a more refined visual cue.
  - **Files:** `src/tui/FirstRunWizard.tsx`
  - **Step 1:** Update `DirectoryStep` layout to render the validation icon (checkmark/cross) before the path/input.
  - **Step 2:** Redesign the editing state indicator (e.g., use a different border color or a prompt icon instead of `[EDITING]`).

- [ ] Task: Fix Directory Validation Bug and Implement Error Feedback
  - **Description:** Prevent proceeding from Step 2 if the path is invalid. Show a visual error message on failed attempt to proceed.
  - **Files:** `src/tui/FirstRunWizard.tsx`
  - **Step 1:** Update `useInput` logic for Step 2 to block `goNext()` if `pathValid` is false or directory is empty.
  - **Step 2:** Implement an error display within the `FirstRunWizard` that triggers when navigation is blocked due to invalid input.

- [ ] Task: Conductor - User Manual Verification 'Directory Step Refinement and Bug Fixes' (Protocol in workflow.md)
