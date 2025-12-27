# Specification: Refine First Run Wizard

## Overview
This track focuses on polishing the recently implemented `FirstRunWizard` in the LemonUp TUI. The goals are to improve visual spacing in the progress bar, refine the directory selection step (layout, validation logic, and editing state), fix a critical bug allowing progress with invalid paths, and ensure full compliance with the application's theming system.

## Functional Requirements

### 1. Progress Bar Styling
*   **Spacing:** Add slight horizontal spacing around the checkmark symbol ("✓") in the wizard's top progress bar. The circles ("●", "○") should remain as they are.

### 2. Directory Step Improvements
*   **Layout:** Move the validation indicator (checkmark or error symbol) to the **LEFT** of the directory path display/input field.
*   **Editing State:** Remove the current `[EDITING]` text label. Replace it with a more subtle and aesthetically pleasing visual cue (e.g., a distinct border color, a specific icon, or a style change to the input field itself) to indicate active text input.
*   **Validation Logic (Bug Fix):**
    *   Prevent the user from proceeding to the next step if the directory path is invalid.
    *   **Error Feedback:** If the user attempts to proceed (presses Enter) with an invalid path, display a visual error message explaining that a valid WoW AddOns directory is required.

### 3. Theming Compliance
*   **Audit & Refactor:** Comprehensive audit of `src/tui/FirstRunWizard.tsx` to identify any hardcoded colors.
*   **Implementation:** Replace all hardcoded colors with semantic tokens from the `useTheme` hook.
*   **Theme Extension:** If existing theme tokens are insufficient for specific wizard elements, refactor `useTheme` (and `theme.ts`) to include new semantic tokens and utilize them in the wizard.

## Non-Functional Requirements
*   **Code Style:** Maintain consistency with the existing React/Ink patterns in the project.
*   **UX:** Ensure visual feedback is immediate and clear, especially for validation errors and editing states.

## Acceptance Criteria
*   The progress bar shows extra spacing around the checkmark icon only.
*   The directory validation icon appears to the left of the path.
*   The `[EDITING]` label is gone, replaced by a refined visual indicator.
*   Users **cannot** advance past the Directory step if the path is invalid.
*   Attempting to advance with an invalid path displays an error message.
*   The wizard correctly renders in both Light and Dark modes without any hardcoded color values.
