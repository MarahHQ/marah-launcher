# Development

This page tells a contributor how to build the launcher, run it, test it and check a change.
Run every command from the repository root. How the code is laid out is in
[architecture.md](architecture.md). How a release is packaged, signed and published is in
[releasing.md](releasing.md).

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
  into errors. A module that allows `unsafe_code` says why on the same line.
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
| Node.js 20 or later | The catalog collector in `scripts/catalog/` | see [scripts/catalog/README.md](../../scripts/catalog/README.md) |

`third_party/oozextract` is a vendored crate outside the workspace (`exclude` in `Cargo.toml`),
so clippy doesn't lint it. See [third-party-notices.md](../third-party-notices.md).

## System packages

On Linux a build needs a C toolchain, `pkg-config`, and the development files for fontconfig
and xkbcommon (Slint), udev (gilrs, for controllers) and ALSA (rodio, for sounds). CI installs
them on Ubuntu (`.github/actions/setup/action.yml`):

```sh
sudo apt-get install libfontconfig1-dev libxkbcommon-dev libudev-dev libasound2-dev
```

On Windows, `crates/app/build.rs` puts the icon and the version info into `marah-launcher.exe`
with the Windows resource compiler. Without the compiler a local build warns and goes on. With
`CI` set, the build fails.

The tools a package needs are in [releasing.md](releasing.md). What the launcher needs at run
time on Linux (libmpv, unar, bubblewrap and more) is the `depends` and `recommends` of each
package in `packaging/linux/nfpm.yaml`.

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
| `make test` | `scripts/check.sh test`: every test, with nextest when it is installed |
| `make run` | `cargo run -p launcher-app` (full screen) |
| `make run-windowed` | `cargo run -p launcher-app -- --windowed` |
| `make run-session` | The launcher in gamescope with `MARAH_SESSION=1`, as the session runs it. Needs gamescope. `GAMESCOPE_ARGS` sets gamescope's options (default `-f -e --xwayland-count 1`). |
| `make run-session-windowed` | `MARAH_SESSION=1` in a normal window, without gamescope |
| `make build` | `cargo build -p launcher-app --release` |
| `make package` | `scripts/package.sh`: a release build in `dist/marah-launcher-<os>-<arch>/` |
| `make linux-packages` | `make package`, then the deb, rpm and Arch packages in `dist/packages/`, checked |
| `make appimage` | `make package`, then `scripts/appimage.sh` |
| `make test-rpm` | `scripts/test-rpm.sh`: an rpm of this checkout in `dist/test-rpm/`, for this PC only |
| `make deny` | `cargo deny check` |
| `make hooks-install` | Installs the git hooks for commit and push |
| `make hooks-run` | Runs the git hooks on every file |
| `make catalog` | Reads the forums again (Chrome opens) into `assets/catalog/`. `CONSOLE=pc` does one catalog. |
| `make catalog-refresh` | Updates `assets/catalog/` from the forums' feed and API, without a browser |
| `make clean` | `cargo clean`, then removes `dist/` and `target/` |

The packaging targets are in [releasing.md](releasing.md). The `catalog` targets are in
[scripts/catalog/README.md](../../scripts/catalog/README.md).

`make clean` also removes the files git tracks in `dist/catalog/`. Run `git checkout -- dist`
to get them back.

## Running

```sh
make run                                   # full screen
make run-windowed                          # a normal window
cargo run -p launcher-app -- --help        # every option
cargo run -p launcher-app -- --windowed --dev --extensions ~/my-themes
```

The options that `--help` prints are in the
[command-line table](../players/troubleshooting.md#command-line-options). Three more things are
for development only:

- `--dev` reloads a theme from a folder when one of its files changes. The built-in themes,
  Default and Classic, are compiled into the binary, so an edit to them needs a new build.
- `--probe-controllers` and `--probe-gpu` answer and exit (`main.rs`). The launcher runs itself
  with them in a child process for the "This PC" checks. `--help` doesn't list them.
- `MARAH_SCALE=<n>` draws everything at that size for this run, from 0.8 to 1.4. It overrides
  "Size of everything" in Settings: `MARAH_SCALE=1.4 make run-windowed`.

For a clean start, point `--data-dir` at an empty folder. The default data folder is in
[Where the launcher keeps its data](../players/troubleshooting.md#where-the-launcher-keeps-its-data).

### Session switches

Three variables make the launcher act as it does in its own session or on Marah OS, on any Linux
desktop. Each counts only when its value is `1` (`Place::detect` in `crates/core/src/system.rs`).

| Variable | What shows |
| --- | --- |
| `MARAH_SESSION=1` | The launcher as the whole screen: "Restart Marah Launcher" in place of Quit, Log out, and Restart and Shut down that ask by a second press, not in a dialog |
| `MARAH_OS=1` | Also Marah OS: Log out goes away |
| `MARAH_USB=1` | Also the Marah OS USB stick: the welcome and install screens |

The network, sound, Bluetooth and drives controls, Settings → System, and Restart and Shut down
show without these variables too, in a window on the desktop. The controls act on this computer through `nmcli`, `bluetoothctl`, `pactl` and `systemctl`.
Joining a Wi-Fi network, pairing, the volume, Restart and Shut down are real. System updates
and the install need Marah OS: elsewhere they fail and show their error. See
[The OS contract](os-contract.md).

`make run-session` does not start the launcher again when it ends, and has no fallback to cage.
The real session script, `packaging/linux/marah-session`, does both.

### Quitting

Ctrl-C, SIGTERM and SIGHUP close the launcher as Quit does (`quit_on_signal` in
`crates/app/src/platform.rs`). The core then gets 10 seconds to shut down (`SHUTDOWN_LIMIT` in
`main.rs`). A second signal quits at once with exit code 130.

## Logging

The launcher logs to stderr through `tracing`. The level is `info` unless `RUST_LOG` sets another
filter (`EnvFilter` in `main.rs`). Targets are module paths: the binary is `marah_launcher`, and
each library crate goes by its name with underscores, such as `launcher_core` or
`launcher_ui_host`. What a plugin logs through its context goes to `launcher_core::facades`,
with the plugin's id in the field `plugin_id`.

```sh
RUST_LOG=debug make run                                # everything
RUST_LOG=info,launcher_core::facades=debug make run    # the plugins' own debug lines
```

The session writes the launcher's output to `~/.cache/marah-session.log` (`MARAH_LOG` in
`packaging/linux/session.conf`).

## Tests

```sh
make test                                                # every test (scripts/check.sh test)
cargo nextest run --workspace                            # every test, each in its own process
cargo test -p launcher-tests --test seams themes::       # one area: a module of tests/tests/seams/
cd scripts/catalog && npm test                           # the catalog collector's tests
```

The tests need no window, GPU, controller or sound device. They use Slint's testing backend
and the fakes in `crates/testkit` (`FakeHost`, `FakeEmulator`, `FakeSystem`, `ThemeDriver` and
more). Unit tests sit beside the code in each crate.

### The seams

The `launcher-tests` crate (`tests/`) holds the seam tests: the core's API, the capability
contracts, and the composed host (the core, the theme host and a theme). They are one test
binary, `seams` (`tests/tests/seams/main.rs`, one module per area), so the shared code in
`tests/tests/seams/common/` builds and links once. `tests/tests/spans.rs` is its own binary,
because `tracing` caches per-callsite interest across threads. What each seam covers is in
[architecture.md](architecture.md).

- A test that needs the Game Catalog to offer releases gives `common::serve_catalog` the
  topics of each console, or `Composed::serve_catalog` on the composed host. It serves each
  file at `common::CATALOG_URL`, sets the Game Catalog's address, and waits until the catalog
  is read.
- An emulator that installs a download is `FakeCatalog::kyty().installs(title)`.
- `every_theme.rs` runs one suite against each built-in theme (`THEMES`: Classic and Default).
  A new built-in theme goes into that list.

`launcher-app`'s desktop tests (`crates/app/tests/desktop.rs`) have their own harness and show
real windows. `.config/nextest.toml` leaves them out of `cargo nextest run`, and stops any test
that runs for more than five minutes.

### Tests that touch the real system

These tests skip, or return at once, unless their variable is set. The weekly workflow runs
some of them (see [releasing.md](releasing.md)).

| Variable | Command | Needs |
| --- | --- | --- |
| `MARAH_DESKTOP_TESTS=1` | `cargo test -p launcher-app --test desktop`: shows real windows and reads which one is in front | A desktop. On Linux, X11 with a window manager, `xdotool` and `xprop`. |
| `MARAH_DESKTOP_REQUIRE=1` | With the above: a skipped desktop test fails | |
| `MARAH_KWIN_TESTS=1` | `cargo test -p launcher-app kwin_says`: asks KWin which window is in front | A KDE Plasma Wayland session |
| `MARAH_TRAILER_TESTS=1` | `cargo test -p launcher-app libmpv_plays`: plays a video through libmpv | libmpv, and `ffmpeg` to make the video |
| `MARAH_SANDBOX_TESTS=1` | `cargo test -p launcher-core --lib bubblewrap_shows`: runs a shell through the real bubblewrap | `bwrap` |
| `MARAH_TORRENT_TESTS=1` | `cargo test -p launcher-app torrents::tests`: real BitTorrent transfers between two sessions on this computer, and the torrents contract | |
| `MARAH_VPN_TESTS=1` | `cargo test -p system-host lists_this_computer_s_networks`: lists this computer's networks and connects nothing | Linux or macOS |
| `MARAH_REAL_RELEASES=1` | `cargo test -p launcher-app real -- --nocapture`: downloads and installs the newest KytyPS5 and shadPS4 releases | The network and hundreds of megabytes. Anonymous GitHub API calls can hit the rate limit. |
| `MARAH_REAL_ARCHIVES=<folder>` | `cargo test -p launcher-tests --test seams real_archives::`: unpacks real release archives through the plugins | The archives. A missing one is skipped. |
| `MARAH_CATALOG_FILES=<file>[:<file>…]` | `cargo test -p release-catalog catalog_files -- --nocapture`: reads catalog files and prints what each holds | Catalog files |
| `MARAH_TIMING=1` | `cargo test -p launcher-tests --test seams timing:: -- --nocapture`: times the Catalog view with the bundled catalogs, and the renders of the first 10 s of a start with 500 games | |
| `MARAH_COMPARE_OUT=<folder>` | `cargo test -p launcher-tests --test seams compare::`: renders the screens on a real library into that folder, with no theme saved (so the fallback theme) | A KytyPS5 library in `MARAH_COMPARE_GAMES` (default `~/Games`). Optional: `MARAH_COMPARE_BUILDS`, `MARAH_COMPARE_WEB`. |
| `PS4_TEST_PKG=<file>` | `cargo test --release -p emu-shadps4 real_ps4_package -- --ignored --nocapture`: lists a real PS4 package | A PS4 package |
| `PS5_TEST_PKG=<file>` | `cargo test -p emulator-kit real_package -- --ignored`: lists a real PS5 package and decodes its largest file. `PS5_TEST_FULL=1` decodes every file. | A PS5 package |

To run the desktop tests as the weekly workflow does (`.github/workflows/weekly.yml`), on
Ubuntu:

```sh
sudo apt-get install xvfb openbox xdotool x11-utils libxkbcommon-x11-0 libxcursor1 libxrandr2 libxi6
MARAH_DESKTOP_TESTS=1 MARAH_DESKTOP_REQUIRE=1 xvfb-run -a -s "-screen 0 1280x800x24" \
  bash -c 'openbox & for i in $(seq 50); do xprop -root _NET_SUPPORTING_WM_CHECK | grep -q window && break; sleep 0.2; done;
  cargo test --locked -p launcher-app --test desktop'
```

## Screenshots

Three tools make pictures of the app:

- **Offscreen tests.** `cargo test -p launcher-tests --test seams screenshots::` renders the
  screens of both built-in themes with made-up games, with Slint's software renderer and no
  window. The PNG files go to `target/tmp/screenshots/`, at 1280 × 760. Look at them after a
  theme change: the pull request template asks for it.
- **`scripts/record_demo.sh`** records the README demo, `docs/demo/demo-loop.webp`. It runs
  the release build on a hidden display (Xvfb) at 1920 × 1080, with a temporary home folder
  and no network, and drives it with key presses. It shows five sample PC games, or your own
  games with `--library ~/.local/share/marah-launcher`: the launcher then starts on a copy of
  that data folder and never writes to it. It also has no system bus, so your Wi-Fi networks,
  Bluetooth devices and drives stay out of the recording. Look at each screen before you
  commit it. It keeps the WebP under 2000 KB, below the git hook's limit. Linux only.
- **`scripts/screenshots.sh`** takes the player docs' screenshots (`docs/screenshots/*.jpg`)
  from the running app, with your real library. Linux only, on X11 or XWayland.

Each script's header lists its options. The demo needs these tools:

| Tool | For | Ubuntu and Debian | Fedora |
| --- | --- | --- | --- |
| Xvfb | The hidden display the launcher runs on | `xvfb` | `xorg-x11-server-Xvfb` |
| xdotool | The key presses that drive the launcher | `xdotool` | `xdotool` |
| ffmpeg | The recording, and the WebP made from it. It needs libx264 and libwebp. | `ffmpeg` | `ffmpeg` |
| ImageMagick 7 (`magick`) | The art of the sample games | `imagemagick` | `ImageMagick` |

The demo also runs the launcher with no network through `unshare` (util-linux).
`scripts/screenshots.sh` needs `xdotool` and ImageMagick's `import`.

## Git hooks

`.pre-commit-config.yaml` sets the hooks for [pre-commit](https://pre-commit.com):

- On each commit: merge-conflict markers, YAML and TOML syntax, a newline at the end of each
  file, trailing spaces (not in `.md` files), no added file over 2048 KB, and
  `scripts/check.sh fmt` when Rust files change.
- Before each push: `scripts/check.sh clippy` and `scripts/check.sh test`.

The cargo hooks run `scripts/check.sh`, the same check as `make check` and CI. CI also runs
clippy on Windows and macOS, so code for those systems must compile there too. To check the
system plugin for Windows from Linux, run `rustup target add x86_64-pc-windows-gnu`, then
`cargo clippy -p system-host --all-targets --target x86_64-pc-windows-gnu -- -D warnings`. The
whole workspace also needs the MinGW C compiler (`x86_64-w64-mingw32-gcc`) for its C libraries.
`make hooks-install` installs both stages. `make hooks-run` runs the hooks on every file.

## The product's names

`brand.toml`, at the root, holds the product's names: what the player reads, the app's id, the
data folder, the names of files written into game folders and backups, and the repository the
launcher's updates come from. Rust reads them as `launcher_api::brand::SECTION_KEY`, which
`crates/api/build.rs` generates. Each name that leaves the program has its own key, and
`tests/tests/seams/brand.rs` pins its value: change one only with the move it needs.

The same test keeps a list of the files that still hold the name
(`tests/fixtures/brand/carry-the-name.txt`). A new file that holds it fails, and so does a
listed file that no longer holds it. Take a new name from `brand.toml` instead, and take a
cleaned file off the list.

## Commits and pull requests

Write each commit message as a Conventional Commit: `feat:`, `fix:`, `docs:`, `ci:`, `chore:`,
`refactor:` or `test:`, with a scope when it helps, such as `feat(backup): …`. Mark a breaking
change with `!`, such as `feat(themes)!: …`. The pull request title becomes the squash-merge
commit, so write it the same way (`.github/PULL_REQUEST_TEMPLATE.md`). Every change needs a
review from the maintainers (`.github/CODEOWNERS`).

Release Please reads these messages to make each release: see [releasing.md](releasing.md).
