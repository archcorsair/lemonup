# Website screenshots

`generate.sh` renders the LemonUp screenshots used on lemonup.org. It runs
the real release binary against a fake, offline WoW install and records each
screen with [VHS](https://github.com/charmbracelet/vhs), so the images can be
regenerated after any UI change.

## What it does

1. Builds `target/release/lemonup` (skipped with `--bin`).
2. Wipes and recreates the sandbox at `/tmp/lemonup-demo` (the path shown in
   the dashboard header). `fixture.py` writes:
   - `World of Warcraft/` with `Data/`, `.build.info` and `_retail_/Wow.exe`,
     so it passes LemonUp's install validation
   - `_retail_/Interface/AddOns/` with 35 addon folders (18 dashboard rows)
     and a small `_retail_/WTF/`
   - a LemonUp v1 `config.json` and `lemonup.db` under the sandbox `HOME`,
     tracking 14 addons from GitHub, Wago, TukUI and WoWInterface, four of
     them with a newer stored remote version (shown as updates, no network)
     and one (HandyNotes) whose stored remote is older ("older remote")
   - three WTF backup archives in the v2 backup folder
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

Every LemonUp run goes through a wrapper script (`/tmp/lemonup-demo/lemonup`)
that sets `HOME` and the `XDG_*` directories inside the sandbox and points
the HTTP proxy variables at a closed port, so an accidental request fails. The
fixture turns background auto-check off and has no Wago API key, and no tape
confirms an install or runs check or update. Your real LemonUp config, data and
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

| File | Tape | Shows |
| --- | --- | --- |
| `dashboard.webp` | `dashboard.tape` | dashboard, cursor on Deadly Boss Mods with an update pending |
| `filter.webp` | `filter.tape` | `/` filter `options`, "3 of 18 shown", matches through owned folders |
| `inspect.webp` | `inspect.tape` | inspect overlay for HandyNotes: "Remote older" (installed v1.6.15, remote v1.6.14) |
| `install.webp` | `install.tape` | install overlay with a GitHub URL waiting for confirmation (never confirmed) |
| `help.webp` | `help.tape` | `?` help overlay |
| `backup.webp` | `backup.tape` | backup overlay listing the three fixture archives |
| `config.webp` | `config.tape` | config overlay |
| `light.webp` | `light.tape` | dashboard in the light theme |

## Adding a screen

1. Copy an existing tape in `tapes/`, for example `help.tape`, to
   `tapes/NAME.tape`. Keep `Source tapes/common.tape` first, and set
   `Output shots/NAME.txt` and `Screenshot shots/NAME.png` with the same NAME
   (VHS captures PNG; `generate.sh` converts it to `NAME.webp`).
   VHS cannot parse absolute paths that contain `-`, so keep paths relative.
2. Launch with `Hide`, `Type "clear && ./lemonup"`, `Enter` and a `Sleep` of
   about 4s so the startup scan settles. Send the keys for the state you
   want, then `Show`, `Sleep` and `Screenshot`.
3. Leave the app with `Hide`, the keys that close the overlay, then `Type "q"`.
4. Never send a key that installs, updates, checks, deletes or restores.
5. If the screen needs other data, change `fixture.py` and the checks in
   `generate.sh` together.
6. Run `generate.sh --out /tmp/shots` and look at the result before writing
   into the website checkout.

`generate.sh` picks up every `tapes/*.tape` except `common.tape`. The tape
named `light` runs with `theme = "light"` in the sandbox config.
