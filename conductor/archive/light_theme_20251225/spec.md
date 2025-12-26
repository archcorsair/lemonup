# Specification: Implement Light Theme and Theme Settings

## Context
The application currently uses a hardcoded dark theme. To improve accessibility and support users with light terminal backgrounds, we need to implement a "Light" theme variant. This requires upgrading the theme system to support dynamic switching and adding a configuration option for the user to select their preference.

## Goals
1.  **Theme Definitions:** Define a `lightTheme` object in `src/tui/theme.ts` with colors optimized for light backgrounds (e.g., darker text, high contrast status colors).
2.  **Configuration:** Add a `theme` setting (enum: 'dark' | 'light') to the user configuration (`src/core/config.ts`) and Zod schema.
3.  **State Management:** Update the Zustand store (`useAppStore`) to hold the current active theme object based on the user's config.
4.  **UI Integration:** Add a setting in `ConfigScreen.tsx` to toggle between Dark and Light modes.
5.  **Dynamic Styling:** Ensure all components (Header, ControlBar, RepositoryRow, etc.) consume the *active* theme from the store/hook rather than the static `THEME` export.

## Technical Design
-   **Config:** Update `config.ts` schema: `theme: z.enum(["dark", "light"]).default("dark")`.
-   **Theme Registry (`src/tui/theme.ts`):**
    -   Rename `THEME` to `DARK_THEME`.
    -   Create `LIGHT_THEME` with inverted/adjusted colors (e.g., `brand` becomes `cyan.bold` (keep), but text colors like `white` become `black`).
    -   Export a type `Theme` based on the shape of `DARK_THEME`.
-   **Store (`src/tui/store/useAppStore.ts`):**
    -   Add `themeMode: 'dark' | 'light'`.
    -   Add `theme: Theme` (computed/derived).
    -   Action `setTheme(mode: 'dark' | 'light')` which updates the config and the store state.
-   **Component Updates:**
    -   Components currently import `{ THEME } from "../theme"`.
    -   They must change to `const theme = useAppStore(state => state.theme)` or a new hook `useTheme()`.

## User Experience
-   **Settings:** User sees a new "Theme" option in Settings.
-   **Visuals:** Switching to Light mode immediately updates colors to be readable on a white background.

## Constraints
-   Must maintain the existing dark theme as the default.
-   Must use `ink-color-pipe` strings.
