# Initial Concept
World of Warcraft addon manager built with a Terminal User Interface (TUI). It allows users to install, update, and manage WoW addons from various sources like GitHub, TukUI, and WoWInterface directly from the terminal.

# Vision
LemonUp is a high-performance, terminal-first World of Warcraft addon manager. It prioritizes speed, a polished user experience, and a lightweight footprint, serving as a modern alternative to traditional GUI-based managers, especially for Linux and macOS users. As a personal passion project, it aims for technical excellence and a delightful terminal aesthetic.

# Target Audience
- **Terminal Enthusiasts:** WoW players who prefer the efficiency and feel of the command line.
- **Cross-Platform Users:** Linux and macOS users seeking a fast, native-feeling, and lightweight alternative to Electron-based managers.
- **Efficiency Seekers:** Users who value performance and a minimal system footprint.

# Core Goals
- **Polished TUI Experience:** Maintain a "terminal-first" aesthetic that is both intuitive and visually appealing, drawing inspiration from modern tools like `btop` and `lazygit`.
- **High Performance:** Leverage the Bun runtime for blazing-fast operations and concurrent processing.
- **Provider Parity:** Support a wide range of addon sources (GitHub, TukUI, WoWInterface, and upcoming Wago.io), intentionally excluding CurseForge to focus on open and developer-friendly platforms.
- **Customization:** Implement a flexible styling system that supports light/dark modes and eventually allows user-defined themes.

# Key Features & Priorities
- **Multi-Source Support:** Ongoing expansion of supported providers, with Wago.io as a near-term priority.
- **Robust Dependency Management:** Continuous quality review and improvement of the automatic library handling logic.
- **Reliable Backups:** Automatic WTF folder backups, with future plans for enhanced local snapshots and potential cloud sync.
- **Modern Distribution:** Stable, versioned releases with pre-built binaries, and a future path toward an automated self-update mechanism.
- **Data Portability:** Implementation of export and import functionality to allow users to easily back up or move their addon configurations (lower priority).
