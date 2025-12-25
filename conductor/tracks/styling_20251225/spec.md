# Specification: Adopt ink-color-pipe for Improved Styling

## Context
The project aims for a polished, modern, and customizable TUI. Currently, styling is likely handled via raw `ink` components or basic color props. To achieve the "World of Warcraft" aesthetic and support future theming (light/dark modes, user themes), we need a robust foundation. `ink-color-pipe` offers a flexible, string-based styling syntax (chalk-pipe) that is ideal for creating a centralized theme registry.

## Goals
1.  **Install & Configure:** specific library `ink-color-pipe` is installed.
2.  **Centralize Styling:** Create a `theme.ts` (or similar) to define semantic style constants (e.g., `success`, `error`, `highlight`, `wowPrimary`) using `ink-color-pipe` syntax.
3.  **Refactor Components:** Update key UI components (ControlBar, Header, HelpPanel, etc.) to use the `<Color>` component and the centralized theme constants.
4.  **Verify:** Ensure no visual regressions and that the new styling system works as expected.

## Technical Design
-   **Dependency:** `ink-color-pipe`
-   **Theme System:**
    -   Create `src/tui/theme.ts`.
    -   Export a `THEME` object containing style strings (e.g., `{ success: 'green.bold', error: 'bgRed.white' }`).
    -   This allows changing the "theme" in one place.
-   **Component Usage:**
    -   Replace `<Text color="green">` with `<Color styles={THEME.success}>`.
    -   Use `chalkPipe` for programmatic string formatting if needed (e.g., logging).

## User Experience
-   **Visuals:** The TUI should look more consistent.
-   **Future-Proofing:** This change is primarily foundational, enabling easier re-skinning later.

## Constraints
-   Must maintain the existing dark-mode friendly look for now (no drastic visual changes yet, just implementation refactor).
-   Must use `bun` for all package operations.
