# Releasing

This page is for maintainers who make a release of the launcher. It covers the packages, the
CI workflows, the release steps, signing, the files a release publishes, and how an installed
launcher finds and applies its own update. Building and testing the code is in
[development.md](development.md).

## Packages

`make package` (`scripts/package.sh`) makes a release build with `--locked`, then fills
`dist/marah-launcher-<os>-<arch>/` with `README.md`, `LICENSE`, an empty `extensions/` and a
`licenses/` folder. It runs the packaged `marah-launcher --help` to check that the binary starts.

| System | What it makes |
| --- | --- |
| Linux | The binary, libraries in `lib/`, `marah-launcher.desktop`, `marah-launcher.png` and `install.sh`, packed as `dist/marah-launcher-linux-<arch>.tar.gz` |
| macOS | `MarahLauncher.app`: the binary, libraries in `Contents/Frameworks`, the icon, `extensions/` and the licences in `Contents/Resources`, and an `Info.plist` with the workspace version. Signed ad hoc and packed as `dist/marah-launcher-macos-<arch>.zip`. `MARAH_BUNDLE_ID` sets the bundle id. |
| Windows | `marah-launcher.exe`, the DLLs beside it, `extensions/` and `marah-launcher.ico`. CI zips the folder with 7-Zip. |

The `licenses/` folder holds the fonts' licences, `THIRD-PARTY.md` and a folder for each
bundled library. What each holds is in [third-party-notices.md](../third-party-notices.md).

- `THIRD-PARTY.md` comes from `cargo about generate` (`about.toml`, `about.hbs`). Without
  cargo-about a local package warns and goes on. With `CI` set it fails.
- `scripts/bundle-libmpv.sh` copies libmpv for the trailer player. `MARAH_NO_LIBMPV=1` leaves
  it out.
- `scripts/bundle-libarchive.sh` builds libarchive for RAR and other archives, or copies
  MSYS2's build on Windows. `MARAH_NO_LIBARCHIVE=1` leaves it out.
- Each bundle script writes a `SOURCES` list. `scripts/fetch-sources.sh <package folder>`
  downloads those sources into `dist/<package>-third-party-sources.tar`.
  `MARAH_SOURCES_CACHE` names a folder that keeps the archives between runs.

From the package folders, the release makes these files:

| File | Made by |
| --- | --- |
| deb, rpm and Arch packages | `packaging/linux/build-packages.sh <folder> <version> <out>` with nfpm (`packaging/linux/nfpm.yaml`), checked by `packaging/linux/check-packages.sh <out>`. `make linux-packages` runs both. |
| `Marah-Launcher-<arch>.AppImage` and its `.zsync` file | `scripts/appimage.sh` (`make appimage`), with `LINUXDEPLOY` and `zsyncmake` |
| `marah-launcher-macos-universal.zip` and `.dmg` | `packaging/macos/universal.sh`, then `packaging/macos/make-dmg.sh` |
| `marah-launcher-windows-x86_64-setup.exe` | `scripts/windows-installer.sh <version>` (Inno Setup 6, `packaging/windows/marah-launcher.iss`) |

The deb, rpm and Arch packages don't ship libmpv: they depend on the system's
(`packaging/linux/build-packages.sh` leaves the bundled copy out).

`scripts/nfpm.sh` runs the nfpm version CI pins (2.47.0, in
`.github/actions/linux-packages/action.yml`), downloaded once into `target/nfpm/` and checked
against its SHA-256. `NFPM` picks another nfpm.

`install.sh`, in the tarball, installs the package folder for the player, without sudo. Its use
is in the [README](../../README.md). Check the script with `shellcheck scripts/install.sh`.

`scripts/test-rpm.sh` (`make test-rpm`) builds this checkout and takes the rest of the package
from the latest release's Linux tarball. It needs `gh`. `TAG` picks another release. The rpm's
version is this one with `^local.<UTC time>`, so it installs over the release it is built on and
the next release replaces it.

### Tools a package needs

On Linux, the two bundle scripts list what they need in their headers. Both work on Ubuntu and
on Fedora:

| | Ubuntu and Debian | Fedora |
| --- | --- | --- |
| libmpv | `libmpv-dev mpv patchelf` | `mpv-libs patchelf` |
| libarchive | `cmake patchelf liblzma-dev libzstd-dev libbz2-dev liblz4-dev zlib1g-dev` | `cmake patchelf xz-devel libzstd-devel bzip2-devel lz4-devel zlib-ng-compat-devel` |

The AppImage also needs [linuxdeploy](https://github.com/linuxdeploy/linuxdeploy), named by
`LINUXDEPLOY`, and `zsyncmake` (the `zsync` package).

- **macOS**: `brew install mpv dylibbundler cmake zstd lz4` (the `libmpv` and `libarchive`
  actions in `.github/actions/`).
- **Windows**: Bash (Git's), 7-Zip, an unpacked shinchiro mpv-dev build named by `LIBMPV_DIR`,
  and MSYS2's libarchive DLLs named by `LIBARCHIVE_DIR`. The `libmpv` and `libarchive` actions
  show how CI fills both. `scripts/windows-installer.sh` needs Inno Setup 6.

The icons need `rsvg-convert` (librsvg) to make them again after a logo change, with
`scripts/make-icons.sh`. The `.icns` for macOS needs macOS's `iconutil`, or libicns's
`png2icns` elsewhere.

## CI

The workflows are in `.github/workflows/`:

| Workflow | When | What it does |
| --- | --- | --- |
| `ci.yml` | Pull requests, pushes to `main`, by hand | On Linux, side by side: Format, Clippy, Test (cargo-nextest) and cargo-deny |
| `weekly.yml` | Mondays, by hand | The tests on Windows. On Linux: the real emulator releases, the desktop adapter under Xvfb and openbox, the trailer player, the game sandbox through the real bubblewrap, a real BitTorrent transfer, and `cargo deny check advisories` |
| `release.yml` | Pushes to `main`, by hand | Release Please: the release pull request, then the draft release and its tag. Starts `package.yml` on the tag. |
| `package.yml` | Started by `release.yml` on a tag, by hand | The packages on Linux, both Mac processors and Windows. On a tag it signs, publishes the release, and tells MarahHQ/marah-os and MarahHQ/packages. |
| `warm-cache.yml` | `Cargo.lock`, the toolchain or the setup action changes on `main`; Mondays and Thursdays; by hand | Builds what `package.yml` builds and saves the cache on `main`, so a release starts from built dependencies |
| `mirror-deps.yml` | By hand | Copies the pinned Windows libmpv build into this repository's `deps` release |

`ci.yml` skips pushes and pull requests that touch only Markdown, `docs/`, `version.txt`,
`LICENSE`, the workflows `package.yml`, `release.yml`, `weekly.yml` and `mirror-deps.yml`, the
Release Please files or `dependabot.yml`. It also skips Release Please's own pull requests and
the commits they merge. It runs on Linux only. To try a change on macOS or Windows, run
`package.yml` by hand on your branch. Its files stay as the run's artifacts.

For branch protection, require Format, Clippy, Test and cargo-deny.

The shared steps are in `.github/actions/`:

| Action | Does |
| --- | --- |
| `setup` | The Rust toolchain, the Linux build packages, and the dependency cache |
| `cargo-tool` | cargo-nextest or cargo-about at a pinned version, from the cache when it has it |
| `libmpv` | libmpv and its bundle tools. Sets `LINUXDEPLOY` on Linux and `LIBMPV_DIR` on Windows. |
| `libarchive` | What `scripts/bundle-libarchive.sh` needs. Sets `LIBARCHIVE_DIR` on Windows. |
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
- Downloads go through `scripts/download.sh`. With `GH_TOKEN` set and `gh` installed, it fetches
  GitHub files through `gh`. Each way tries 5 times.

## Release steps

[Release Please](https://github.com/googleapis/release-please) makes the releases from the
Conventional Commit messages ([development.md](development.md#commits-and-pull-requests)). The
settings are in `.github/release-please-config.json` and `.github/release-please-manifest.json`.
`feat` raises the minor version and `fix` the patch version. Before 1.0 a breaking change raises
the minor version (`bump-minor-pre-major`).

Release Please bumps `version.txt`, `version` in `[workspace.package]`, the workspace's crates in
`Cargo.lock`, and `CHANGELOG.md`. Don't edit `CHANGELOG.md` by hand. Tags are `v<version>`.
Marah OS has its own releases, in [MarahHQ/marah-os](https://github.com/MarahHQ/marah-os).

A release goes like this:

1. `release.yml` keeps one release pull request open.
2. Merging it makes a draft release and the tag `v<version>`. `release.yml` then starts
   `package.yml` on the tag.
3. `package.yml` checks that the tag is `v` plus the workspace version, builds every package,
   signs what it can, attaches each file with a `.sha256` file and build provenance, writes
   `latest.json`, and publishes the release as the latest. It does not run the tests: CI did
   that on the commit.
4. The `os` job then writes the tag into `marah-launcher.txt` on MarahHQ/marah-os's `main`,
   starts that repository's `os.yml`, and starts `publish.yml` in MarahHQ/packages. It uses a
   token of the Marah Bot GitHub App (organization variable `MARAH_BOT_CLIENT_ID`, secret
   `MARAH_BOT_PRIVATE_KEY`). Without them, the job says so and does nothing.

When `package.yml` fails for a release, run it again on the same tag: Actions, Package, Run
workflow, with the tag under "Use workflow from". Do not make a new release.

Pull requests opened with the workflow's own token do not start CI. To get checks on the
release pull request, set the secret `RELEASE_PLEASE_TOKEN` to a personal access token or a
GitHub App's token.

## Signing

Signing is optional. Without the secrets, `package.yml` still builds every package and prints a
warning when `MACOS_CERT_P12` or `WINDOWS_CERT_PFX` is missing. The packages then carry only the
signing the build does by itself: ad hoc on macOS, none on Windows. An unsigned build makes
macOS Gatekeeper ask before the first start, and Windows SmartScreen warn.

| Package | Signed with | When |
| --- | --- | --- |
| `MarahLauncher.app` and `marah-launcher-macos-universal.dmg` | Apple Developer ID, notarized and stapled | `MACOS_CERT_P12` is set |
| `marah-launcher-macos-universal.zip` | Not signed itself: it holds the signed, stapled app | `MACOS_CERT_P12` is set |
| `MarahLauncher.app` without the secrets | Ad hoc (`codesign --sign -`) | Always: Apple Silicon runs nothing unsigned |
| `marah-launcher.exe` and the `.dll` files | Authenticode | `WINDOWS_CERT_PFX` is set |
| `marah-launcher-windows-x86_64-setup.exe` | Authenticode | `WINDOWS_CERT_PFX` is set |
| Linux tar.gz, AppImage, deb, rpm, Arch package | Nothing | Never |
| Every release file | A `.sha256` file and a build provenance attestation | Release tags only |

### Secrets

Set them in the repository's Actions secrets. Never print or commit their values.

| Secret | Holds | Read by |
| --- | --- | --- |
| `MACOS_CERT_P12` | The Developer ID Application certificate as a `.p12` file, in base64 | `macos` job |
| `MACOS_CERT_PASSWORD` | The password of the `.p12` file | `macos` job |
| `APPLE_ID` | The Apple ID that notarizes | `macos` job |
| `APPLE_TEAM_ID` | The Apple team id | `macos` job |
| `APPLE_APP_PASSWORD` | An app-specific password for that Apple ID | `macos` job |
| `WINDOWS_CERT_PFX` | The code-signing certificate as a `.pfx` file, in base64 | `scripts/sign-windows.ps1` |
| `WINDOWS_CERT_PASSWORD` | The password of the `.pfx` file | `scripts/sign-windows.ps1` |
| `RELEASE_PLEASE_TOKEN` | Optional: a personal access token or a GitHub App's token, so the release pull requests start CI | `release.yml` |
| `MARAH_BOT_PRIVATE_KEY` | The Marah Bot GitHub App's private key (organization secret, with the variable `MARAH_BOT_CLIENT_ID`) | `os` job |

Set the five macOS secrets together. The workflow tests only `MACOS_CERT_P12` before it signs
and notarizes.

### The jobs of `package.yml`

On a branch `package.yml` only builds, and the files stay as the run's artifacts: use that to
try a signing change.

| Job | Permissions | Does |
| --- | --- | --- |
| `version` | `contents: read` | Reads the version from `Cargo.toml`. On a tag, fails when the tag is not `v<version>`. |
| `build` | `contents: read` | Linux (Ubuntu 22.04) and Windows. Builds, packs, and signs the Windows files. |
| `build-mac` | `contents: read` | Apple Silicon (`macos-15`) and Intel (`macos-15-intel`). Builds and packs each app as a `mac-*` artifact. |
| `macos` | `contents: read` | Joins the two apps into one universal app, signs and notarizes it, makes the disk image. |
| `sources` | `contents: read` | Fetches the third-party sources the builds list, one tar per package. |
| `publish` | `contents: write`, `id-token: write`, `attestations: write` | On a tag only. Checksums, `latest.json`, attestations, upload, publish. |
| `os` | None: it makes a Marah Bot token | After `publish`. See [Release steps](#release-steps). |

`publish` downloads only the `build-*` artifacts. The per-processor Mac zips are `mac-*`
artifacts, so they never reach the release.

### macOS

`scripts/package.sh` signs each `MarahLauncher.app` ad hoc: `codesign --force --deep --sign -`.
`packaging/macos/universal.sh` joins the Apple Silicon and the Intel app with `lipo` and signs
the result ad hoc again. Without the secrets, the release ships that app.

With `MACOS_CERT_P12` set, the step "Sign and notarize the app" in the `macos` job does this:

1. Makes a keychain in `$RUNNER_TEMP`, decodes the certificate, imports it for `codesign`, and
   deletes the decoded file.
2. Signs every `.dylib` in `Contents/Frameworks`, then the app, with
   `codesign --force --timestamp --options runtime --sign "Developer ID Application"`.
   Notarization asks for the hardened runtime and a timestamp.
3. Zips the app with `ditto` and sends it with `xcrun notarytool submit … --wait`.
4. Staples the ticket to the app with `xcrun stapler staple`.
5. Zips the stapled app again as `marah-launcher-macos-universal.zip`.

The step "Disk image" runs `packaging/macos/make-dmg.sh`: a disk image with `MarahLauncher.app`
and a link to `/Applications`. With `MACOS_CERT_P12` set, it signs, notarizes and staples the
image. Then it checks the image with `hdiutil verify`, mounts it, and tests that the app's
binary and the link are there.

The workflow passes no entitlements file to `codesign`.

### Windows

`scripts/sign-windows.ps1 <file or wildcard>…` signs files with Authenticode. The `build` job
runs it twice:

1. Before packing: `dist\marah-launcher-windows-*\*.exe` and `dist\marah-launcher-windows-*\*.dll`.
   The zip and the installer are then built from the signed files.
2. After `scripts/windows-installer.sh`: `dist\marah-launcher-windows-x86_64-setup.exe`.

The zip itself is not signed.

Without `WINDOWS_CERT_PFX`, the script prints a warning and signs nothing. With it, the script
writes the certificate to `$RUNNER_TEMP\cert.pfx`, takes the newest `signtool.exe` under
`Windows Kits\10\bin`, and signs each file with `/fd sha256` and DigiCert's timestamp server
(`/tr http://timestamp.digicert.com /td sha256`). A failed file stops the script with
signtool's exit code. At the end it deletes the certificate file.

The job then tries the signed installer: a silent install, a check for the file
`installed-by-setup`, then a silent uninstall.

### Linux

Nothing is signed. `packaging/linux/nfpm.yaml` sets no signature.

The AppImage carries its update information in its `.upd_info` section:
`gh-releases-zsync|<owner>|<repo>|latest|Marah-Launcher-<arch>.AppImage.zsync`, with the owner
and repository from `update_repo` in `brand.toml`. `scripts/appimage.sh` sets it through
linuxdeploy's `LDAI_UPDATE_INFORMATION`. Then `zsyncmake` writes the zsync control file,
`Marah-Launcher-<arch>.AppImage.zsync`, from the finished AppImage. The control file goes into
the release beside the AppImage. It has no `.sha256` and no entry in `latest.json`.

The tools CI downloads are pinned and checked against a SHA-256: nfpm (`linux-packages`
action), linuxdeploy and the Windows mpv build (`libmpv` action). The Windows libarchive comes
from the runner's MSYS2, which the workflow does not pin.

## Checksums, attestations and `latest.json`

The `publish` job runs on a tag only. For each release file (`*.tar.gz`, `*.zip`, `*.tar`,
`*.AppImage`, `*.deb`, `*.rpm`, `*.pkg.tar.zst`, `*.exe`, `*.dmg`) it:

1. Writes `<file>.sha256` with `sha256sum`.
2. Attests the file with `actions/attest-build-provenance`. This is a signed record of the
   workflow run that built the file, not a code-signing certificate. A player checks it with
   `gh attestation verify <file> --repo MarahHQ/marah-launcher`.

Then it writes `latest.json` with `scripts/update-manifest.sh`, uploads every file with
`gh release upload --clobber`, and publishes the draft with
`gh release edit --draft=false --latest`. The launcher reads the latest release, so it never
sees a release without its files.

`scripts/update-manifest.sh <folder> <tag> <owner/repo>` writes the version (the tag without
its `v`) and one entry for each kind of install. Each entry has a `url` into the release and the
`sha256` from the file's `.sha256`.

| Key | File |
| --- | --- |
| `linux-<arch>` | `marah-launcher-linux-<arch>.tar.gz` |
| `linux-<arch>-appimage` | `Marah-Launcher-<arch>.AppImage` |
| `linux-x86_64-deb` | `marah-launcher_<version>-1_amd64.deb` |
| `linux-x86_64-rpm` | `marah-launcher-<version>-1.x86_64.rpm` |
| `linux-x86_64-arch` | `marah-launcher-<version>-1-x86_64.pkg.tar.zst` |
| `macos-aarch64`, `macos-x86_64` | `marah-launcher-macos-universal.zip` (one app for both) |
| `windows-<arch>` | `marah-launcher-windows-<arch>.zip` |
| `windows-<arch>-setup` | `marah-launcher-windows-<arch>-setup.exe` |

The third-party source tars and the disk image get no entry. `latest.json` itself is not
signed.

## The launcher's own update

The core checks for an update in `crates/core/src/launcher_update.rs`.
`crates/app/src/update.rs` applies a kept update at the next start.

1. The repository is `app.update_repo` in `config.json` (`owner/name`), by default
   `MarahHQ/marah-launcher` (`DEFAULT_REPO`, from `brand.toml`). An empty value turns the
   checks off.
2. The core reads `https://github.com/<repo>/releases/latest/download/latest.json`. Marah OS
   releases are in their own repository, so the latest release here is always a launcher one.
3. It offers the update only when the file's version is newer, as semver.
4. It takes the entry for this install's key (`package_key`). `Install::detect` decides the
   install: the AppImage (`APPIMAGE` is set), a deb, rpm or Arch package (the file `packaged`
   beside the binary), the Windows installer (`installed-by-setup`), `MarahLauncher.app`, the
   unpacked folder, or Marah OS. With no entry for the key, it says the version has no package
   for this system.
5. It downloads only over HTTPS from `github.com`, `objects.githubusercontent.com` and
   `release-assets.githubusercontent.com` (`HOSTS`), with at most 5 redirects
   (`MAX_REDIRECTS`). Any other host stops the download.
6. An AppImage install first tries a delta update (`crates/core/src/zsync.rs`). It reads
   `<AppImage url>.zsync`, finds the blocks the running AppImage already has, and downloads
   only the missing blocks with HTTP `Range` requests. When that fails, it logs a line and
   downloads the whole file.
7. It computes the file's SHA-256 and compares it with `sha256` in `latest.json`. A mismatch
   fails the update.

How the update goes in (`Install::method`):

| Install | Method | What happens |
| --- | --- | --- |
| tar.gz folder, AppImage, `MarahLauncher.app`, Windows zip | `swap` | Kept in `updates/` in the data folder. At the next start the launcher puts the new install in place of the old one and starts it. When anything fails, the old one comes back. |
| Windows installer | `installer` | At the next start the launcher runs the new installer with `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART`, and the installer starts the new version. |
| deb, rpm, Arch | `package` | The launcher runs `pkexec` with `apt-get install -y`, `dnf install -y` or `pacman -U --noconfirm` on the file. The system asks for the administrator password. |
| Marah OS | `system` | Nothing downloads. The new launcher comes with the next system update ([The OS contract](os-contract.md)). |

A build that is not installed, such as `cargo run`'s, never swaps an update in.

## Limitations

- **The update check proves the download, not the publisher.** `latest.json` and the packages
  come from the same release. The SHA-256 catches a damaged download, but whoever can publish a
  release can publish a matching hash. The launcher does not check the Authenticode signature,
  the Apple signature or the attestation.
- **Linux packages are not signed.** No GPG key signs the deb, rpm, Arch package or AppImage.
- **The Windows zip and the macOS zip are not signed as files.** The files inside them are.
- **Not tried for this page.** It was written from the workflows and the scripts. No build,
  signing or notarization ran for it.
