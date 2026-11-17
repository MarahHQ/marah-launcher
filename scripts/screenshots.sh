#!/usr/bin/env bash
# Takes the user-guide screenshots from the running app, with your real library. The README shows
# the demo of scripts/record_demo.sh instead.
#   scripts/screenshots.sh          builds Marah Launcher, then writes docs/screenshots/*.jpg
#
# It opens a Marah Launcher window of 1600 × 900 for about a minute, in the Default theme (the
# --theme flag doesn't save the choice), and drives it with keys. Don't touch the keyboard
# or the mouse while it runs: the keys go to the window in front. Close any other Marah Launcher
# first. Marah Launcher reads and writes its usual data folder, as when you start it yourself; the
# script presses no key that installs, deletes or restores anything.
#
# Linux only. It needs xdotool and ImageMagick's `import`, and runs Marah Launcher on X11 (XWayland
# on a Wayland desktop), where xdotool can find and drive the window.
#
# The pictures:
#   home.jpg  library.jpg  detail.jpg (Game Hub)  catalog.jpg  downloads.jpg
#   settings.jpg  backup.jpg (Settings → Backup)  backup-browser.jpg
set -euo pipefail
cd "$(dirname "$0")/.."

for tool in xdotool import; do
    command -v "$tool" >/dev/null || { echo "$tool is missing" >&2; exit 1; }
done

out=docs/screenshots
mkdir -p "$out"
cargo build --release -p launcher-app
bin=target/release/marah-launcher

# X11 even on a Wayland desktop: without WAYLAND_DISPLAY, winit opens an XWayland window.
env -u WAYLAND_DISPLAY "$bin" --windowed --theme default &
pid=$!
trap 'kill "$pid" 2>/dev/null || true' EXIT

window=$(xdotool search --sync --onlyvisible --pid "$pid" | head -n 1)
xdotool windowsize --sync "$window" 1600 900
xdotool windowactivate --sync "$window"

# The library, its art and the catalogs load.
sleep 15

# Presses each key, then waits for the screen to settle.
press() {
    for key in "$@"; do
        xdotool key --clearmodifiers "$key"
        sleep 0.4
    done
    sleep 1.5
}

# X hands `import` the frame the window showed last, which can be the screen before: a resize by
# a pixel and back makes the window draw twice, so the frame it shows is the one on screen.
shoot() {
    sleep 2
    xdotool windowsize --sync "$window" 1601 900
    sleep 0.5
    xdotool windowsize --sync "$window" 1600 900
    sleep 1
    import -window "$window" -quality 90 "$out/$1.jpg"
    echo "$out/$1.jpg"
}

shoot home
# Enter on the first entry, All games: the Library.
press Return
sleep 3
shoot library
# Enter on the first game: its Game Hub. Its art and details come from the network.
press Return
sleep 3
shoot detail
# Back to Home, from where each screen opens.
press Escape Escape
# Ctrl+D opens Downloads.
press ctrl+d
sleep 3
shoot downloads
press Escape
# L1 (Tab) on Home goes to the top bar's first item, Search: the Catalog.
press Tab Return
sleep 2
shoot catalog
press Escape Escape
# Settings is the top bar's last item: Search, Downloads, Emulators, Themes, Settings.
press Tab Tab Tab Tab Tab Return
shoot settings
# Up from the first row to the rail of sections, down it to Backup (Games, Downloads, Launcher,
# Updates, Backup: no System section in a window), and Right into its rows.
press Up Down Down Down Down Right
shoot backup
# Down to "Restore from another folder": below Back up now, Include large items and the
# backup folder. Enter opens the browser.
press Down Down Down Return
# The places, then down from Home past Downloads, Desktop and Documents to the first drive,
# which opens: no folder of yours shows in the picture.
press Left Down Down Down Down Return
sleep 1
shoot backup-browser
# The places, where Escape closes it.
press Left Escape

echo "done: check each picture before you commit it"
