#!/usr/bin/env bash
# Makes each system's icon and cover, and the Default theme's "All games" icon and cover.
#
#   Emulator plugins   plugins/emulators/<plugin>/assets/icon.png (256 px) and cover.jpg
#   Default theme      plugins/themes/default/theme/images/all-games-icon.png and -cover.jpg
#
# The icons come from the SVGs in assets/system-art, each cropped to its rounded square. The
# covers come from the folder given as the first argument, one <system>-cover.png per system
# (ps5, ps4, switch, pc, steam, and nes, genesis, ps1 when there is one), and are written as
# JPEG, quality 85, at their own size. A retro system without one keeps its cover, a plain
# gradient with its name. The
# "All games" cover comes from the picture given as the second argument, cut to 16:9 from its
# left edge, where its name is.
#
# The results are kept in the repo, so a build needs no image tools; run this again after
# changing a picture. Needs ImageMagick 7 (magick) and librsvg (rsvg-convert).
#
# Usage: scripts/make-system-art.sh <covers-folder> <all-games-picture>
set -euo pipefail
covers="$(realpath "${1:?usage: $0 <covers-folder> <all-games-picture>}")"
all_games="$(realpath "${2:?usage: $0 <covers-folder> <all-games-picture>}")"
cd "$(dirname "$0")/.."

art=assets/system-art
theme=plugins/themes/default/theme/images

# A square SVG as a PNG `size` pixels wide, through librsvg: ImageMagick's own SVG reader
# leaves out filters such as feDropShadow, and with them whole layers of the icons.
icon() { rsvg-convert -w "$2" -h "$2" -o "$3" "$1"; }

# The system's art name, then its emulator plugin's folder.
# A console of the Retro plugin has its pictures in a folder of its own there.
for pair in ps5:kytyps5 ps4:shadps4 switch:lichii pc:pc steam:steam nes:retro/assets/nes genesis:retro/assets/genesis \
    atari2600:retro/assets/atari2600 ps1:ps1; do
    system="${pair%%:*}"
    case "${pair#*:}" in
        */*) out="plugins/emulators/${pair#*:}" ;;
        *) out="plugins/emulators/${pair#*:}/assets" ;;
    esac
    mkdir -p "$out"
    icon "$art/$system.svg" 256 "$out/icon.png"
    case "$system" in
        nes | genesis | atari2600 | ps1) [ -f "$covers/$system-cover.png" ] || continue ;;
    esac
    magick "$covers/$system-cover.png" -strip -quality 85 "$out/cover.jpg"
done

mkdir -p "$theme"
icon "$art/all-games.svg" 256 "$theme/all-games-icon.png"
# 16:9 at the picture's own height, from its left edge.
height="$(magick identify -format %h "$all_games")"
magick "$all_games" -gravity west -crop "$((height * 16 / 9))x$height+0+0" +repage -strip -quality 85 \
    "$theme/all-games-cover.jpg"
