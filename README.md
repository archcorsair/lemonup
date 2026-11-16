# LemonUp

<div align="center">
  <img src="./lemonup.png" alt="LemonUp Logo" width="150" />
  <h3>World of Warcraft Addon Manager</h3>
  <p>Fast, terminal-first addon management for WoW.</p>
</div>

---

[![Test](https://github.com/archcorsair/lemonup/actions/workflows/test.yml/badge.svg?branch=main)](https://github.com/archcorsair/lemonup/actions/workflows/test.yml)

**LemonUp** is a Rust TUI and CLI for installing, checking, updating, importing, exporting, backing up, and restoring World of Warcraft addons.

## Current Scope

- Retail and WoW Forever (`_forever_`) support, one AddOns folder per profile (use `--profile` to manage both)
- GitHub, TukUI, WoWInterface, and Wago sources
- TUI + CLI workflows
- Background update checks
- WTF backup and restore

## Highlights

- Terminal-native addon list with Details, sorting, a `/` filter, multi-select, bulk update/delete, and `z` undo delete
- Not linked addons can be matched to a source with `f` (find online)
- Install by search or from a pasted URL
- Guided setup in three steps with strict addon-path safety rails
- Portable addon import/export format
- Wago search and install support with your own API key

## Quick Start

Launch the TUI:

```powershell
lemonup
```

Use a separate profile when you want isolated config/state, for example:
- manual smoke testing
- trying a different addon directory
- keeping work and personal setups separate

```powershell
lemonup --profile manual-smoke tui
```

Main keys in the TUI: `U` updates every addon with an update ready, `C` checks every addon,
`s` or `,` opens Settings, `b` opens Backups, `?` shows Help for the current screen. `q` goes
back and quits only from the list.

Check for updates:

```powershell
lemonup check
lemonup check WeakAuras
```

Update addons:

```powershell
lemonup update
lemonup update WeakAuras
lemonup update-all --dry-run
```

## Releases

Download the latest binaries from [Releases](https://github.com/archcorsair/lemonup/releases).

## Troubleshooting

**Leftover `.lemonup-staging-*` folder in AddOns.** An update replaces addon folders through a
hidden `.lemonup-staging-*` folder inside your AddOns folder and deletes it when the update is
done. If something (an antivirus, a locked file) keeps it from being deleted, the update still
succeeded, but the folder stays behind and holds the previous versions of the addons it updated.
`lemonup sync` and the TUI warn when they see one. The game ignores it. To recover, copy any
addon folder you want back out of its `old` subfolder, then delete the whole
`.lemonup-staging-*` folder once nothing else is using it.

## Documentation

For installation, configuration, usage guides, and detailed docs, visit:

[**lemonup.org**](https://lemonup.org)

## Contributing

Contributions are welcome.

Validation:

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

MIT © [ArchCorsair](https://github.com/archcorsair)
