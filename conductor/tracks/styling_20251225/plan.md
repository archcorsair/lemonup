# Plan: Adopt ink-color-pipe

## Phase 1: Foundation & Setup [checkpoint: 9985789]

- [x] Task: Install `ink-color-pipe` dependency ed0a184
  - **Description:** Add the library to the project using Bun.
  - **Files:** `package.json`
  - **Step 1:** Run `bun add ink-color-pipe`.

- [x] Task: Create centralized Theme Registry 3b2ff12
  - **Description:** Define the semantic style constants for the application.
  - **Files:** `src/tui/theme.ts`
  - **Step 1:** Create `src/tui/theme.ts`.
  - **Step 2:** Define a `THEME` constant object.
  - **Step 3:** Populate it with initial semantic keys matching current usage (e.g., `brand`, `success`, `error`, `warning`, `muted`, `highlight`). Use `ink-color-pipe` style strings (e.g., `'cyan.bold'`).

- [x] Task: Conductor - User Manual Verification 'Foundation & Setup' (Protocol in workflow.md)

## Phase 2: Component Refactoring (Core UI)

- [x] Task: Refactor `Header.tsx` to use Theme b72368b
  - **Description:** Update the Header component to use `<Color>` and `THEME`.
  - **Files:** `src/tui/components/Header.tsx`
  - **Step 1:** Write Test: Verify Header renders with current text.
  - **Step 2:** Implementation: Import `Color` from `ink-color-pipe` and `THEME`. Replace direct `Text` color props or `ink-gradient` (if replacing) or wrap content in `<Color styles={THEME.brand}>` where appropriate.
  - **Step 3:** Verify: Ensure visual output remains consistent/improved.

- [x] Task: Refactor `ControlBar.tsx` to use Theme 0a24f54
  - **Description:** Update the ControlBar (footer) to use `<Color>` and `THEME`.
  - **Files:** `src/tui/components/ControlBar.tsx`
  - **Step 1:** Write Test: Verify ControlBar rendering.
  - **Step 2:** Implementation: Use `THEME.keybinding` (or similar) for keys and `THEME.description` for text.

- [x] Task: Refactor `ScreenTitle.tsx` to use Theme 439ab1b
  - **Description:** Update the ScreenTitle to use `<Color>` and `THEME`.
  - **Files:** `src/tui/components/ScreenTitle.tsx`
  - **Step 1:** Write Test: Verify ScreenTitle.
  - **Step 2:** Implementation: Apply standard heading styles from `THEME`.

- [ ] Task: Conductor - User Manual Verification 'Component Refactoring (Core UI)' (Protocol in workflow.md)

## Phase 3: Component Refactoring (Lists & Help)

- [ ] Task: Refactor `RepositoryRow.tsx` to use Theme
  - **Description:** Update the list items to use standard styling for names, versions, and status.
  - **Files:** `src/tui/components/RepositoryRow.tsx`
  - **Step 1:** Write Test: Verify row rendering.
  - **Step 2:** Implementation: Use `THEME.addonName`, `THEME.version`, `THEME.installed`, etc.

- [ ] Task: Refactor `HelpPanel.tsx` to use Theme
  - **Description:** Update the help panel to use theme constants.
  - **Files:** `src/tui/components/HelpPanel.tsx`
  - **Step 1:** Write Test: Verify help panel.
  - **Step 2:** Implementation: Standardize help text styling.

- [ ] Task: Conductor - User Manual Verification 'Component Refactoring (Lists & Help)' (Protocol in workflow.md)
