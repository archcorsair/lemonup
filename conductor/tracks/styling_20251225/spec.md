# Specification: Tune Light Theme for Readability

## Context
The initial implementation of the Light Theme used standard ANSI colors which often lack sufficient contrast on white terminal backgrounds (e.g., standard Yellow or Cyan can be unreadable). To ensure accessibility and a polished look, we need to tune the color palette specifically for light mode.

## Goals
1.  **High Contrast:** Ensure all text is legible against a white background.
2.  **Semantic Clarity:** Maintain clear distinction between status types (Success, Error, Warning, Info) using appropriate colors.
3.  **Aesthetic:** Create a pleasing, modern look that fits the "clean" design goal.

## Technical Design
-   **File:** `src/tui/theme.ts`
-   **Palette Adjustments (Light Theme):**
    -   **Text:** `black` or `black.bold`.
    -   **Brand:** Shift from `cyan` to something darker like `blue` or `magenta` if `cyan` is too light.
    -   **Success:** Standard `green` might be too light. Consider `green` (standard ANSI green is usually darker than bright green). Avoid `greenBright`.
    -   **Warning:** `yellow` is notoriously hard to read on white. Consider `magenta` or `red` (if distinct from error) or use background colors (e.g., `black` text on `yellow` bg). Or use `keywords` like `blue`.
    -   **Error:** `red` is usually fine.
    -   **Table/List Headers:** `black.bold`.
    -   **Selected Item:** `blue` or `inverse` (white on black).

## Constraints
-   Must work within `ink-color-pipe` capabilities (standard ANSI colors + modifiers).
-   Must not affect `DARK_THEME`.
