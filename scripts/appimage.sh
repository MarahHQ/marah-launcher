#!/usr/bin/env bash
# Makes dist/Marah-Launcher-<arch>.AppImage from the Linux package scripts/package.sh made: one file
# that runs on most distributions. linuxdeploy (LINUXDEPLOY, the AppImage of a pinned
# release) bundles libmpv and the libraries it needs, deciding itself which the system has.
# The AppImage carries its update information (gh-releases-zsync, the release repo in
# brand.toml), and zsyncmake (ZSYNCMAKE, default `zsyncmake`, the zsync package) writes its
# control file, dist/Marah-Launcher-<arch>.AppImage.zsync, for updates that download only what
# changed.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${LINUXDEPLOY:?set LINUXDEPLOY to the linuxdeploy AppImage}"
zsyncmake="${ZSYNCMAKE:-zsyncmake}"
command -v "$zsyncmake" > /dev/null || { echo "no $zsyncmake: install zsync" >&2; exit 1; }
# The release repo, owner/name, from brand.toml's [ids] update_repo.
repo="$(awk '/^\[/ { here = ($0 == "[ids]") } here && $1 == "update_repo" { sub(/^[^=]*= *"/, ""); sub(/".*$/, ""); print; exit }' brand.toml)"
[[ $repo == */* ]] || { echo "brand.toml has no [ids] update_repo" >&2; exit 1; }
package="$(find dist -maxdepth 1 -type d -name 'marah-launcher-linux-*' | head -n 1)"
[ -n "$package" ] || { echo "no Linux package in dist/: run scripts/package.sh first" >&2; exit 1; }
arch="${package##*-}"

appdir="$(mktemp -d)/MarahLauncher.AppDir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/licenses/marah-launcher" "$appdir/usr/share/doc/marah-launcher"
cp "$package/marah-launcher" "$appdir/usr/bin/"
mkdir -p "$appdir/usr/bin/extensions"
cp -R "$package/licenses/." "$appdir/usr/share/licenses/marah-launcher/"
cp "$package/LICENSE" "$appdir/usr/share/licenses/marah-launcher/"
cp "$package/README.md" "$appdir/usr/share/doc/marah-launcher/"
# awk reads to the end: stopping early would end ldconfig with SIGPIPE, which pipefail fails on.
libmpv="$(ldconfig -p | awk '/libmpv\.so\.[0-9]/ && !found {print $NF; found = 1}')"

# libxkbcommon comes from the system, never from the AppImage: it reads the system's keyboard
# files (/usr/share/X11/locale/*/Compose), and an older copy crashed on Fedora 45's at start
# (SIGSEGV in libxkbcommon.so.0, 0.4.0's AppImage).
# Runners have no FUSE: run linuxdeploy from its extracted files.
export APPIMAGE_EXTRACT_AND_RUN=1
appimage="Marah-Launcher-$arch.AppImage"
# The standard AppImage update information: the latest GitHub release's control file. The
# plugin passes it to appimagetool, which puts it in the AppImage's .upd_info section.
export LDAI_UPDATE_INFORMATION="gh-releases-zsync|${repo%%/*}|${repo#*/}|latest|$appimage.zsync"
OUTPUT="dist/$appimage" "$LINUXDEPLOY" \
    --appdir "$appdir" \
    --executable "$appdir/usr/bin/marah-launcher" \
    --library "$libmpv" \
    --exclude-library 'libxkbcommon*' \
    --desktop-file "$package/marah-launcher.desktop" \
    --icon-file "$package/marah-launcher.png" \
    --output appimage
# appimagetool writes a control file of its own when it finds zsyncmake. This one, from the
# finished file, replaces it or makes it when it didn't. Its URL is the AppImage's name, so the
# AppImage is beside the control file.
"$zsyncmake" -u "$appimage" -o "dist/$appimage.zsync" "dist/$appimage"
echo "made dist/$appimage and dist/$appimage.zsync"
