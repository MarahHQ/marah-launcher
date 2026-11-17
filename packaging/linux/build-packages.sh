#!/usr/bin/env bash
# Builds the deb, rpm and Arch packages from the Linux folder scripts/package.sh made, with nfpm
# (NFPM, default `nfpm` on PATH), into a folder:
#   packaging/linux/build-packages.sh dist/marah-launcher-linux-x86_64 0.1.0 dist/packages [deb rpm archlinux]
# Run it from the repository root.
set -euo pipefail
folder="${1:?usage: $0 <release folder> <version> <output folder> [formats…]}"
version="${2:?version}"
out="${3:?output folder}"
shift 3
formats=("$@")
[ ${#formats[@]} -gt 0 ] || formats=(deb rpm archlinux)
nfpm="${NFPM:-nfpm}"
mkdir -p "$out"

# The packages take libmpv from the system (the depends in nfpm.yaml): bundled, it and its
# libraries make each package over 100 MB, more than the package repository holds. The trailer
# player finds the system's libmpv.so.2 when lib/ has none. So lib/ keeps libarchive and the
# libraries it needs, followed through their NEEDED entries, and nothing else.
staged="$(mktemp -d)/$(basename "$folder")"
cp -a "$folder" "$staged"
needed() { readelf -d "$1" 2>/dev/null | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'; }
keep=()
queue=(libarchive.so.13)
while [ ${#queue[@]} -gt 0 ]; do
    lib="${queue[0]}"
    queue=("${queue[@]:1}")
    [ -e "$folder/lib/$lib" ] || continue
    case " ${keep[*]} " in *" $lib "*) continue ;; esac
    keep+=("$lib")
    while read -r dep; do queue+=("$dep"); done < <(needed "$folder/lib/$lib")
done
for file in "$staged"/lib/*; do
    case " ${keep[*]} " in *" $(basename "$file") "*) ;; *) rm -f "$file" ;; esac
done
# A library kept under its soname may be a link to a versioned file: keep what links point to.
for lib in "${keep[@]}"; do
    target="$(readlink "$folder/lib/$lib" || true)"
    [ -z "$target" ] || cp -a "$folder/lib/$target" "$staged/lib/"
done
folder="$staged"
marker="$(mktemp -d)/packaged"
for format in "${formats[@]}"; do
    case "$format" in
        deb | rpm) echo "$format" > "$marker" ;;
        archlinux) echo arch > "$marker" ;;
        *) echo "unknown format $format" >&2; exit 1 ;;
    esac
    FOLDER="$folder" VERSION="$version" PACKAGED="$marker" \
        "$nfpm" package -f packaging/linux/nfpm.yaml -p "$format" -t "$out"
done
ls -l "$out"
