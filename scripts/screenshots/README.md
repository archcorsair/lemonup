# Website screenshots

`generate.sh` renders the LemonUp screenshots used on lemonup.org. It runs
the real release binary against a fake, offline WoW install and records each
screen with [VHS](https://github.com/charmbracelet/vhs), so the images can be
regenerated after any UI change.

## What it does

1. Builds `target/release/lemonup` (skipped with `--bin`).
2. Wipes and recreates the sandbox at `/tmp/lemonup-demo` (the path shown in
   the addon list header). `fixture.py` writes:
   - `World of Warcraft/` with `Data/`, `.build.info` and `_retail_/Wow.exe`,
     so it passes LemonUp's install validation
   - `_retail_/Interface/AddOns/` with 35 addon folders (18 rows on the
     addon list) and a small `_retail_/WTF/`
   - a LemonUp v1 `config.json` and `lemonup.db` under the sandbox `HOME`,
     tracking 14 addons from GitHub, Wago, TukUI and WoWInterface, four of
     them with a newer stored remote version (shown as "Update ready", no
     network) and one (HandyNotes) whose stored remote is older ("Newer than
     source"); the four untracked addons show as "Not linked"
   - three WTF backup archives in the v2 backup folder
   - `fresh/home/Games/World of Warcraft/` with `_retail_` (18 addons) and
     `_forever_` (3 addons) AddOns folders, under a separate, empty `HOME`,
     for the first-run Setup screen
3. Runs `lemonup sync` once, which performs the v1 migration and a local scan,
   then checks the migrated state with `sqlite3` (read-only; the state is
   exactly what the migration produced).
4. Runs every tape in `tapes/` from inside the sandbox and checks the captured
   terminal text for the real home directory or user name.
5. Rounds the window corners with transparency, strips metadata and writes
   lossless WebP files (2880x1620, a 2x image) to the output directory. It
   deletes a `NAME.png` left there by older versions of this script, for the
   screen names it writes only. Lossless WebP is about 44% smaller than the
   optimized PNGs and pixel-identical.

Every LemonUp run goes through a wrapper script that sets `HOME` and the
`XDG_*` directories inside the sandbox and points the HTTP proxy variables at
a closed port, so an accidental request fails. `/tmp/lemonup-demo/lemonup`
uses the migrated fixture; `/tmp/lemonup-demo/lemonup-fresh` uses the empty
`HOME` in `fresh/`, so Setup opens and its search only walks that folder. The
fixture turns background auto-check off and has no Wago.io key, and no tape
confirms an install, answers a question, or runs a check or update (`U`, `u`,
`C`, `c`, `x`, `y`, or `b` inside Backups). Your real LemonUp config, data and
WoW install are never read or written.

## Prerequisites

- `vhs` (tested with 0.12.1), which needs `ttyd` and downloads a headless
  Chromium on first use
- `python3`, `sqlite3`, ImageMagick 7 (`magick`)
- a Rust toolchain, unless you pass `--bin`
- a monospace font: the tapes ask for JetBrains Mono, then FiraCode Nerd Font
  Mono, then DejaVu Sans Mono

## Running

```sh
scripts/screenshots/generate.sh                      # build, write ../lemonup.org/docs/public/screens
scripts/screenshots/generate.sh --bin target/release/lemonup --out /tmp/shots
```

The default output is the sibling website checkout,
`../lemonup.org/docs/public/screens`, relative to this repo. A full run takes
about 70 seconds plus the build. Set `LEMONUP_SCREENSHOT_SANDBOX` to use
another sandbox; it must start with `/tmp/lemonup-demo` because the script
deletes it.

## Screens

| File | Tape | Shows | Used on |
| --- | --- | --- | --- |
| `dashboard.webp` | `dashboard.tape` | addon list, cursor on Deadly Boss Mods (Update ready) | home, Features |
| `light.webp` | `light.tape` | addon list in the Light theme | home, Features |
| `filter.webp` | `filter.tape` | filter box over the header with `options`, "3 of 18 shown", matches through their folders | Features |
| `details.webp` | `details.tape` | Details as a sheet (under 120 columns, FontSize 42) for HandyNotes: "Newer than source" | Features |
| `details-docked.webp` | `details-docked.tape` | Details docked on the right (about 140 columns) for Deadly Boss Mods | Features |
| `install.webp` | `install.tape` | "Install an addon" with a GitHub link and the "Install from a link?" question (never answered) | Addon Sources |
| `help.webp` | `help.tape` | `?` Help for the addon list | Keyboard Shortcuts |
| `backup.webp` | `backup.tape` | Backups of your game settings, listing the three fixture backups | Features, Configuration |
| `settings.webp` | `settings.tape` | Settings (`s`) | Configuration |
| `setup.webp` | `setup.tape` | Setup step 1, game found: Retail and WoW Forever with addon counts (`./lemonup-fresh`) | Getting Started |

## Adding a screen

1. Copy an existing tape in `tapes/`, for example `help.tape`, to
   `tapes/NAME.tape`. Keep `Source tapes/common.tape` first, and set
   `Output shots/NAME.txt` and `Screenshot shots/NAME.png` with the same NAME
   (VHS captures PNG; `generate.sh` converts it to `NAME.webp`).
   VHS cannot parse absolute paths that contain `-`, so keep paths relative.
2. Launch with `Hide`, `Type "clear && ./lemonup"`, `Enter` and a `Sleep` of
   about 4s so the startup scan settles. Send the keys for the state you
   want, then `Show`, `Sleep` and `Screenshot`.
3. Leave the app with `Hide`, the keys that close the sheet (`Escape`), then
   `Type "q"`. `Ctrl+C` quits from anywhere, including Setup.
4. Never send a key that installs, updates, checks, deletes, restores or
   backs up, and never answer a question with `y` or `enter`.
5. If the screen needs other data, change `fixture.py` and the checks in
   `generate.sh` together.
6. Run `generate.sh --out /tmp/shots` and look at the result before writing
   into the website checkout.

`generate.sh` picks up every `tapes/*.tape` except `common.tape`. The tape
named `light` runs with `theme = "light"` in the sandbox config. A tape can
change the terminal size with `Set FontSize` after `Source` (Details docks at
120 columns or wider; FontSize 32 gives about 140). It does not remove images
of screens that no longer have a tape; delete those from the website by hand.
