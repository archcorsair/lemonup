# LemonUp 🍋 (Alpha)

<div align="center">
  <img src="./lemonup.png" alt="LemonUp Logo" width="150" />
  <h3>World of Warcraft Addon Manager</h3>
  <p>A fast, lightweight, and beautiful TUI for managing your WoW addons.</p>
  <img width="1514" height="746" alt="lemonup screenshot" src="https://github.com/user-attachments/assets/f8af4c10-a84a-4ace-b9a6-4806add3294a" />
</div>

---

**LemonUp** is a terminal-based addon manager for World of Warcraft built with [Bun](https://bun.com) and [Ink](https://github.com/vadimdemedes/ink). It simplifies the process of installing, updating, and managing your addons directly from your terminal with a modern, interactive interface.
## ⚠️ Active Development
<p><strong>🚧 This project is under active development and isn't production ready, use at your own risk! 🚧</strong></p>

**Supported Addon Sources**

| Platform      | Support Status |
|---------------|----------------|
| GitHub        | ✅ Supported   |
| WowInterface  | ✅ Supported   |
| TukUI         | ✅ Supported   |
| Wago.io       | 📋 Planned     |
| WowAce        | ❌ Unsupported |
| CurseForge    | ❌ Unsupported |

**Supported Game Flavors**

| Flavor        | Support Status |
|---------------|----------------|
| Retail        | ✅ Supported   |
| Classic       | ⏳ WIP         |
| Other         | ⏳ WIP         |

## ✨ Features

- **🚀 Fast & Lightweight:** Built on the blazing-fast Bun runtime.
- **🖥️ Beautiful TUI:** Interactive terminal user interface powered by Ink.
- **📦 Multi-Source Support:**
  - Install directly from **GitHub** repositories.
  - Install/Update **TukUI** & **ElvUI** seamlessly.
  - Support for **WoWInterface** addons.
- **🎨 Theming:** Dark and light themes inspired by [tokyonight](https://github.com/folke/tokyonight.nvim).
  - Custom themes coming soon.
- **⌨️ Keyboard Driven:** Vim-style navigation (j/k) and shortcuts for all actions.
- **🔍 Search & Filter:** Fuzzy search addons by name or author.
- **📚 Library Management:** Track addon dependencies separately, toggle visibility.
- **🛡️ Secure & Safe:**
  - Robust path traversal protection for zip extraction.
  - Automatic **WTF folder backups** before updates.
- **🔄 Smart Updates:** Checks for updates using Git commit hashes for accuracy.
- **⚡ Parallel Processing:** Updates multiple addons concurrently.

## Getting Started

The easiest way to start using LemonUp is by heading over to our [getting started page](https://lemonup.org/getting-started.html)

## 🛠️ Building & Installation

LemonUp aims to provide pre-built executables for all major operating systems in future releases. For now, you can build it yourself:

**Prerequisites:** Ensure you have [Bun](https://bun.com) installed.

1. **Clone the repository:**
   ```bash
   git clone https://github.com/archcorsair/lemonup.git
   cd lemonup
   ```

2. **Install dependencies:**
   ```bash
   bun install
   ```

3. **Build the executable:**
   ```bash
   bun run build
   ```
   This will generate a standalone `lemonup` executable for your current operating system in the project root.

## 🚀 Usage

You can run the built executable directly:

```bash
./lemonup
```

Or run via Bun during development:

```bash
bun run start
```

### CLI Arguments

- `--cli`: Run in non-interactive CLI mode.
- `--force`, `-f`: Force re-installation/update of addons.
- `--version`, `-v`: Show version information.

## 🔧 Configuration

On first run, LemonUp will guide you through a setup wizard to locate your WoW installation.

You can access the **Settings** menu at any time to configure:
- WoW Addon Directory Path
- Max Concurrent Downloads
- Update Check Interval
- Auto-backup WTF Folder
- Backup Retention Policy
- Nerd Font Support
- Theme (Dark/Light)
- Debug Logging

## ⌨️ Keyboard Shortcuts

LemonUp supports vim-style navigation (`h`/`j`/`k`/`l`) throughout the interface.

| Screen | Key | Action |
|--------|-----|--------|
| All | `↑`/`↓` or `j`/`k` | Navigate |
| All | `esc` | Back/Cancel |
| Main Menu | `t` | Toggle theme |
| Main Menu | `space` | Set default option |
| Manage | `/` | Search addons |
| Manage | `1`-`4` | Sort by column |
| Manage | `space` | Select addon |
| Manage | `l` | Toggle libraries |
| Manage | `u` | Update selected |
| Manage | `c` | Check for updates |
| Manage | `d` | Delete selected |
| Manage | `b` | Backup WTF folder |

## 🤝 Contributing

Contributions are welcome! Please run the pre-commit checks before submitting a PR:

```bash
bun run typecheck
bun test
```

## 📄 License

MIT © [ArchCorsair](https://github.com/archcorsair)
