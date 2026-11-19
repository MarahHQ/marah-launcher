# Third-party code and notices

This page is for anyone who redistributes the launcher. It lists what comes from other projects:
code in this repository, Rust crates, libraries inside the packages, fonts and pictures, and the
programs the launcher downloads at run time. Each licence named here was read from a file in
the repository or from a script that fetches it. Where the repository holds no licence
information, this page says so.

## The launcher's own licence

The launcher is under the GNU General Public License, version 3 or (at your option) any later
version. `LICENSE` holds the text of version 3. The workspace declares
`license = "GPL-3.0-or-later"` in `Cargo.toml`. The PS4 package reader is a port of shadPS4
code, which is GPL-2.0-or-later and so can be part of a GPL-3.0-or-later work. Slint is used
under its GPL-3.0 option.

## Code from other projects in this repository

| Where | What | Licence |
| --- | --- | --- |
| `plugins/emulators/shadps4/src/pkg4.rs` | The PS4 fake package reader. Ported from shadPS4 v0.7.0 (`src/core/file_format/pkg.cpp`, `src/core/crypto/crypto.cpp`), with the file system walk of LibOrbisPkg in place of shadPS4's guesses. | shadPS4: GPL-2.0-or-later (the file's SPDX line). LibOrbisPkg: MIT (the file's header). |
| `plugins/emulators/shadps4/src/pkg4_data.rs` | Key data for that reader, generated from shadPS4 v0.7.0 (`src/core/crypto/keys.h`, `src/core/file_format/pkg_type.cpp`). Do not edit it by hand. | GPL-2.0-or-later (the file's SPDX line). Copyright 2024 shadPS4 Emulator Project. |
| `plugins/emulators/shadps4/src/default-input.ini` | shadPS4's default key map, from shadPS4 v0.21.0 (`GetDefaultInputConfig` in `src/input/input_handler.cpp`, as `src/key_map.rs` says), with the touchpad button on the right half of the touchpad. | shadPS4's licence, GPL-2.0-or-later. The file itself has no licence line. |
| `third_party/oozextract` | A vendored copy of the `oozextract` crate 0.5.5, a Kraken decompressor that the PS5 package reader uses. | MIT, as the crate declares in `third_party/oozextract/Cargo.toml`. |

Everything else in the repository is this project's own code. That includes the PS5 package
reader (`plugins/shared/emulator-kit/src/pkg5.rs`) and the exFAT reader (`exfat.rs`).

### oozextract

`third_party/oozextract/NOTICE.md` records the origin. The crate is by lvlvllvlvllvlvl
(<https://github.com/lvlvllvlvllvlvl/oozextract>). It is a Rust port of `ooz` by powzix
(<https://github.com/powzix/ooz>).

The project changed the copy in these ways, as `NOTICE.md` lists them:

- It removed the command line tool, the tests, the benches, and the optional `tokio`, `wasm` and
  `clap` dependencies.
- `src/algorithm/kraken.rs` and `src/decoder/mod.rs` now read the "excess bytes" form of a
  Kraken table. Upstream refuses it. PS5 packages use it.
- `decode_bytes_type12` returned the wrong byte count for a table with one symbol. It now
  returns the bytes it read, as `ooz` does.
- `Cargo.toml` has an empty `[workspace]` table, so the crate is its own workspace root. The
  launcher's workspace excludes it.

## Rust crates

Each crate keeps its own licence. `deny.toml` allows these licences, and `cargo deny check`
fails on any other. CI runs the check (the `deny` job in `.github/workflows/ci.yml`).

```
0BSD, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, BSL-1.0,
CC0-1.0, CDLA-Permissive-2.0, GPL-3.0-only, ISC, MIT, MIT-0, MPL-2.0, Unicode-3.0, Zlib
```

The comments in `deny.toml` say why:

- The launcher is GPL-3.0-or-later, so every licence must allow use in a GPL-3.0 work.
- Apache-2.0 and GPL-3.0-only are fine in a GPL-3.0 work.
- Slint is GPL-3.0-only (or its own licences). MPL-2.0 is the symphonia audio decoders.
  CDLA-Permissive-2.0 is the webpki root certificates.

`deny.toml` also denies wildcard versions, unknown registries and unknown git sources: only
crates.io is allowed. It lists the RustSec advisories it accepts, each with its reason.

`about.toml` lists the same licences for `cargo about`, and `about.hbs` is the template. Each
package gets `licenses/THIRD-PARTY.md`, which `scripts/package.sh` makes with
`cargo about generate --locked --workspace --fail`. The file lists each licence with the crates
that use it, then each licence text.

`about.toml` and `deny.toml` both set `private = { ignore = true }`, and a crate with
`publish = false` counts as private. `third_party/oozextract` sets `publish = false`, so
`THIRD-PARTY.md` most likely leaves it out. See [Limitations](#limitations).

## What the packages ship

Besides the binary, each package holds `README.md`, `LICENSE` and a `licenses/` folder. How
the packages are made is in [contributors/releasing.md](contributors/releasing.md).

| Package | `licenses/` is in |
| --- | --- |
| Linux tar.gz | The package folder, beside the binary |
| deb, rpm, Arch | `/usr/lib/marah-launcher/licenses` (`packaging/linux/nfpm.yaml`) |
| AppImage | `usr/share/licenses/marah-launcher/`, with `LICENSE` (`scripts/appimage.sh`) |
| macOS | `MarahLauncher.app/Contents/Resources/licenses` |
| Windows | The package folder, beside `marah-launcher.exe` |

The folder holds:

- `OFL-Inter.txt` and `OFL-Tajawal.txt`, the licences of the fonts.
- `THIRD-PARTY.md`, for the Rust crates.
- `libmpv/`, from `scripts/bundle-libmpv.sh`. The deb, rpm and Arch packages have none: see
  [libmpv](#libmpv).
- `libarchive/`, from `scripts/bundle-libarchive.sh`.

Setting `MARAH_NO_LIBMPV=1` or `MARAH_NO_LIBARCHIVE=1` leaves a library, and its folder, out of
a package.

The launcher loads both libraries at run time with `libloading`. It does not link to them. The
search order puts the package's copy first: `lib/` beside the binary on Linux,
`Contents/Frameworks` in the macOS app, and the binary's own folder on Windows.

### Fonts

Both built-in themes, Default and Classic, carry Inter and Tajawal in the binary. Each theme's
`theme.toml` lists the font files, and the licence files sit beside them in
`plugins/themes/default/theme/fonts/` and `plugins/themes/classic/theme/fonts/`.
`scripts/package.sh` copies Default's licence files into `licenses/`.

| Font | Copyright line in the licence file | Licence |
| --- | --- | --- |
| Inter | Copyright (c) 2016 The Inter Project Authors (<https://github.com/rsms/inter>) | SIL Open Font License 1.1 (`OFL-Inter.txt`) |
| Tajawal | Copyright 2018 Boutros International (<http://www.boutrosfonts.com>) | SIL Open Font License 1.1 (`OFL-Tajawal.txt`) |

### Pictures and sounds

Each built-in emulator plugin carries its system's icon and cover in the binary
(`plugins/emulators/*/assets/`, made by `scripts/make-system-art.sh`). The icons come from the
SVGs in `assets/system-art/`. The covers show third-party game art and console marks, such as
the PlayStation, Nintendo Switch, Windows and Steam logos and characters from their games. The
repository holds no licence information for the covers. The marks belong to their owners.

`scripts/make-sounds.py` writes the `select`, `back`, `error` and `start` sounds as synthesized
tones. Both built-in themes ship the same five sound files.

### libmpv

libmpv plays the trailers. `scripts/bundle-libmpv.sh` copies it and the libraries it needs into
the package. It leaves out the system's own libraries.

The deb, rpm and Arch packages don't bundle it. They depend on the distribution's libmpv
(`libmpv2` or `libmpv1`, `mpv-libs`, `mpv`), and `packaging/linux/build-packages.sh` leaves the
bundled copy out. The tarball, the AppImage, the macOS app and the Windows packages bundle it.

| System | libmpv comes from |
| --- | --- |
| Linux | The build machine's package. On Ubuntu, `libmpv-dev`; on Fedora, `mpv-libs`. The release builds on Ubuntu 22.04 (`.github/workflows/package.yml`). |
| macOS | Homebrew's `mpv` formula, with its libraries copied by `dylibbundler`. |
| Windows | shinchiro's `mpv-winbuild-cmake` build: a pinned file with a pinned SHA-256 (`.github/actions/libmpv/action.yml`). This repository's `deps` release keeps a copy. |

The script writes `licenses/libmpv/` with:

- `Copyright`, `LICENSE.GPL` and `LICENSE.LGPL`, fetched from mpv's source at the version
  shipped.
- `ffmpeg/` with FFmpeg's licence files, fetched from FFmpeg's source. On Windows the
  FFmpeg commit is the one the `libmpv` action names (`LIBMPV_FFMPEG`), or `master` when it is
  unset.
- `NOTICE`, which names the build and lists the files.
- `SOURCES`, which gives the address of the source of mpv and FFmpeg.

The `NOTICE` says: libmpv is mpv's library, under the GNU General Public License, version 2 or
later, with some parts under the LGPL named in `Copyright`. FFmpeg is under the LGPL or the
GPL, and the libraries libmpv uses each keep their own licence.

### libarchive

libarchive unpacks RAR, tar.xz, tar.bz2, tar.zst, ISO and cab archives and some 7z archives:
see [contributors/archives-and-pc-games.md](contributors/archives-and-pc-games.md).
`crates/app/Cargo.toml` says why the launcher uses it for RAR instead of the `unrar` crate:
UnRAR's licence adds terms the GPL doesn't allow.

| System | libarchive comes from |
| --- | --- |
| Linux and macOS | Built from source by `scripts/bundle-libarchive.sh`: release 3.8.9 (`VERSION` in the script), checked against `SHA256` in the script. |
| Windows | MSYS2's `mingw-w64-ucrt-x86_64-libarchive` package and the DLLs it loads. `.github/actions/libarchive/action.yml` does not pin these packages, so it records their names and versions in the package's `NOTICE` and `SOURCES`. |

The build on Linux and macOS turns off XML, ACL, crypto, iconv and other optional parts (the
`cmake` flags in the script). The script says this leaves only compression codecs, so nothing
under the LGPL comes with it.

The script writes `licenses/libarchive/` with:

- `COPYING`, libarchive's licence: BSD 2-clause, as `crates/app/src/libarchive.rs` and the
  `NOTICE` say.
- The licences of the codecs. On macOS: zstd's and lz4's `LICENSE`. On Linux: Debian's
  `copyright` files for the codec packages, or on an rpm system the licence files of the owning
  packages. On Windows: the `licenses` folder of the MSYS2 build.
- `NOTICE`, which names the build and lists the files. It says zstd and lz4 are BSD, xz is
  0BSD, bzip2 has its own BSD-style licence, and that on Windows other libraries come too.
- `SOURCES`, with the address of the libarchive release. On Windows the action writes the MSYS2
  source package addresses.

## Source of the shipped libraries

`scripts/fetch-sources.sh <package folder>` reads every `licenses/*/SOURCES` file in a package
and downloads each address into `dist/<package>-third-party-sources.tar`. The release workflow
runs it for each package and attaches the tar to the release beside the package. On macOS it
runs once, for the universal app.

## Test data

`crates/app/testdata` holds small archives for the tests. Its `README.md` says `version.rar` and
`crypted.rar` come from the test data of the `unrar` crate 0.5.8
(<https://github.com/muja/unrar.rs>, MIT OR Apache-2.0). No UnRAR code is in the repository. The
other files there were made for these tests with bsdtar, 7-Zip and Info-ZIP.

## Programs the launcher downloads at run time

These are not part of any package. The launcher downloads them when the player installs them or
agrees to the download. They keep their own licences, and this repository holds none of them.

- KytyPS5, from the GitHub releases of `KytyPS5/KytyPS5`.
- shadPS4, from the GitHub releases of `shadps4-emu/shadPS4`.
- Lichii, from the GitHub releases of `zortos293/Lichii`.
- The PS4 system modules and fonts that shadPS4 can use, from the retrobios collection
  (`Abdess/retrobios`, at a fixed commit). They are Sony's files.
- The Switch firmware, from the GitHub releases of `THZoria/NX_Firmware`, and its keys, from
  `files.prodkeys.net`. They are Nintendo's files.
- umu-launcher, from the GitHub releases of `Open-Wine-Components/umu-launcher`. It downloads
  Proton (GE-Proton) by itself when a Windows game runs.

## Marah OS images

The Marah OS images ship the launcher with Fedora's packages. Their licences are recorded in
[MarahHQ/marah-os](https://github.com/MarahHQ/marah-os), not here.

## Limitations

- **`oozextract` may be missing from the shipped licence list.** `THIRD-PARTY.md` most likely
  leaves it out (see [Rust crates](#rust-crates)), and no package ships
  `third_party/oozextract/NOTICE.md`. The crate declares MIT, but this repository holds no MIT
  licence text for it. This page did not run `cargo about` to check.
- **The `libmpv/` folder holds licence texts for mpv and FFmpeg only.** Other libraries that
  libmpv needs are copied into the package, but the script fetches no licence text for them.
  The `NOTICE` says they each keep their own licence.
- **The source of libmpv's other libraries is not in the sources tar.** `SOURCES` lists mpv and
  FFmpeg only.
- **The Windows libarchive packages are not pinned.** The MSYS2 packages are the ones the runner
  installs on that day. The package's `NOTICE` and `SOURCES` record the versions.
- **Some pictures and sounds have no recorded licence.** The themes' `images/` (such as
  `logo.png` and the Default theme's "All games" pictures), the `move.wav` sound, the system
  covers, and the icons in `assets/` (`marah-launcher.png`, `marah-launcher.ico`,
  `marah-launcher.icns`, `logo.svg`) have none. This page does not guess who owns them.
- **The bundled catalogs have no recorded licence.** `assets/catalog/` and
  `assets/catalog-sources.json` hold lists of releases. The repository states no licence for
  them.
- **Not checked.** This page was written from the files. No package was built, and no crate
  licence was checked against the lock file.
