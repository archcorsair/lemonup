# Technology Stack

## Core Runtime & Language
- **Runtime:** [Bun](https://bun.com) - A fast all-in-one JavaScript runtime.
- **Language:** [TypeScript](https://www.typescriptlang.org/) - Providing static typing and modern language features.

## User Interface (TUI)
- **Framework:** [Ink](https://github.com/vadimdemedes/ink) - React for interactive command-line apps.
- **State Management:** [Zustand](https://github.com/pmndrs/zustand) - A small, fast, and scalable bearbones state-management solution.
- **Components/Styling:**
    - `ink-gradient` - For beautiful color transitions.
    - `ink-spinner` - For loading indicators.
    - `ink-text-input` - For interactive user input.

## Data & Configuration
- **Database:** `bun:sqlite` - Bun's native, high-performance SQLite driver for addon metadata.
- **Configuration:** `conf` - Simple config handling for user preferences.
- **Validation:** [Zod](https://zod.dev/) - TypeScript-first schema declaration and validation.

## Tools & Utilities
- **Addon Providers:**
    - `git` - For GitHub-based installations and updates.
    - Custom providers for TukUI and WoWInterface.
- **Utilities:**
    - `adm-zip` - For zip file extraction.
    - `fuse.js` - For fuzzy searching addon lists.
    - `p-limit` - For managing concurrent download tasks.

## Quality Assurance
- **Linting & Formatting:** [Biome](https://biomejs.dev/) - A high-performance toolchain for web projects.
- **Type Checking:** `tsc` - The TypeScript compiler.
- **Testing:** `bun run test` - Bun's built-in test runner, configured with custom flags for optimized output.
- **Git Hooks:** `lefthook` - Fast and powerful Git hooks manager.
