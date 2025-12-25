# Plan: Implement Light Theme and Theme Settings

## Phase 1: Theme Engine & Configuration

- [x] Task: Update Config Schema for Theme 38b7d1a
  - **Description:** Add the `theme` property to the configuration schema.
  - **Files:** `src/core/config.ts`, `tests/core/config.test.ts`
  - **Step 1:** Update `configSchema` in `src/core/config.ts` to include `theme`.
  - **Step 2:** Verify with test.

- [x] Task: Define Light Theme 0162416
  - **Description:** Create the light theme definition in the registry.
  - **Files:** `src/tui/theme.ts`, `tests/tui/theme.test.ts`
  - **Step 1:** Rename `THEME` to `DARK_THEME`.
  - **Step 2:** Create `LIGHT_THEME` with appropriate color mappings (e.g., `white` -> `black`, `gray` -> `gray` or `black.dim`).
  - **Step 3:** Export a helper or map to retrieve theme by name.
  - **Step 4:** Ensure `THEME` is still exported (as a getter or proxy) temporarily if needed, or prepare for refactor.
  - **Step 5:** Update tests.

- [ ] Task: Update Store for Dynamic Theming
  - **Description:** specific `theme` state in the Zustand store.
  - **Files:** `src/tui/store/useAppStore.ts`, `tests/tui/store.test.ts` (if exists) or create one.
  - **Step 1:** Add `theme` (the object) and `themeMode` (string) to the store interface.
  - **Step 2:** Initialize store with the theme from `config.get('theme')`.
  - **Step 3:** Implement `setTheme` action that updates config and store.

- [ ] Task: Conductor - User Manual Verification 'Theme Engine & Configuration' (Protocol in workflow.md)

## Phase 2: Component Migration & UI

- [ ] Task: Create `useTheme` Hook
  - **Description:** A helper hook to easily access the current theme in components.
  - **Files:** `src/tui/hooks/useTheme.ts`
  - **Step 1:** Create hook that selects `state.theme` from store.

- [ ] Task: Refactor Components to use Dynamic Theme
  - **Description:** Update all components to use `useTheme()` instead of importing `THEME`.
  - **Files:** `src/tui/components/*.tsx`
  - **Step 1:** Update `Header.tsx`, `ControlBar.tsx`, `RepositoryRow.tsx`, `HelpPanel.tsx`, `ScreenTitle.tsx`.
  - **Step 2:** Replace `import { THEME }` with `const theme = useTheme()`.

- [ ] Task: Add Theme Setting to ConfigScreen
  - **Description:** Add a selector to change the theme.
  - **Files:** `src/tui/screens/ConfigScreen.tsx`
  - **Step 1:** Add a new field/row for "Theme".
  - **Step 2:** Use `SelectInput` or similar to toggle Dark/Light.
  - **Step 3:** Call `setTheme` on change.

- [ ] Task: Conductor - User Manual Verification 'Component Migration & UI' (Protocol in workflow.md)
