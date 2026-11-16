#!/usr/bin/env bash
# Records the README demo, docs/demo/demo-loop.webp: a short loop of the real launcher, driven
# by key presses on a hidden display.
#
#   scripts/record_demo.sh                    builds the release binary, then records
#   scripts/record_demo.sh --binary <file>    records with this binary
#   scripts/record_demo.sh --keep             keeps the work folder and prints where it is
#   scripts/record_demo.sh --online           lets the launcher use the network
#
# The launcher runs on a virtual display (Xvfb) at 1920x1080. HOME and the XDG folders point
# into a new temporary folder, so it sees none of your settings, games or files. Its library is
# five made-up PC games that this script makes from the repo's own content: the titles of the
# sample catalogs in plugins/services/release-catalog/samples/, art drawn with ImageMagick and
# the theme's Inter font, and a start.sh that does nothing. It runs without network unless you
# pass --online: no downloads, no store art, no update check. Programs that would open a browser
# or a video player are stubs that do nothing.
#
# The path: Home, the Library, a game's Game Hub, back to Home, Settings, and back to Home, so
# the loop closes. It takes about 25 seconds.
#
# Linux only. Needs Xvfb (xorg-x11-server-Xvfb on Fedora, xvfb on Ubuntu and Debian), xdotool,
# ffmpeg with libx264 and libwebp, and ImageMagick 7 (magick), plus unshare (util-linux) for the
# offline run. Nothing shows on your screen and your keyboard stays yours.
#
# Check the WebP before you commit it: open it in a browser and look at each screen.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$PWD

binary=""
keep=false
online=false
while (($#)); do
    case "$1" in
        --binary) binary=${2:?--binary needs a file}; shift 2 ;;
        --keep) keep=true; shift ;;
        --online) online=true; shift ;;
        -h | --help) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1 (see --help)" >&2; exit 2 ;;
    esac
done

# The tools first, so a missing one stops the script before the long build.
missing=()
command -v Xvfb >/dev/null || missing+=("Xvfb (xorg-x11-server-Xvfb on Fedora, xvfb on Ubuntu and Debian)")
command -v xdotool >/dev/null || missing+=("xdotool")
command -v ffmpeg >/dev/null || missing+=("ffmpeg")
command -v ffprobe >/dev/null || missing+=("ffprobe (comes with ffmpeg)")
command -v magick >/dev/null || missing+=("ImageMagick 7 (magick: ImageMagick on Fedora, imagemagick on Ubuntu and Debian)")
$online || command -v unshare >/dev/null || missing+=("unshare (util-linux), or pass --online")
if ((${#missing[@]})); then
    echo "record_demo.sh needs these tools, which are missing:" >&2
    printf '  - %s\n' "${missing[@]}" >&2
    exit 1
fi
for codec in libx264 libwebp_anim; do
    ffmpeg -hide_banner -encoders 2>/dev/null | grep -q " $codec " || { echo "ffmpeg has no $codec encoder" >&2; exit 1; }
done

# The product's names, from brand.toml: `key "section" "name"`.
brand() {
    awk -v section="[$1]" -v key="$2" '
        /^\[/ { here = ($0 == section) }
        here && $1 == key { sub(/^[^=]*= *"/, ""); sub(/".*$/, ""); print; exit }
    ' brand.toml
}
bin_name=$(brand ids binary)
game_file=$(brand files game_file)
art_folder=$(brand files art_folder)
data_folder=$(brand files data_folder)
product=$(brand product name)
for value in "$bin_name" "$game_file" "$art_folder" "$data_folder" "$product"; do
    [[ -n $value ]] || { echo "brand.toml is missing a name this script reads" >&2; exit 1; }
done

if [[ -z $binary ]]; then
    cargo build --release -p launcher-app
    binary=target/release/$bin_name
fi
[[ -x $binary ]] || { echo "no launcher binary at $binary" >&2; exit 1; }
binary=$(realpath "$binary")

out=docs/demo
mkdir -p "$out"
work=$(mktemp -d "${TMPDIR:-/tmp}/launcher-demo.XXXXXX")
home=$work/home
font=$root/plugins/themes/default/theme/fonts/Inter-Bold.ttf
logo=$root/assets/$bin_name.png

xvfb_pid=""
app_pid=""
rec_pid=""
cleanup() {
    for pid in "$rec_pid" "$app_pid" "$xvfb_pid"; do
        [[ -n $pid ]] && kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
    if $keep; then
        echo "work folder kept: $work"
    else
        rm -rf "$work"
    fi
}
trap cleanup EXIT

# --- The throwaway library -------------------------------------------------------------------

mkdir -p "$home/Games/PC" "$home/Downloads" "$work/runtime" "$work/stubs"
chmod 700 "$work/runtime"

# A gradient with the logo and the title: the cover (2:3) and the background (16:9).
draw_art() { # <dir> <title> <colour from> <colour to>
    local dir=$1 title=$2 from=$3 to=$4
    magick -size 600x900 "gradient:$from-$to" \
        \( "$logo" -resize 260x260 \) -gravity center -geometry +0-120 -composite \
        -font "$font" -fill white -pointsize 52 -size 520x -background none \
        \( caption:"$title" \) -gravity south -geometry +0+110 -composite \
        -quality 90 "$dir/cover.jpg"
    magick -size 1920x1080 "gradient:$to-$from" \
        \( "$logo" -resize 520x520 -alpha set -channel A -evaluate multiply 0.25 +channel \) \
        -gravity east -geometry +160+0 -composite \
        -quality 88 "$dir/background.jpg"
}

# One PC game: <folder> <title> <genre> <release date> <colour from> <colour to> <description>.
make_game() {
    local dir=$home/Games/PC/$1
    mkdir -p "$dir/$art_folder"
    printf '#!/bin/sh\n# A demo game: it starts nothing.\nexit 0\n' >"$dir/start.sh"
    chmod +x "$dir/start.sh"
    draw_art "$dir/$art_folder" "$2" "$5" "$6"
    cat >"$dir/$game_file" <<EOF
{
  "format": 1,
  "title": "$2",
  "platform": "PC",
  "description": "$7",
  "publisher": "$product Samples",
  "genres": ["$3"],
  "release": "$4",
  "art": {
    "cover": "$art_folder/cover.jpg",
    "background": "$art_folder/background.jpg"
  },
  "starts": [{ "kind": "script", "path": "start.sh" }]
}
EOF
}

make_game sample-windows-game "Sample Windows Game" Adventure 2012-09-26 '#1d4e89' '#0b1a2e' \
    "A made-up adventure for the demo. Its title comes from the sample catalogs."
make_game sample-platformer "Sample Platformer" Platformer 2024-03-14 '#2a9d8f' '#0b2a28' \
    "A made-up platformer for the demo. Its title comes from the sample catalogs."
make_game sample-puzzle "Sample Puzzle" Puzzle 2023-11-02 '#7b2cbf' '#1d0b33' \
    "A made-up puzzle game for the demo. Its title comes from the sample catalogs."
make_game sample-quest "Sample Quest" Adventure 2025-06-20 '#c8553d' '#2e110b' \
    "A made-up quest for the demo. Its title comes from the sample catalogs."
make_game sample-racer "Sample Racer" Racing 2025-01-30 '#e9c46a' '#3a2c0b' \
    "A made-up racing game for the demo. Its title comes from the sample catalogs."

# Settings: the Welcome screen is done, no sounds, no emulator updates.
data_dir=$home/.local/share/$data_folder
mkdir -p "$data_dir"
cat >"$data_dir/config.json" <<'EOF'
{ "app": { "welcomed": true, "sounds": false, "auto_update": false } }
EOF

# Nothing outside the launcher may show: browsers, file managers and video players do nothing.
for tool in xdg-open gio kde-open gnome-open mpv vlc ffplay firefox chromium google-chrome; do
    printf '#!/bin/sh\nexit 0\n' >"$work/stubs/$tool"
    chmod +x "$work/stubs/$tool"
done

# --- The hidden display ----------------------------------------------------------------------

display=""
for n in $(seq 90 120); do
    [[ -e /tmp/.X11-unix/X$n || -e /tmp/.X$n-lock ]] || { display=:$n; break; }
done
[[ -n $display ]] || { echo "no free X display number from :90 to :120" >&2; exit 1; }
Xvfb "$display" -screen 0 1920x1080x24 +extension GLX -nolisten tcp -noreset >"$work/xvfb.log" 2>&1 &
xvfb_pid=$!
for _ in $(seq 50); do
    [[ -e /tmp/.X11-unix/X${display#:} ]] && break
    sleep 0.1
done
[[ -e /tmp/.X11-unix/X${display#:} ]] || { echo "Xvfb did not start; see $work/xvfb.log" >&2; keep=true; exit 1; }

# xdotool and ffmpeg talk to the hidden display only.
x() { DISPLAY=$display "$@"; }

# --- The launcher ----------------------------------------------------------------------------

# A clean environment: no Wayland socket, session bus or settings of yours reach the launcher.
env_vars=(
    "HOME=$home"
    "PATH=$work/stubs:/usr/local/bin:/usr/bin:/bin"
    "DISPLAY=$display"
    "XDG_CONFIG_HOME=$home/.config"
    "XDG_DATA_HOME=$home/.local/share"
    "XDG_CACHE_HOME=$home/.cache"
    "XDG_STATE_HOME=$home/.local/state"
    "XDG_RUNTIME_DIR=$work/runtime"
    "LANG=C.UTF-8"
    "LIBGL_ALWAYS_SOFTWARE=1"
    "RUST_LOG=info"
)
offline=()
$online || offline=(unshare --map-current-user --net)
(
    cd "$home"
    exec env -i "${env_vars[@]}" "${offline[@]}" "$binary" --windowed --theme default \
        --data-dir "$data_dir" --folder "games=$home/Games"
) >"$work/launcher.log" 2>&1 &
app_pid=$!

window=""
for _ in $(seq 120); do
    window=$(x xdotool search --onlyvisible --name "$product" 2>/dev/null | head -n 1 || true)
    [[ -n $window ]] && break
    kill -0 "$app_pid" 2>/dev/null || { echo "the launcher quit; see $work/launcher.log" >&2; keep=true; exit 1; }
    sleep 0.25
done
[[ -n $window ]] || { echo "no launcher window after 30 s; see $work/launcher.log" >&2; keep=true; exit 1; }

# No window manager: the window fills the screen by its size and place alone.
x xdotool windowmove --sync "$window" 0 0
x xdotool windowsize --sync "$window" 1920 1080
x xdotool windowfocus --sync "$window"

# The library, its art and the theme load.
sleep 12

# Presses each key with a short gap, then waits `pause` seconds for the screen to settle.
press() { # <pause> <key>...
    local pause=$1
    shift
    for key in "$@"; do
        x xdotool key --clearmodifiers "$key"
        sleep 0.5
    done
    sleep "$pause"
}

# --- The recording ---------------------------------------------------------------------------

raw=$work/raw.mkv
x ffmpeg -loglevel error -y -f x11grab -draw_mouse 0 -framerate 30 -video_size 1920x1080 \
    -i "$display" -t 60 -c:v libx264 -preset ultrafast -qp 0 "$raw" </dev/null &
rec_pid=$!
sleep 2.5

# Home, on "All games". Enter opens the Library.
press 3 Return
# Along the games, then Enter opens the Game Hub of the focused one.
press 0.6 Right Right
press 4 Return
# Back to Home.
press 2 Escape Escape
# L1 (Tab) along the top bar: Search, Downloads, Emulators, Themes, Settings.
press 0.3 Tab Tab Tab Tab Tab
press 3 Return
# Down the first tab of Settings.
press 1.5 Down Down Down
# Back to Home, where the loop starts.
press 2.5 Escape Escape

kill -INT "$rec_pid"
wait "$rec_pid" || true
rec_pid=""

# --- The WebP --------------------------------------------------------------------------------

duration=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$raw")
webp=$out/demo-loop.webp
# The repo's pre-commit hook takes no file over 2048 KB: the first try that fits wins.
limit=$((2000 * 1024))
for try in "1280 15 70" "1120 12 65" "960 12 60" "800 10 55"; do
    read -r width fps quality <<<"$try"
    ffmpeg -loglevel error -y -i "$raw" -vf "fps=$fps,scale=$width:-2:flags=lanczos" \
        -c:v libwebp_anim -lossless 0 -quality "$quality" -compression_level 6 -loop 0 -an "$webp"
    size=$(stat -c %s "$webp")
    echo "$webp: ${width} px wide, $fps fps, quality $quality: $((size / 1024)) KB"
    ((size <= limit)) && break
done
((size <= limit)) || { echo "$webp is still over $((limit / 1024)) KB" >&2; exit 1; }
printf '%s: %s s, %d KB\n' "$webp" "${duration%.*}" "$((size / 1024))"
echo "done: open $webp and look at each screen before you commit it"
