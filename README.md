# LemonUp

<div align="center">
  <img src="./lemonup.png" alt="LemonUp Logo" width="150" />
  <h3>World of Warcraft Addon Manager</h3>
  <p>Fast, terminal-first addon management for WoW.</p>
</div>

---

[![Test](https://github.com/archcorsair/lemonup/actions/workflows/test.yml/badge.svg?branch=main)](https://github.com/archcorsair/lemonup/actions/workflows/test.yml)

**LemonUp** finds your WoW install, shows every addon and whether it has an update, and keeps
them up to date from GitHub, WoWInterface, TukUI and Wago.io. It runs in your terminal as a TUI,
with a CLI for scripts, and ships as one binary for Linux, macOS and Windows.

![LemonUp updating addons](https://lemonup.org/demos/lemonup-manage.gif)

## Install

**Homebrew** (macOS and Linux):

```sh
brew install archcorsair/lemonup/lemonup
```

**Scoop** (Windows):

```sh
scoop bucket add lemon-bucket https://github.com/archcorsair/lemon-bucket
scoop install lemonup
```

**Or download a binary** from [Releases](https://github.com/archcorsair/lemonup/releases) and put
it on your `PATH`.

Coming from LemonUp 1? Your settings and linked addons carry over on the first run. See
[Upgrading](https://lemonup.org/upgrading).

## Features

- **Setup in three steps:** LemonUp searches for your game, lists Retail and WoW Forever with
  their addon counts, and accepts any folder inside the install
- **Addon list:** every addon with its status (Update ready, Up to date, Newer than source, Not
  linked), sorting, a `/` filter, multi-select, and Details docked on the right in wide terminals
- **Install** by name (Wago.io) or by pasting a GitHub, WoWInterface, TukUI or Wago.io link
- **Find online:** press `f` on a "Not linked" addon to link it to its source, so LemonUp keeps
  it updated
- **Safe updates:** new folders are staged and rolled back if anything fails, folders that belong
  to another addon are never replaced, and deletes can be undone with `z`
- **Backups of your game settings** (the WTF folder) before updates, with restore
- **Keyboard first:** vim-style `j`/`k` and `h`/`l` alongside the arrow keys, `?` for Help on
  every screen
- **Works in any terminal:** a Terminal theme that uses your colors, `NO_COLOR` support, and an
  ASCII mode for consoles without Unicode
- **CLI** for scripts and automation, plus addon list import and export

## Quick start

Run the TUI:

```sh
lemonup
```

Main keys: `U` updates every addon with an update ready, `C` checks every addon, `enter` opens
Details, `i` installs, `s` opens Settings, `b` opens Backups, and `?` shows Help for the current
screen. `q` goes back, and quits from the addon list.

Check and update from the command line:

```sh
lemonup check
lemonup check WeakAuras
lemonup update WeakAuras
lemonup update-all --dry-run
```

Searching Wago.io by name needs a Wago.io API key (Wago gives them to Patreon supporters).
GitHub, WoWInterface and TukUI links work without one.

To manage a second game version, or to keep separate setups, use a profile:

```sh
lemonup --profile forever
```

## Troubleshooting

**Leftover `.lemonup-staging-*` folder in AddOns.** An update replaces addon folders through a
hidden `.lemonup-staging-*` folder inside your AddOns folder and deletes it when the update is
done. If something (an antivirus, a locked file) keeps it from being deleted, the update still
succeeded, but the folder stays behind and holds the previous versions of the addons it updated.
`lemonup sync` and the TUI warn when they see one. The game ignores it. To recover, copy any
addon folder you want back out of its `old` subfolder, then delete the whole
`.lemonup-staging-*` folder once nothing else is using it.

## Documentation

Guides for setup, keyboard shortcuts, addon sources, configuration and the CLI are at
[**lemonup.org**](https://lemonup.org).

## Contributing

Contributions are welcome. Read [AGENTS.md](AGENTS.md) first: it covers the architecture, the
safety rules every change has to keep, and the checks to run:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

MIT © [ArchCorsair](https://github.com/archcorsair)
