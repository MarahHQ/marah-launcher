#!/usr/bin/env bash
# Makes or updates the catalogs Marah Launcher bundles: assets/catalog/<name>.json, one for each
# catalog in assets/catalog-sources.json.
#
#   scripts/catalog/update-catalogs.sh [--console <name>] [--refresh] [--dry-run]
#
# --console reads only that catalog's forums, and replaces only that catalog. Without it, every
# catalog in assets/catalog-sources.json is made.
#
# Without --refresh (the full run), each forum in assets/catalog-sources.json is read again in a
# visible Chrome window: do the Cloudflare check yourself when it shows. Its new listing is
# Russian, and translate.mjs makes it English. Set STEAM=1 to add Steam art and ProtonDB ratings
# (steam.mjs) before the translation.
#
# With --refresh, no browser opens: refresh.mjs starts from each forum's topics in the published
# assets/catalog/<name>.json and updates them from the forum's feed and API, and translate.mjs
# makes the new topics English. A catalog with no file there yet is skipped.
#
# Then publish.mjs writes the catalogs, and they are copied into assets/catalog/, and into
# dist/catalog/publish/ for publish-catalog.sh. dist/catalog/ is
# the scripts' work folder, which git ignores: the published files are the source of the next
# refresh. Before anything changes, the current assets/catalog/*.json are copied to
# dist/catalog/backup/<UTC time>/. The run stops at the first forum whose listing
# is partial, and never copies a file that is not complete and English.
#
# A full run that stopped after it read every listing page keeps its listing in
# dist/catalog/<key>-topics.json.partial.json. A full run in the next day goes on from it and reads
# only the topic pages left. Delete that file to read the forum again from its first page.
#
# CONCURRENCY=4 reads up to 4 topic pages at once (default 1): about 4 times faster, but harder on
# the forum.
#
# --dry-run prints each command and copy without doing it.
#
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
sources="$root/assets/catalog-sources.json"
dist="$root/dist/catalog"
assets="$root/assets/catalog"

refresh=0
dry=0
# Topic pages read at once in the browser, 1 to 4 (list.mjs --concurrency).
concurrency=${CONCURRENCY:-1}
console=""
while [ $# -gt 0 ]; do
  case $1 in
    --refresh) refresh=1 ;;
    --dry-run) dry=1 ;;
    --console)
      [ $# -ge 2 ] || { echo "--console needs a catalog name, for example pc" >&2; exit 64; }
      console=$2
      shift
      ;;
    -h | --help)
      sed -n '2,33p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown option: $1 (use --console, --refresh, --dry-run or --help)" >&2
      exit 64
      ;;
  esac
  shift
done

# Prints a command, then runs it unless this is a dry run.
run() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  if [ "$dry" = 0 ]; then "$@"; fi
}

fail() {
  echo "update-catalogs: $*" >&2
  exit 1
}

# Checks.
command -v jq >/dev/null || fail "jq is required: install it"
command -v node >/dev/null || fail "Node 20 or later is required: install it"
node_major=$(node -p 'process.versions.node.split(".")[0]')
[ "$node_major" -ge 20 ] || fail "Node 20 or later is required: this is Node $(node -v)"
if [ ! -d "$here/node_modules" ]; then
  command -v npm >/dev/null || fail "npm is required to install the collector's packages"
  run npm --prefix "$here" install
fi
if [ "$refresh" = 0 ]; then
  node --input-type=module -e "import { findChrome } from '$here/chrome.mjs'; process.exit(findChrome() ? 0 : 1);" \
    || fail "Google Chrome or Chromium is required: install it, or set CHROME_PATH to its program"
fi

if [ -n "$console" ]; then
  jq -e --arg c "$console" 'has($c)' "$sources" >/dev/null \
    || fail "no catalog named $console in $sources: use one of $(jq -r 'keys_unsorted | join(", ")' "$sources")"
  names=$console
  forums=$(jq -r --arg c "$console" '.[$c].forums[].key' "$sources")
else
  names=$(jq -r 'keys_unsorted[]' "$sources")
  forums=$(jq -r '.[].forums[].key' "$sources")
fi
[ -n "$forums" ] || fail "no forums in $sources"

# Back up the bundled catalogs before anything changes them.
backup="$dist/backup/$(date -u +%Y%m%dT%H%M%SZ)"
echo "Backup of the current catalogs: $backup"
run mkdir -p "$backup"
for name in $names; do
  if [ -f "$assets/$name.json" ]; then run cp -p "$assets/$name.json" "$backup/$name.json"; fi
done

if [ "$refresh" = 0 ]; then
  for key in $forums; do
    echo "== $key: read the forum (a Chrome window opens; do the Cloudflare check when it shows)"
    status=0
    partial="$dist/$key-topics.json.partial.json"
    # A partial run of the last day that read every listing page left only topic pages to read:
    # go on from it.
    if [ -n "$(find "$partial" -mmin -1440 2>/dev/null)" ] && jq -e '(.pages_remaining // []) == [] and (.topics // []) != []' "$partial" >/dev/null; then
      echo "== $key: going on from $partial ($(jq '[.topics[] | select(.magnet_status != "available" or .game_info_status != "available")] | length' "$partial") topics left to read)"
      run node "$here/list.mjs" --platform "$key" --enrich-existing "$partial" --concurrency "$concurrency" || status=$?
    else
      run node "$here/list.mjs" --platform "$key" --include-magnets --concurrency "$concurrency" || status=$?
    fi
    if [ "$status" = 2 ]; then
      fail "the $key listing is partial (verification, login or an HTTP failure): see $partial. Nothing was copied into $assets. Run again to go on from it."
    elif [ "$status" != 0 ]; then
      fail "list.mjs failed for $key (exit $status). Nothing was copied into $assets."
    fi
    # The partial file is used up: the next run reads the forum again.
    if [ "$dry" = 0 ]; then rm -f "$partial"; fi
    # The new listing becomes the Russian source. translate.mjs and steam.mjs then work from it,
    # not from the source of an earlier run.
    run cp "$dist/$key-topics.json" "$dist/$key-topics.json.ru-backup.json"
    if [ "${STEAM:-0}" = 1 ]; then
      run node "$here/steam.mjs" --platform "$key"
    fi
    run node "$here/translate.mjs" --platform "$key" --from-original
  done
else
  for key in $forums; do
    name=$(jq -r --arg k "$key" 'to_entries[] | select(any(.value.forums[]; .key == $k)) | .key' "$sources")
    # A listing of an earlier run must not stand in for this one.
    if [ "$dry" = 0 ]; then rm -f "$dist/$key-topics.json"; fi
    if [ ! -f "$assets/$name.json" ]; then
      echo "== $key: skipped, no catalog $assets/$name.json yet. Run make catalog first."
      continue
    fi
    echo "== $key: refresh $name.json from the feed and the forum list"
    run node "$here/refresh.mjs" --platform "$key"
    run node "$here/translate.mjs" --platform "$key"
  done
fi

# Publish into a new folder, so a file from an earlier run is never copied.
if [ "$dry" = 0 ]; then
  out=$(mktemp -d "$dist/publish-run.XXXXXX")
  trap 'rm -rf "$out"' EXIT
else
  out="$dist/publish-run.XXXXXX"
fi
run node "$here/publish.mjs" --output "$out"

# Check every file before the first copy, so a bad file leaves assets/catalog as it was.
if [ "$dry" = 0 ]; then
  for name in $names; do
    file="$out/$name.json"
    if [ -f "$file" ] && ! jq -e '.complete == true and .translation.language == "en"' "$file" >/dev/null; then
      fail "$file is not a complete, English catalog. Nothing was copied into $assets."
    fi
  done
fi
run mkdir -p "$dist/publish" "$assets"
copied=0
for name in $names; do
  file="$out/$name.json"
  if [ "$dry" = 0 ] && [ ! -f "$file" ]; then continue; fi
  run cp "$file" "$dist/publish/$name.json"
  run cp "$file" "$assets/$name.json"
  copied=$((copied + 1))
done
[ "$copied" -gt 0 ] || fail "publish.mjs wrote no catalog"
if [ "$dry" = 0 ]; then
  for name in $names; do
    if [ -f "$assets/$name.json" ]; then
      echo "$assets/$name.json: $(jq -r .topic_count "$assets/$name.json") topics"
    fi
  done
fi
echo "The earlier files are in $backup. Build Marah Launcher again to bundle the new catalogs, and commit them: the next refresh starts from them."
