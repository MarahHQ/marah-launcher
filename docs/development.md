# Development

This page is for contributors: how to build Marah Launcher, run it, test it, package it and release it.
Run every command from the repository root. How the code is laid out is in
[architecture.md](architecture.md).

## The short version

```sh
make check            # format check, clippy (warnings are errors) and every test, as CI runs them
make run-windowed     # the launcher in a normal window
```

Run `make check` before you open a pull request. The pull request template asks for it.

## Toolchain

- **Rust 1.99 or newer**, stable channel. `rust-version` in `Cargo.toml` sets the minimum.
  `rust-toolchain.toml` asks for `stable` with `clippy` and `rustfmt`, so rustup installs both
  on the first `cargo` call in the repository.
- **Format**: `rustfmt.toml` sets `max_width = 120` and `use_small_heuristics = "Max"`.
- **Lints**: the workspace denies `unsafe_code` and warns on `clippy::all`
  (`[workspace.lints]` in `Cargo.toml`). `make lint`, `make check` and CI turn the warnings
  into errors.
- **Bash** runs `scripts/` and **make** runs the `Makefile`.
- **cargo-nextest** (optional) runs each test in its own process. `scripts/check.sh` uses it
  when it is installed and falls back to `cargo test` when it is not. CI pins version 0.9.149.
  Its settings are in `.config/nextest.toml`.

Optional tools:

| Tool | For | Install |
| --- | --- | --- |
| cargo-deny | `make deny`: advisories, licences and sources (`deny.toml`) | `cargo install --locked cargo-deny` |
| cargo-about | `THIRD-PARTY.md` in a package (`about.toml`, `about.hbs`) | `cargo install --locked cargo-about --version 0.9.2 --features cli` |
| pre-commit | The git hooks (`.pre-commit-config.yaml`) | see [pre-commit.com](https://pre-commit.com) |
| Node.js | The catalog collector in `scripts/catalog/` | see [scripts/catalog/README.md](../scripts/catalog/README.md) |

`third_party/oozextract` is outside the workspace (`exclude` in `Cargo.toml`). It is not
formatted or linted with the rest. See [third-party-notices.md](third-party-notices.md).

## System packages

On Linux the build needs a C toolchain, `pkg-config`, and the development files for
fontconfig and xkbcommon (Slint), udev (gilrs, for controllers) and ALSA (rodio, for sounds).
CI installs them on Ubuntu (`.github/actions/setup/action.yml`):

```sh
sudo apt-get install libfontconfig1-dev libxkbcommon-dev libudev-dev libasound2-dev
```

<!-- TODO: unverified: the Fedora names of the build packages. Only the Ubuntu names are in the
repository (.github/actions/setup/action.yml). -->

To package on Linux you also need what the two bundle scripts list in their headers
(`scripts/bundle-libmpv.sh`, `scripts/bundle-libarchive.sh`). Both scripts work on Ubuntu and
on Fedora:

| | Ubuntu and Debian | Fedora |
| --- | --- | --- |
| libmpv | `libmpv-dev mpv patchelf` | `mpv-libs patchelf` |
| libarchive | `cmake patchelf liblzma-dev libzstd-dev libbz2-dev liblz4-dev zlib1g-dev` | `cmake patchelf xz-devel libzstd-devel bzip2-devel lz4-devel zlib-ng-compat-devel` |

The icons need `rsvg-convert` (librsvg) to make them again after a logo change, with
`scripts/make-icons.sh`: `librsvg2-bin` on Ubuntu and Debian, `librsvg2-tools` on Fedora. The
`.icns` for macOS needs macOS's `iconutil`, or `png2icns` elsewhere: `icnsutils` on Ubuntu and
Debian, `libicns-utils` on Fedora.

The README demo (`scripts/record_demo.sh`) needs four more tools:

| Tool | For | Ubuntu and Debian | Fedora |
| --- | --- | --- | --- |
| Xvfb | The hidden display the launcher runs on | `xvfb` | `xorg-x11-server-Xvfb` |
| xdotool | The key presses that drive the launcher | `xdotool` | `xdotool` |
| ffmpeg | The recording, and the WebP made from it. It needs libx264 and libwebp. | `ffmpeg` | `ffmpeg` |
| ImageMagick (`magick`) | The art of the sample games | `imagemagick` | `ImageMagick` |

The script also runs the launcher with no network through `unshare` (util-linux), which most
systems have already.

The AppImage also needs [linuxdeploy](https://github.com/linuxdeploy/linuxdeploy), named by
`LINUXDEPLOY`.

- **macOS**: to package, `brew install mpv dylibbundler cmake zstd lz4` (the `libmpv` and
  `libarchive` actions in `.github/actions/`). For the icons, `brew install librsvg`.
- **Windows**: `crates/app/build.rs` puts the icon and the version info into `marah-launcher.exe`
  with the Windows resource compiler. Without it, a local build warns and goes on. With `CI`
  set, the build fails. To package, you need Bash (Git's), 7-Zip, an unpacked shinchiro
  mpv-dev build named by `LIBMPV_DIR`, and MSYS2's libarchive DLLs named by `LIBARCHIVE_DIR`.
  The `libmpv` and `libarchive` actions show how CI fills both.

<!-- TODO: unverified: which Visual Studio or Xcode components a build needs. The repository
names none. -->

The packages Marah Launcher uses at run time (gamescope, xdotool, libmpv, Vulkan and more) are in
[user-guide.md](user-guide.md).

## Build profiles

`[profile.dev]` in `Cargo.toml` builds the workspace's own crates at `opt-level = 1`, so a
change rebuilds fast. `[profile.dev.package."*"]` builds every dependency at `opt-level = 2`
with `debug = "line-tables-only"`. Image decoding, JSON reading and Slint then run near release
speed under `make run` and in the tests. The first build after `cargo clean` takes longer for
it. `make build` and `make package` use Cargo's release profile.

## Make targets

`make` alone prints this list (`make help`).

| Target | What it runs |
| --- | --- |
| `make check` | `scripts/check.sh`: `cargo fmt --all --check`, clippy with `-D warnings`, and every test |
| `make fmt` | `cargo fmt --all` |
| `make fmt-check` | `scripts/check.sh fmt`: `cargo fmt --all --check` |
| `make lint` | `scripts/check.sh lint`: the format check, and clippy with `-D warnings` |
| `make test` | `scripts/check.sh test`: every test, with nextest when it's installed |
| `make run` | `cargo run -p launcher-app` (full screen) |
| `make run-windowed` | `cargo run -p launcher-app -- --windowed` |
| `make run-session` | Marah Launcher in gamescope with `MARAH_SESSION=1`, as the Marah Launcher session runs it. Needs gamescope. `GAMESCOPE_ARGS` sets gamescope's options (default `-f -e --xwayland-count 1`). |
| `make run-session-windowed` | `MARAH_SESSION=1` in a normal window, without gamescope |
| `make build` | `cargo build -p launcher-app --release` |
| `make package` | `scripts/package.sh`: a release build in `dist/marah-launcher-<os>-<arch>/` |
| `make linux-packages` | `make package`, then the deb, rpm and Arch packages in `dist/packages/`, checked. `NFPM` picks the nfpm (default `scripts/nfpm.sh`). |
| `make appimage` | `make package`, then `scripts/appimage.sh`. Needs `LINUXDEPLOY`. |
| `make test-rpm` | `scripts/test-rpm.sh`: an rpm of this checkout in `dist/test-rpm/`, for this PC only |
| `make deny` | `cargo deny check` |
| `make hooks-install` | Installs the git hooks for commit and push |
| `make hooks-run` | Runs the git hooks on every file |
| `make catalog` | Reads the forums again (Chrome opens) into `assets/catalog/`. `CONSOLE=pc` does one catalog. |
| `make catalog-refresh` | Updates `assets/catalog/` from the forums' feed and API, without a browser |
| `make clean` | `cargo clean`, then removes `dist/` and `target/` |

`make clean` also removes the files git tracks in `dist/catalog/`. Run `git checkout -- dist`
to get them back.

The Marah OS image, stick and VM are built in [MarahHQ/marah-os](https://github.com/MarahHQ/marah-os). The
`catalog` targets are in [scripts/catalog/README.md](../scripts/catalog/README.md).

## Running

```sh
make run                                   # full screen
make run-windowed                          # a normal window
cargo run -p launcher-app -- --help           # every option
cargo run -p launcher-app -- --windowed --dev --extensions ~/marah-launcher-themes
```

The options are the ones `marah-launcher --help` prints (`USAGE` in `crates/app/src/args.rs`):

| Option | What it does |
| --- | --- |
| `--theme <id>` | Shows this theme now. Not saved. |
| `--safe-mode` | Loads the built-in plugins only, and shows the fallback theme only. |
| `--dev` | Reloads the theme when one of its files changes. A broken edit logs an error and keeps the last good version on screen. |
| `--windowed` | Opens in a normal window for this run. Full screen is the default. |
| `--monitor <name>` | Opens on this monitor for this run, such as `DP-2`. A screen picked in Settings wins. |
| `--data-dir <path>` | Where settings and plugin data live. |
| `--extensions <path>` | The folder of third-party plugins and themes. Default: `<data-dir>/extensions`. |
| `--folder <name>=<path>` | Maps a named host folder, such as `games=/srv/games`. It is saved. Repeat it for more folders under one name. |
| `--supports <feature>` | Exits 0 when this launcher has the feature (`gamescope`, `system-settings`, `os-install`), else 1. |
| `--version` | Prints `marah-launcher <version>` on one line. |
| `--help` | Prints the options. |

Marah Launcher also reads the `extensions/` folder beside the binary (in `Contents/Resources` in a macOS
app). `--dev` reloads only a theme that loads from a folder. The built-in themes, Default and
Classic, are compiled into the binary, so an edit to them needs a new build.

`main.rs` also has `--probe-controllers` and `--probe-gpu`. Marah Launcher runs itself with them in a
child process for the "This PC" checks. They are not in `--help`.

The default data folder is `$XDG_DATA_HOME/marah-launcher` (`~/.local/share/marah-launcher`) on Linux,
`~/Library/Application Support/marah-launcher` on macOS and `%APPDATA%\marah-launcher` on Windows
(`default_data_dir` in `args.rs`). For a clean start, point `--data-dir` at an empty folder.

`MARAH_SCALE=<n>` draws everything at that size for this run, from 0.8 to 1.4. It overrides
"Size of everything" in Settings: `MARAH_SCALE=1.4 make run-windowed`.

### Session switches

Three variables make Marah Launcher act as it does in the Marah Launcher session or on Marah OS, on any Linux
desktop. Each counts only when its value is `1` (`Place::detect` in
`crates/core/src/system.rs`):

| Variable | What shows |
| --- | --- |
| `MARAH_SESSION=1` | Marah Launcher as the whole screen: the network, sound and Bluetooth buttons in the top bar, Settings → System, Log out, Restart and Shut down |
| `MARAH_OS=1` | Also Marah OS: Log out goes away |
| `MARAH_USB=1` | Also the Marah OS USB stick: "Welcome to Marah OS" and the install screens |

The controls act on this computer through `nmcli`, `bluetoothctl`, `pactl` and `systemctl`.
Joining a Wi-Fi network, pairing, the volume, Restart and Shut down are real. System updates
and the install need Marah OS: elsewhere they fail and show their error. See
[marah-os.md](marah-os.md).

`make run-session` does not start Marah Launcher again when it ends and has no fallback to cage. The
real session script, `packaging/linux/marah-session`, does both.

### Quitting

Ctrl-C, SIGTERM and SIGHUP close Marah Launcher as Quit does (`quit_on_signal` in
`crates/app/src/platform.rs`). The core gets 10 seconds to shut down. A second signal
quits at once with exit code 130.

## Logging

Marah Launcher logs to stderr through `tracing`. The level is `info` unless `RUST_LOG` sets another
filter (`EnvFilter` in `main.rs`). Targets are module paths. The binary's crate is `marah-launcher`, and
each library crate goes by its name with underscores, such as `launcher_core` or `launcher_ui_host`.
What a plugin logs through its context goes to `launcher_core::facades`, with the plugin's id in
the field `plugin_id`.

```sh
RUST_LOG=debug make run                             # everything
RUST_LOG=info,launcher_core::facades=debug make run    # the plugins' own debug lines
```

The Marah Launcher session writes Marah Launcher's output to `~/.cache/marah-session.log` (`MARAH_LOG` in
`packaging/linux/session.conf`).

## Tests

```sh
make test                                             # every test (scripts/check.sh test)
cargo nextest run --workspace                         # every test, each in its own process
cargo test -p launcher-tests --test seams themes::       # one area: a module of tests/tests/seams/
cd scripts/catalog && npm test                        # the catalog collector's tests
```

The tests need no window, GPU, controller or sound device. They use Slint's testing backend
and the fakes in `crates/testkit` (`FakeHost`, `FakeEmulator`, `FakeSystem`,
`ThemeDriver` and more).

### The seams

The `launcher-tests` crate (`tests/`) holds the seam tests: the core's app API, the capability
contracts, and the composed host (core, theme host and a theme). They are one test binary,
`seams` (`tests/tests/seams/main.rs`, one module per area), so the shared code in
`tests/tests/seams/common/` builds and links once. `tests/tests/spans.rs` is its own binary,
because `tracing` caches per-callsite interest across threads.

`every_theme.rs` runs one suite against each built-in theme (Classic and Default). A new
built-in theme goes into its `THEMES` list.

Unit tests sit beside the code in each crate. `launcher-app`'s desktop tests
(`crates/app/tests/desktop.rs`) have their own harness and show real windows.
`.config/nextest.toml` leaves them out of `cargo nextest run`.

### Screenshots

Three ways make pictures of the app:

- **Offscreen tests.** `cargo test -p launcher-tests --test seams screenshots::` renders the
  screens of both built-in themes with made-up games, with Slint's software renderer and no
  window. The PNG files go to `target/tmp/screenshots/`, at 1280 × 760. Look at them after a
  theme change: the pull request template asks for it.
- **`scripts/record_demo.sh`** records the README demo, `docs/demo/demo-loop.webp`. It runs
  the release build on a hidden display (Xvfb) at 1920 × 1080, with a temporary home folder,
  five sample PC games and no network, and drives it with key presses. The WebP loops, and the
  script keeps it under 2000 KB, below the git hook's limit. See the script's header.
  <!-- TODO: unverified: scripts/record_demo.sh has not run yet: Xvfb was missing when it was
  written. -->
- **`scripts/screenshots.sh`** takes the user-guide screenshots (`docs/screenshots/*.jpg`)
  from the running app, with your real library; see the script's header.

### Tests that touch the real system

These tests skip, or return at once, unless their variable is set. Weekly CI runs some of
them (see [CI](#ci)).

| Variable | Command | Needs |
| --- | --- | --- |
| `MARAH_DESKTOP_TESTS=1` | `cargo test -p launcher-app --test desktop`: shows real windows and reads which one is in front | A desktop. On Linux, X11 with a window manager, `xdotool` and `xprop`. |
| `MARAH_DESKTOP_REQUIRE=1` | With the above: a skipped desktop test fails | |
| `MARAH_KWIN_TESTS=1` | `cargo test -p launcher-app kwin_says`: asks KWin which window is in front | A KDE Plasma Wayland session |
| `MARAH_TRAILER_TESTS=1` | `cargo test -p launcher-app libmpv_plays`: plays a video through libmpv | libmpv, and `ffmpeg` to make the video |
| `MARAH_TORRENT_TESTS=1` | `cargo test -p launcher-app a_torrent_downloads`: a BitTorrent transfer between two sessions on this computer | |
| `MARAH_VPN_TESTS=1` | `cargo test -p system-host lists_this_computer_s_networks`: lists this computer's networks and connects nothing | |
| `MARAH_REAL_RELEASES=1` | `cargo test -p launcher-app real -- --nocapture`: downloads and installs the newest KytyPS5 and shadPS4 releases | The network and hundreds of megabytes. Anonymous GitHub API calls can hit the rate limit. |
| `MARAH_REAL_ARCHIVES=<folder>` | `cargo test -p launcher-tests --test seams real_archives::`: unpacks real release archives through the plugins | The archives. A missing one is skipped. |
| `MARAH_CATALOG_FILES=<file>[:<file>…]` | `cargo test -p release-catalog catalog_files -- --nocapture`: reads catalog files and prints what each holds | Catalog files |
| `MARAH_TIMING=1` | `cargo test -p launcher-tests --test seams timing:: -- --nocapture`: times the Catalog view with the bundled catalogs | |
| `MARAH_COMPARE_OUT=<folder>` | `cargo test -p launcher-tests --test seams compare::`: renders the screens on a real library into that folder, with no theme saved (so the fallback theme) | A KytyPS5 library in `MARAH_COMPARE_GAMES` (default `~/Games`). Optional: `MARAH_COMPARE_BUILDS`, `MARAH_COMPARE_WEB`. |
| `PS4_TEST_PKG=<file>` | `cargo test --release -p emu-shadps4 real_ps4_package -- --ignored --nocapture`: lists a real PS4 package | A PS4 package |
| `PS5_TEST_PKG=<file>` | `cargo test -p emulator-kit real_package -- --ignored`: lists a real PS5 package and decodes its largest file. `PS5_TEST_FULL=1` decodes every file. | A PS5 package |

To run the desktop tests as weekly CI does (`.github/workflows/weekly.yml`), on Ubuntu:

```sh
sudo apt-get install xvfb openbox xdotool x11-utils libxkbcommon-x11-0 libxcursor1 libxrandr2 libxi6
MARAH_DESKTOP_TESTS=1 MARAH_DESKTOP_REQUIRE=1 xvfb-run -a -s "-screen 0 1280x800x24" \
  bash -c 'openbox & for i in $(seq 50); do xprop -root _NET_SUPPORTING_WM_CHECK | grep -q window && break; sleep 0.2; done;
  cargo test --locked -p launcher-app --test desktop'
```

## Git hooks

`.pre-commit-config.yaml` sets the hooks for [pre-commit](https://pre-commit.com):

- On each commit: merge-conflict markers, YAML and TOML syntax, a newline at the end of each
  file, trailing spaces (not in `.md` files), no added file over 2048 KB, and
  `scripts/check.sh fmt` when Rust files change.
- Before each push: `scripts/check.sh clippy` and `scripts/check.sh test`.

The cargo hooks run `scripts/check.sh`, the same check as `make check` and CI. `make
hooks-install` installs both stages. `make hooks-run` runs the hooks on every file.

## Packaging

`make package` (`scripts/package.sh`) makes a release build with `--locked`, then fills
`dist/marah-launcher-<os>-<arch>/` with `README.md`, `LICENSE`, an empty `extensions/` and a
`licenses/` folder. It runs the packaged `marah-launcher --help` to check that the binary starts.

| System | What it makes |
| --- | --- |
| Linux | The binary, libraries in `lib/`, `marah-launcher.desktop`, `marah-launcher.png` and `install.sh`, packed as `dist/marah-launcher-linux-<arch>.tar.gz` |
| macOS | `MarahLauncher.app`: the binary, libraries in `Contents/Frameworks`, the icon, `extensions/` and the licences in `Contents/Resources`, an `Info.plist` with the workspace version. Signed ad hoc and packed as `dist/marah-launcher-macos-<arch>.zip`. `MARAH_BUNDLE_ID` sets the bundle id. |
| Windows | `marah-launcher.exe`, the DLLs beside it, `extensions/` and `marah-launcher.ico`. CI zips the folder with 7-Zip. |

The `licenses/` folder holds the fonts' licences, `THIRD-PARTY.md` and a folder for each
bundled library:

- `THIRD-PARTY.md` comes from `cargo about generate` (`about.toml`, `about.hbs`). Without
  cargo-about a local package warns and goes on. With `CI` set it fails.
- `scripts/bundle-libmpv.sh` copies libmpv for the trailer player. `MARAH_NO_LIBMPV=1` leaves
  it out.
- `scripts/bundle-libarchive.sh` builds libarchive for RAR and other archives (it copies MSYS2's
  build on Windows). `MARAH_NO_LIBARCHIVE=1` leaves it out.
- Each bundle script writes a `SOURCES` list. `scripts/fetch-sources.sh <package folder>`
  downloads those sources into `dist/<package>-third-party-sources.tar`.
  `MARAH_SOURCES_CACHE` names a folder that keeps the archives between runs.

Release CI makes these files from the package folders:

| File | Made by |
| --- | --- |
| deb, rpm and Arch packages | `packaging/linux/build-packages.sh <folder> <version> <out>` with nfpm (`packaging/linux/nfpm.yaml`), checked by `packaging/linux/check-packages.sh <out>` |
| `Marah-Launcher-<arch>.AppImage` | `scripts/appimage.sh`, with `LINUXDEPLOY` |
| `marah-launcher-macos-universal.zip` and `.dmg` | `packaging/macos/universal.sh`, then `packaging/macos/make-dmg.sh` |
| `marah-launcher-windows-x86_64-setup.exe` | `scripts/windows-installer.sh <version>` (Inno Setup 6, `packaging/windows/marah-launcher.iss`) |

`scripts/nfpm.sh` runs the nfpm version CI pins (`.github/actions/linux-packages/action.yml`),
downloaded once into `target/nfpm/` and checked against its SHA-256.

`install.sh`, in the tarball, installs the package folder for the user, without sudo.
`./install.sh --uninstall` removes it. Check the script with `shellcheck scripts/install.sh`.

`scripts/test-rpm.sh` builds this checkout and takes the rest of the package from the latest
release's Linux tarball. It needs `gh`. `TAG` picks another release. The rpm's version is this
one with `^local.<UTC time>`, so it installs over the release it is built on and the next
release replaces it.

Signing, checksums and `latest.json` are in [signing.md](signing.md).

## CI

The workflows are in `.github/workflows/`:

| Workflow | When | What it does |
| --- | --- | --- |
| `ci.yml` | Pull requests, pushes to `main`, by hand | On Linux, side by side: Format, Clippy, Test (cargo-nextest) and cargo-deny |
| `weekly.yml` | Mondays, by hand | The tests on Windows; on Linux the real emulator releases, the desktop adapter under Xvfb and openbox, the trailer player, and `cargo deny check advisories` |
| `release.yml` | Pushes to `main`, by hand | Release Please: the release pull request, then the draft release and tag. Starts `package.yml` on the tag. |
| `package.yml` | Started by `release.yml` on a Marah Launcher tag, by hand | The packages on Linux, both Mac processors and Windows. On a tag it signs, publishes the release, names it in MarahHQ/marah-os's `marah-launcher.txt`, and starts that repository's `os.yml`, with a token of the Marah Bot GitHub App (variable `MARAH_BOT_CLIENT_ID`, secret `MARAH_BOT_PRIVATE_KEY`). |
| `warm-cache.yml` | `Cargo.lock`, the toolchain or the setup action changes on `main`; Mondays and Thursdays; by hand | Builds what `package.yml` builds and saves the cache on `main`, so a release starts from built dependencies |
| `mirror-deps.yml` | By hand | Copies the pinned Windows libmpv build into this repository's `deps` release |

`ci.yml` skips pushes and pull requests that touch only Markdown, `docs/`,
`version.txt`, `LICENSE`, the workflows `package.yml`, `release.yml`, `weekly.yml`
and `mirror-deps.yml`, the Release Please files or `dependabot.yml`.
It also skips Release Please's own pull requests and the commits they merge. It runs on Linux
only. To try a change on macOS or Windows, run `package.yml` by hand on your branch. Its files
stay as the run's artifacts.

For branch protection, require Format, Clippy, Test and cargo-deny.

The shared steps are in `.github/actions/`:

| Action | Does |
| --- | --- |
| `setup` | The Rust toolchain, the Linux build packages, and the dependency cache |
| `cargo-tool` | cargo-nextest or cargo-about at a pinned version, from the cache when it has it |
| `libmpv` | libmpv and its bundle tools; sets `LINUXDEPLOY` on Linux and `LIBMPV_DIR` on Windows |
| `libarchive` | What `scripts/bundle-libarchive.sh` needs; sets `LIBARCHIVE_DIR` on Windows |
| `sources-list` | Uploads a package's `SOURCES` lists for the `sources` job |
| `linux-packages` | The deb, rpm and Arch packages with a pinned nfpm, checked against its SHA-256 |

The workflows follow these rules:

- Every third-party action is pinned to a commit, with its version in a comment.
- Permissions are `contents: read` or none at the top. A job that writes asks for what it
  needs.
- Every job has a `timeout-minutes`.
- Cargo runs with `--locked`, so CI fails when `Cargo.lock` is out of date.
- A new push to a pull request cancels its older CI run. Every run on `main` finishes and
  saves its cache.
- `package.yml` only restores caches. `warm-cache.yml` saves them on `main`.
- Downloads go through `scripts/download.sh`. With `GH_TOKEN` set, it fetches GitHub files
  through `gh`. Each way tries 5 times.

## The product's names

`brand.toml`, at the root, holds the product's names: what the player reads, the app's id,
the data folder, the names of files written into game folders and backups, and the repo the
launcher's updates come from. Rust reads them as `launcher_api::brand::SECTION_KEY`, generated by
`crates/api/build.rs`. Each name that leaves the program has its own key, and
`tests/tests/seams/brand.rs` pins its value: change one only with the move it needs.

The same test keeps a list of the files that still hold the name
(`tests/fixtures/brand/carry-the-name.txt`). A new file that holds it fails; so does a listed
file that no longer does. Take a new name from `brand.toml` instead, and take a cleaned file off
the list.

## Commits and releases

Write each commit message as a Conventional Commit: `feat:`, `fix:`, `docs:`, `ci:`,
`chore:`, `refactor:`, `test:`, with a scope when it helps, such as
`feat(backup): …`. Mark a breaking change with `!`, such as `feat(themes)!: …`. The pull
request title becomes the squash-merge commit, so write it the same way
(`.github/PULL_REQUEST_TEMPLATE.md`). Every change needs the maintainer's review
(`.github/CODEOWNERS`).

[Release Please](https://github.com/googleapis/release-please) makes the releases from these
messages. The settings are in `.github/release-please-config.json` and
`.github/release-please-manifest.json`. `feat` raises the minor version and `fix` the patch
version. Before 1.0 a breaking change raises the minor version (`bump-minor-pre-major`).

Release Please bumps `version.txt`, `version` in `[workspace.package]`, the workspace's crates in
`Cargo.lock`, and `CHANGELOG.md`. Tags are `v<version>`. Marah OS has its own releases, in
[MarahHQ/marah-os](https://github.com/MarahHQ/marah-os).

A Marah Launcher release goes like this:

1. `release.yml` keeps one release pull request open.
2. Merging it makes a draft release and the tag `v<version>`.
   `release.yml` then starts `package.yml` on the tag.
3. `package.yml` checks that the tag is `v` plus the workspace version, builds every package,
   signs what it can, attaches each file with a `.sha256` file and build provenance, writes
   `latest.json`, and publishes the release as the latest. It does not run the tests: CI did
   that on the commit.
4. `package.yml` then writes the tag into `marah-launcher.txt` on MarahHQ/marah-os's `main` and
   starts its `os.yml`, with a token of the Marah Bot GitHub App (variable `MARAH_BOT_CLIENT_ID`, secret `MARAH_BOT_PRIVATE_KEY`). That push starts
   Marah OS's `release.yml`, which opens the Marah OS release pull request. Without the secret,
   the job says so and does nothing.

When `package.yml` fails for a release, run it again on the same tag: Actions, Package, Run
workflow, with the tag under "Use workflow from". Do not make a new release.

Pull requests opened with the workflow's own token do not start CI. To get checks on the
release pull request, set the secret `RELEASE_PLEASE_TOKEN` to a personal access token or a
GitHub App's token.

## The launcher's own update

The launcher reads `https://github.com/<repo>/releases/latest/download/latest.json`. `<repo>`
is `app.update_repo` in `config.json` (`owner/name`), `MarahHQ/marah-launcher` by default. An
empty `app.update_repo` turns the checks off. The check runs from the "Marah Launcher updates" row in
Settings. A build that is not installed from a package, such as `cargo run`'s, never swaps an
update in. The code is `crates/core/src/launcher_update.rs` and
`crates/app/src/update.rs`. How `latest.json` is made and checked is in
[signing.md](signing.md).
