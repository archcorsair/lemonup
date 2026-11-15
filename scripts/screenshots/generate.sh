#!/usr/bin/env bash
# Regenerates the LemonUp website screenshots (lossless WebP) with VHS, fully offline.
#
#   scripts/screenshots/generate.sh [--out DIR] [--bin PATH]
#
# See scripts/screenshots/README.md.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

OUT_DIR=""
BIN=""
# Short, neutral path: it is shown in the dashboard header. VHS cannot parse
# absolute Output/Screenshot paths with '-' segments, so tapes use relative
# paths and vhs runs from inside the sandbox.
SANDBOX="${LEMONUP_SCREENSHOT_SANDBOX:-/tmp/lemonup-demo}"
# Window corner radius in output pixels (the images are 2x).
CORNER_RADIUS=24

usage() {
    sed -n '2,6p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --out) OUT_DIR="${2:?--out needs a directory}"; shift 2 ;;
        --bin) BIN="${2:?--bin needs a path}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

die() { echo "error: $*" >&2; exit 1; }
log() { printf '\033[1;33m==>\033[0m %s\n' "$*"; }

for tool in vhs ttyd python3 sqlite3 magick; do
    command -v "$tool" >/dev/null 2>&1 || die "'$tool' is required but not on PATH (see scripts/screenshots/README.md)"
done

START=$(date +%s)

# The sandbox is wiped on every run, so refuse anything that is not clearly ours.
SANDBOX="$(realpath -m -- "$SANDBOX")"
case "$SANDBOX" in
    /tmp/lemonup-demo | /tmp/lemonup-demo/*) ;;
    *) die "LEMONUP_SCREENSHOT_SANDBOX must resolve to /tmp/lemonup-demo or below (got: $SANDBOX)" ;;
esac

if [[ -z "$OUT_DIR" ]]; then
    OUT_DIR="$REPO_ROOT/../lemonup.org/docs/public/screens"
fi
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

if [[ -z "$BIN" ]]; then
    log "building release binary"
    (cd "$REPO_ROOT" && cargo build --release -p lemonup-app --bin lemonup)
    BIN="$REPO_ROOT/target/release/lemonup"
fi
[[ -x "$BIN" ]] || die "binary not found or not executable: $BIN"
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"

log "creating fixture sandbox at $SANDBOX"
rm -rf "$SANDBOX"
mkdir -p "$SANDBOX"
python3 "$SCRIPT_DIR/fixture.py" "$SANDBOX"

ADDONS_DIR="$SANDBOX/World of Warcraft/_retail_/Interface/AddOns"
STATE_DB="$SANDBOX/xdg/data/lemonup/state.sqlite"
CONFIG_TOML="$SANDBOX/home/.config/lemonup/config.toml"

# Every LemonUp run goes through this wrapper: isolated HOME and XDG dirs, and
# a dead proxy so any accidental HTTP request fails instead of leaving the box.
cat > "$SANDBOX/lemonup" <<EOF
#!/usr/bin/env bash
export HOME="$SANDBOX/home"
export XDG_CONFIG_HOME="$SANDBOX/home/.config"
export XDG_DATA_HOME="$SANDBOX/xdg/data"
export XDG_CACHE_HOME="$SANDBOX/xdg/cache"
export XDG_STATE_HOME="$SANDBOX/xdg/state"
for var in HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy; do
    export "\$var=http://127.0.0.1:9"
done
export NO_PROXY= no_proxy=
unset LEMONUP_WAGO_API_KEY WAGO_API_KEY GITHUB_TOKEN GH_TOKEN
export RUST_LOG=off LANG=C.UTF-8 COLORTERM=truecolor
exec "$BIN" "\$@"
EOF
chmod +x "$SANDBOX/lemonup"

log "migrating the v1 fixture (lemonup sync, local only)"
sync_out="$(cd "$SANDBOX" && ./lemonup sync 2>&1)"
echo "$sync_out" | sed 's/^/    /'
grep -q "Migrated LemonUp v1 settings | 14 addons imported, 0 skipped" <<<"$sync_out" \
    || die "v1 migration did not import the 14 tracked addons"

log "verifying migrated state"
check() {
    local label="$1" sql="$2" want="$3" got
    got="$(sqlite3 "$STATE_DB" "$sql")"
    [[ "$got" == "$want" ]] || die "$label: expected '$want', got '$got'"
    printf '    %-34s %s\n' "$label" "$got"
}
check "addons in state"            "SELECT count(*) FROM addons;" "18"
check "tracked by source"          "SELECT group_concat(source || '=' || n, ' ') FROM (SELECT source, count(*) n FROM addons GROUP BY source ORDER BY source);" \
                                   "github=3 manual=4 tukui=1 wago=6 wowinterface=4"
# Remote differs from installed: four updates plus HandyNotes, whose stored
# remote is older than the installed version ("older remote" on the dashboard).
check "remote differs"             "SELECT group_concat(folder, ' ') FROM (SELECT folder FROM addons WHERE source <> 'manual' AND remote_version <> coalesce(git_commit, version) ORDER BY folder);" \
                                   "Auctionator DBM-Core HandyNotes Pawn WeakAuras"
check "remote older (HandyNotes)"  "SELECT version || ' > ' || remote_version FROM addons WHERE folder = 'HandyNotes';" "v1.6.15 > v1.6.14"
# Ownership exactly as the migration left it: only rows that listed owned
# folders in v1 are managed. Nothing in this script edits the state database.
check "ownership as migrated"      "SELECT group_concat(ownership_source || '=' || n, ' ') FROM (SELECT ownership_source, count(*) n FROM addons GROUP BY ownership_source ORDER BY ownership_source);" \
                                   "managed=6 none=12"
grep -qF "addon_dir = \"$ADDONS_DIR\"" "$CONFIG_TOML" || die "config.toml addon_dir is not the fixture AddOns path"
grep -q '^auto_check_enabled = false' "$CONFIG_TOML" || die "auto-check must be off"
grep -q '^wago_api_key' "$CONFIG_TOML" && die "fixture must not carry a Wago API key"
backups=$(find "$SANDBOX/xdg/data/lemonup/backups/wtf" -name 'WTF-*.zip' | wc -l)
[[ "$backups" -eq 3 ]] || die "expected 3 WTF backups, found $backups"

log "recording tapes"
mkdir -p "$SANDBOX/tapes" "$SANDBOX/shots"
cp "$SCRIPT_DIR"/tapes/*.tape "$SANDBOX/tapes/"

set_theme() {
    sed -i "s/^theme = .*/theme = \"$1\"/" "$CONFIG_TOML"
}

shots=()
for tape in "$SANDBOX"/tapes/*.tape; do
    name="$(basename "$tape" .tape)"
    [[ "$name" == "common" ]] && continue
    if [[ "$name" == "light" ]]; then set_theme light; else set_theme dark; fi
    printf '    %-12s' "$name"
    t0=$(date +%s)
    (cd "$SANDBOX" && vhs -q "tapes/$name.tape") || die "vhs failed for $name"
    [[ -s "$SANDBOX/shots/$name.png" ]] || die "$name.tape produced no shots/$name.png"
    echo " $(( $(date +%s) - t0 ))s"
    shots+=("$name")
done
set_theme dark

log "checking captured frames for personal data"
# VHS also writes each tape's terminal text (shots/NAME.txt). Nothing on screen
# may mention the real home directory or user.
real_home="$(getent passwd "$(id -u)" | cut -d: -f6)"
if grep -rlF -e "$real_home" -e "$(id -un)" "$SANDBOX"/shots/*.txt; then
    die "a captured frame mentions the real home directory or user name"
fi

log "post-processing into $OUT_DIR"
for name in "${shots[@]}"; do
    # Round the window corners with real transparency (VHS would fill them
    # with an opaque colour), keep the 2x size, -strip metadata, and encode
    # lossless WebP (about 44% smaller than optimised PNG, pixel-identical).
    magick "$SANDBOX/shots/$name.png" \
        \( +clone -alpha off -fill black -colorize 100 -fill white \
           -draw "roundrectangle 0,0,%[fx:w-1],%[fx:h-1],$CORNER_RADIUS,$CORNER_RADIUS" \) \
        -alpha off -compose CopyOpacity -composite \
        -strip -define webp:lossless=true \
        "$OUT_DIR/$name.webp"
    # Earlier versions of this script wrote PNGs; remove only those names.
    rm -f "$OUT_DIR/$name.png"
    printf '    %-16s %s  %s bytes\n' "$name.webp" \
        "$(magick identify -format '%wx%h' "$OUT_DIR/$name.webp")" "$(stat -c%s "$OUT_DIR/$name.webp")"
done

log "done in $(( $(date +%s) - START ))s"
