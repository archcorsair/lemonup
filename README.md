# LemonUp 🍋

<div align="center">
  <img src="./lemonup.png" alt="LemonUp Logo" width="150" />
  <h3>World of Warcraft Addon Manager</h3>
  <p>A fast, lightweight, and beautiful TUI for managing your WoW addons.</p>
  <img width="1514" height="746" alt="lemonup screenshot" src="https://github.com/user-attachments/assets/f8af4c10-a84a-4ace-b9a6-4806add3294a" />
</div>

---

[![Built with Bun](https://img.shields.io/badge/Built_with-Bun-fbf0df?logo=bun&labelColor=212121)](https://bun.sh)
[![Test](https://github.com/archcorsair/lemonup/actions/workflows/test.yml/badge.svg?branch=main)](https://github.com/archcorsair/lemonup/actions/workflows/test.yml)

**LemonUp** is a high-performance, terminal-first addon manager for World of
Warcraft. Built with [Bun](https://bun.com) and
[Ink](https://github.com/vadimdemedes/ink), it provides a modern and efficient
way to handle your addons without leaving the terminal.

## Compatibility

| Platform Support               | Content Sources          |
| :-------------------------     | :----------------------- |
| ✅ **Retail**                  | ✅ **GitHub**            |
| 📋 **Classic / Era** (Planned) | ✅ **TukUI** (ElvUI)     |
| 📋 **Cata** (Planned)          | ✅ **WoWInterface**      |
|                                | ✅ **Wago.io**           |
|                                | 📋 **WeakAuras** (Planned) |

### Supported Architectures

- **Windows:** x64
- **Linux:** x64
- **macOS:** Apple Silicon (arm64), Intel (x64)

## Features

- **Interactive TUI:** A beautiful, terminal-native user interface with fluid
  animations and responsive layout.
- **Multi-Source Support:** Seamlessly install and update addons directly
  from **GitHub**, **TukUI**, **WoWInterface**, and **Wago.io**.
- **Smart Updates & Dependencies:** Git-based version tracking ensures
  pinpoint accuracy, while robust library handling keeps your dependencies in
  check.
- **Intelligent Setup:** Intelligent WoW installation detection and a guided
  first-run wizard to get you started in seconds.
- **⌨️ Keyboard Driven:** Full **Vim-style** navigation (`h/j/k/l`) and
  intuitive shortcuts for all actions.
- **Wago.io Integration:** Bring your own API key to search and install addons
  directly from Wago.io within the app.
- **WeakAuras Management:** Coming soon.

## 📦 Install

### Binary Releases

Download the latest pre-built binaries from the
[**Releases**](https://github.com/archcorsair/lemonup/releases) page.

### Package Managers

#### Homebrew (macOS & Linux)

```bash
brew install archcorsair/lemonup/lemonup
```

#### Scoop (Windows)

```bash
scoop bucket add lemon-bucket https://github.com/archcorsair/lemon-bucket
scoop install lemonup
```

## 📖 Documentation

For detailed installation guides, configuration options, and usage instructions,
please visit our official documentation website:

[**lemonup.org**](https://lemonup.org)

## 🤝 Contributing

Contributions are welcome! Please see our
[contribution guidelines](https://lemonup.org/contributing) on the docs site.

```bash
bun install
bun run typecheck
bun test
```

## 📄 License

MIT © [ArchCorsair](https://github.com/archcorsair)
