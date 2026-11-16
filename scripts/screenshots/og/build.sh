#!/usr/bin/env bash
# Builds the lemonup.org link preview card (og:image) from the website's
# dashboard screenshot: crops the logo and the addon list, lays them out in
# card.html and renders a 1200x630 JPEG with a headless Chromium.
#
# Usage: scripts/screenshots/og/build.sh [--site ../lemonup.org] [--chrome /path/to/chrome]
# Needs ImageMagick 7 (`magick`) and a Chromium-based browser.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
site="$here/../../../../lemonup.org"
chrome="${CHROME:-}"
while [ $# -gt 0 ]; do
    case "$1" in
        --site) site="$2"; shift 2 ;;
        --chrome) chrome="$2"; shift 2 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

if [ -z "$chrome" ]; then
    for candidate in chromium chromium-browser google-chrome google-chrome-stable chrome-headless-shell; do
        if command -v "$candidate" >/dev/null 2>&1; then chrome="$(command -v "$candidate")"; break; fi
    done
fi
if [ -z "$chrome" ]; then
    chrome="$(ls -d "$HOME"/.cache/ms-playwright/chromium_headless_shell-*/chrome-headless-shell-linux64/chrome-headless-shell 2>/dev/null | tail -1 || true)"
fi
[ -x "$chrome" ] || { echo "no Chromium found; pass --chrome or set CHROME" >&2; exit 1; }

screenshot="$site/docs/public/screens/dashboard.webp"
[ -f "$screenshot" ] || { echo "missing $screenshot; run generate.sh first" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cp "$here/card.html" "$work/card.html"
# The screenshot is 2880x1620 (a 2x image of a ~140x36 terminal).
# Logo: the block LEMONUP in the header, with the terminal background made transparent.
magick "$screenshot" -crop 640x100+48+108 +repage -fuzz 4% -transparent "srgb(35,38,58)" -trim +repage "$work/logo.png"
# List: the column header and the first rows, Addon, Status and Version columns only.
magick "$screenshot" -crop 1780x530+40+232 +repage "$work/list.png"

"$chrome" --headless --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
    --window-size=1200,630 --screenshot="$work/card.png" "file://$work/card.html" >/dev/null 2>&1
magick "$work/card.png" -quality 88 "$site/docs/public/og-card.jpg"
echo "wrote $site/docs/public/og-card.jpg"
