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

**Nix** (flake, macOS and Linux):

```sh
nix run github:archcorsair/lemonup            # try it
nix profile install github:archcorsair/lemonup
```

Or add it as a flake input for NixOS or Home Manager:

```nix
inputs.lemonup.url = "github:archcorsair/lemonup";
# then: inputs.lemonup.packages.${pkgs.system}.default
# in environment.systemPackages or home.packages
```

Nix downloads LemonUp prebuilt from the `lemonup` Cachix cache; it asks once whether to trust it.

**Arch Linux:** AUR packages (`lemonup` and `lemonup-bin`) are coming soon. Until then, build
the package from this repo with `makepkg -si` in `packaging/aur/lemonup-bin`.

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
- **Find your addons online:** press `F` to look up every "Not linked" addon (or the selected
  ones) on Wago.io, WoWInterface and TukUI by its files, see what matched, and link the ones you
  pick after one question. Partial matches can be replaced with the source's copy. `f` does the
  same for one addon, and offers a Wago.io name search when nothing matches. Keys `1`, `2` and `3`
  choose which sources to look at, and LemonUp remembers the choice for each game
- **Edit .toc fields:** press `e` in Details to change the Title, Notes or Category the game shows
  for an addon. Updates keep your values, and clearing one puts the original back
- **Safe updates:** new folders are staged and rolled back if anything fails, folders that belong
  to another addon are never replaced, and deletes can be undone with `z`
- **Backups of your game settings** (the WTF folder) before updates, with restore
- **Keyboard first:** vim-style `j`/`k`, `h`/`l` and `gg`/`G` alongside the arrow keys, `?` for Help on
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

Find where the addons LemonUp didn't install come from (Wago.io, WoWInterface and TukUI), then
link the exact matches so LemonUp keeps them updated. Nothing is downloaded:

```sh
lemonup find
lemonup find --link
```

`lemonup find` prints a readable report: what is ready to link, what is only a likely match, and
what was not found, ending with what was changed (nothing, unless you pass `--link`). Add
`--plain` for one `key=value` line per result, for scripts.

`--source` limits which sources are asked, as a comma separated list of `wago`, `wowinterface` and
`tukui` (default: all of them; the choice the TUI remembers is not used):

```sh
lemonup find --source wowinterface,tukui
```

Searching Wago.io by name needs a Wago.io API key (Wago gives them to Patreon supporters).
GitHub, WoWInterface and TukUI links work without one.

### Your games

LemonUp looks after each game version you play, such as Retail and WoW Forever, with its own
addon list, backups of your game settings and game folder. If Setup finds both Retail and WoW
Forever, its last step can set up the other one too ("Also set up", off until you turn it on).
Press `p` on the addon list to switch
games or to add another one; Add another game finds the installs you haven't set up yet. In the
same sheet, `x` removes a game you're not using from LemonUp after asking. Its addons, game
settings and backups stay on disk.

Plain `lemonup` opens the game you used last. The header shows when another game has updates
ready: while LemonUp is open it checks your other games in the background (Settings, "Check other
games", from Off to every 3 days; once a day by default). It only checks them and never updates
or changes them.

Settings shows what all games share (the Wago.io key, theme, symbols, motion and update checks)
apart from what belongs to the game you're using (backups, show libraries and the game folder).

On the command line each game is a profile:

```sh
lemonup profiles
lemonup --profile forever check
```

`lemonup profiles` lists each game with its profile name, game version, AddOns folder and addon
count. Commands use the default game unless you pass `--profile`.

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
