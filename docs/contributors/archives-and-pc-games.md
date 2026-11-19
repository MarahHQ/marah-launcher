# Archives and PC games

This page says what the launcher does with a finished download: it unpacks the archives in it,
and the PC plugin installs the game, finds the file that starts it, and runs it on Play. It is
for contributors who change the unpack, the archive adapter or the PC plugin. Players get a
short version in [Downloads](../players/downloads.md) and [Emulators](../players/emulators.md).

The launcher never runs anything in a download while it unpacks. The core and its ports are in
[architecture.md](architecture.md). The plugin contract is in
[the plugin reference](../plugins/reference.md).

## Where the work happens

| Part | Where | Job |
| --- | --- | --- |
| Archive sets | `crates/core/src/archives.rs` | Finds the sets in a download, by name or by bytes. |
| Archive plan | `crates/core/src/archive_plan.rs` | Decides what an unpack may write. |
| Unpack flow | `crates/core/src/game_installs/unpacking.rs` | Runs the sets one by one and keeps the state. |
| `Archives` port | `crates/core/src/ports.rs` | The one call, `unpack`, that an adapter implements. |
| Real adapter | `crates/app/src/archives.rs`, `libarchive.rs` | `HostArchives`: the format readers. |
| Fake adapter | `crates/testkit/src/archives.rs` | `FakeArchives`: the same plan, no files. |
| PC plugin | `plugins/emulators/pc/` | Installs, lists and runs PC games. |
| Shared code | `plugins/shared/emulator-kit/` | Reads a download's folder, and finds the archive sets' names. |

## Archive sets

An archive set is one archive, or one archive cut into parts. `part_of` in
`crates/core/src/archives.rs` reads a file name and says which set and which part it is.

| Name | Kind | Part |
| --- | --- | --- |
| `game.rar` | `Rar` | 1 |
| `game.part1.rar`, `game.part2.rar` | `Rar` | 1, 2 |
| `game.rar` with `game.r00`, `game.r01` | `Rar` | 1, then 2, 3 |
| `game.7z`, `game.7z.001`, `game.7z.002` | `SevenZ` | 1, then 1, 2 |
| `game.zip`, `game.tar` | `Zip`, `Tar` | 1 |
| `.tar.gz`, `.tgz` | `TarGz` | 1 |
| `.tar.xz`, `.txz` | `TarXz` | 1 |
| `.tar.bz2`, `.tbz2`, `.tbz` | `TarBz2` | 1 |
| `.tar.zst`, `.tzst` | `TarZst` | 1 |
| `.iso`, `.cab` | `Iso`, `Cab` | 1 |

Names are matched in lower case. Only RAR and 7z can be cut into parts. A set has at most 999
parts (`MAX_PARTS`).

Two more kinds are found by their bytes, not their names. A file that ends in `.sh` or `.run`
is a script installer when:

- It is a `Gog` installer: a `.sh` file that starts with `#!`, is at least 1024 bytes long, and
  has a zip end record (`PK\x05\x06`) in its last 64 KiB.
- It is a `Script` installer: it starts with `#!` and states its archive's size on an `arcsz=`
  line within its first 128 KiB (`SCRIPT_HEAD`). The archive is the last bytes of the file, and
  `embedded_archive` finds where it starts. Kron4ek's YAD installers are packed this way.

The launcher never runs these scripts. It reads the archive inside.

`scan` looks at the top of the download and one folder down. A set deeper than that is left
alone, and so is the folder `unpacked`. Two files that are the same part of a set, or a missing
part, give the set a `problem`. Nothing is written for such a set, and the download fails with
the problem as its text
(`a_set_with_a_missing_part_fails_before_anything_is_unpacked` in
`tests/tests/seams/release_archives.rs`).

## When the launcher unpacks

`Unpack::of` scans a download when its torrent finishes (`torrents/mod.rs`). The download then
has one of these unpack phases (`UnpackPhase`, by the word the view gets):

| Phase | Meaning |
| --- | --- |
| empty | No archive set. The download installs as it is. |
| `waiting` | Sets found, not unpacked yet. |
| `unpacking` | An unpack runs. |
| `unpacked` | Every set is unpacked. |
| `failed` | An unpack failed. The error says why. |
| `cancelled` | The player cancelled. |

An unpack starts in two ways:

- When a download finishes in `waiting`, the core starts the unpack at once (`torrent_event`
  in `release_downloads.rs`).
- When the player presses Install on a download in `waiting`, `failed` or `cancelled`, the
  unpack runs first and the install after it (`install_download` in `game_installs/mod.rs`). In
  `unpacking`, Install does nothing.

Each set goes to `unpacked/<folder>/<name>/` inside the download, where `<folder>` is the set's
folder, if it has one. For example, `a/game.rar` unpacks to `unpacked/a/game/`.

`run_unpack` in `game_installs/unpacking.rs` works set by set:

1. It skips a set whose destination folder exists already.
2. It makes a work folder, `unpacked/<name>.part`, and empties it.
3. It reads the free space of the disk that holds the work folder.
4. It calls `Archives::unpack` into the work folder.
5. On success, it renames the work folder to the destination.
6. On failure, it removes the work folder and every set this run already unpacked, then reports
   the error.

The progress is the share of the bytes of all sets, saved at each whole percent.

Cancel sets a flag. The adapter stops soon after and returns `ArchiveError::Cancelled`. The
phase becomes `cancelled`, and the download stays.

If the launcher closes during an unpack, `Unpack::restored` runs at the next start. It sets the
phase to `failed`, with a text that says Install unpacks again, and removes the work folders.

Remove unpacked files (`remove_unpacked`) deletes the folder `unpacked` and sets the phase back
to `waiting`. The torrent's own files stay, and so does the seeding. It does nothing while an
unpack runs. When an install moves the unpacked folder into the game folder instead of copying
it, the phase also goes back to `waiting`, because the unpacked copy is gone.

## The archive plan

Every `Archives` adapter reads an archive twice. The first pass lists it, and `plan_unpack`
checks the listing. The second pass writes only what the plan holds.

`plan_unpack` refuses the whole archive at the first entry that breaks a rule:

| Rule | Error |
| --- | --- |
| An entry needs a password. | `Password` |
| A path is absolute, is empty, or climbs out with `..`. | `Escapes` with `Leaves` |
| A link points to an absolute path, or climbs out with `..`. | `Escapes` with `LinksOut` |
| A link climbs out once the archive's other links on its way are followed. | `Escapes` with `LinksOut` |
| A link goes through more than 40 links (`MOST_LINKS`), as a link loop does. | `Escapes` with `LinksOut` |
| An entry sits under a link's path, or at a link's path when it is not that link. | `Escapes` with `ThroughLink` |
| A hard link names a target that is not a plain path inside the folder. | `Escapes` with `LinksOut` |
| A hard link names one of the archive's links, or a path through one. | `Escapes` with `LinksOut` |
| The files' bytes and the spare don't fit in the free space. | `NoRoom` |

The spare is `UNPACK_SPARE`, 64 MiB. A disk that doesn't report its free space counts as having
room. The plan leaves out the archive's own top, an entry such as `./` that some tars list
first. It also leaves out devices and pipes (`EntryKind::Other`).

The link rules look at the whole listing, not only at the entries before, because a reader may
write the entries in another order. For example, link `d/l` points to `..` and link `x` points
to `d/l/..`. Each link alone stays inside. But `d/l` ends at the folder's top, so `x` ends one
folder above it, and the plan refuses `x`. A link may go through other links that stay inside,
as `Qt.framework/Qt` to `Versions/Current/Qt` does in a macOS framework.

A hard link to a link makes a second link at the hard link's path. For example, link `d/e/s`
points to `../..`, the folder's top. Hard link `h` names `d/e/s`, so `h` is a link to `../..` at
the top, which ends one folder above it. A hard link through a link may reach a link on the
disk the same way. The plan refuses both, even when the link stays inside.

The link rules match names that differ only in case, on every system: macOS and Windows find
`d/l` by the name `D/l`. On Linux the plan so refuses a little more than it must. The rules also
match a name in another Unicode form: `é` as one character and `e` with a combining accent are
one name on macOS, and one name to the plan.

The adapter checks the disk again during the second pass. `Work::place` in
`crates/app/src/archives.rs` refuses to write where a link is already on the disk
(`Escape::ThroughLink`), and where the path ends up outside the folder once links are followed
(`Escape::LeavesThroughLink`). If the second pass finds an entry the first pass didn't list, it
fails with "the archive changed while it was unpacked".

A file the adapter writes keeps its Unix mode, so a program stays a program. Windows has no
modes. Symbolic links are made on Unix only.

## Format readers

`HostArchives` picks a reader by the set's kind. A format reader decides nothing about safety:
the plan does.

| Kind | Reader |
| --- | --- |
| `Zip`, `Gog` | The `zip` crate. It finds a zip's start from its end, past the script. |
| `Tar`, `TarGz` | The `tar` and `flate2` crates. |
| `SevenZ` | The `sevenz-rust2` crate. A split 7z is read as one stream. |
| `Rar`, `TarXz`, `TarBz2`, `TarZst`, `Iso`, `Cab`, `Script` | libarchive. |
| `SevenZ` with a codec `sevenz-rust2` lacks | libarchive. |

`SEVEN_ELSEWHERE` lists the codecs that send a 7z to libarchive: BZip2, Zstd, Brotli, LZ4, LZS,
Lizard and Deflate64. A 7z listing that can't be read for lack of a codec goes there too.

### libarchive

The launcher loads libarchive at run time (`libarchive.rs`). It doesn't link to it.
`candidates` lists where it looks, the package's copy first, then the system's:

| System | Package copy | System copy |
| --- | --- | --- |
| Linux | `lib/libarchive.so.13`, `../lib/libarchive.so.13` | `libarchive.so.13`, `libarchive.so` |
| macOS | `lib/libarchive.13.dylib`, `../Frameworks/libarchive.13.dylib` | Homebrew paths, `libarchive.13.dylib`, `/usr/lib/libarchive.2.dylib` |
| Windows | `libarchive-13.dll`, `archive.dll` | the same names |

The package paths are relative to the folder of the launcher's program. The library must read
RAR 5, or the launcher counts it as missing and tries the next place.

If no libarchive loads, the unpack fails with `ArchiveError::Missing` and a hint from
`install_hint`: `brew install libarchive` on macOS, one of the launcher's packages on Windows,
and `libarchive13` on Debian and Ubuntu or `libarchive` on Fedora. The packages ship a copy:
see [the third-party notices](../third-party-notices.md).

### unar

A `Rar` set or a `Script` archive that libarchive can't read comes back as
`ArchiveError::Unsupported`. `unpack` in `crates/app/src/archives.rs` then tries `unar` (The
Unarchiver), for example for a RAR 5 with a dictionary bigger than libarchive takes. It does
this for those two kinds only, and not for a password, a path or a lack of room.

The launcher needs both `lsar` and `unar` on the `PATH`. If either is missing, the error says
"Install unar to unpack it".

The flow is the same: `lsar -j -jss -nr` lists, `plan_unpack` checks, then `unar` writes, with
these arguments (`Unar::unpack`):

```text
-f -D -nr -o <folder> <archive>
```

`unar` reads only a whole file. For a `Script` archive, the launcher copies the archive out of
the script to a file beside the work folder, and removes the copy at the end. The copy needs
room on the same disk, and the plan counts it. The launcher reads the progress from the lines
`unar` prints. After `unar` ends, it sets the Unix mode that a RAR 5 made on Unix keeps,
because `unar` leaves the program flag off.

### Errors

`ArchiveError` says why an unpack failed. The core shows its text. An adapter picks a fallback
reader by the kind, never by the text.

| Kind | Meaning |
| --- | --- |
| `Password` | An entry needs a password. The launcher can't ask for one. |
| `Escapes` | A path or a link gets out of the folder, or an entry would be written through a link. |
| `NoRoom` | The disk is too small. The text gives both sizes in GB. |
| `Unsupported` | The reader can't read this. Another reader may. |
| `Missing` | A library or a program is not installed. The text says which. |
| `Corrupt` | The archive is damaged, cut short, or changed during the unpack. |
| `Io` | Reading the parts or writing the folder failed. |
| `Cancelled` | The player cancelled. |

## Unpacking a plugin's build

Emulator builds have a second unpack, in `crates/core/src/unpack.rs`. It doesn't use the
`Archives` port. A file in a plugin's download plan can carry an `Unpack` with an
`ArchiveFormat` (`TarGz`, `Zip` or `AppImageInZip`) and a folder to unpack into. The core
unpacks once the file passes its checks. It unpacks into a sibling folder named
`<folder>.unpacking` first and replaces the old folder at the end, so a failure leaves the old
contents. It applies the same path rules (`inside`, `link_stays_inside`, `stays_inside`).

For `AppImageInZip`, the core takes the one AppImage out of the zip and runs it with
`--appimage-extract`. That runs on Linux only, and only when the plugin's
`permissions.process` allows the file. The run is killed after `EXTRACT_TIMEOUT` (600 seconds).
The result must be a `squashfs-root` folder.

## Testing an adapter

`contract::archives` in `crates/testkit/src/contract.rs` checks the refusals every `Archives`
adapter must make. The adapter gives a fixture that builds archives. The contract checks that:

- A plain archive unpacks, and the progress ends at the files' bytes.
- A `..` path is refused, and nothing is written.
- An absolute path is refused.
- A link to `../../../etc/passwd` or to `/etc/passwd` is refused.
- A link that climbs out through another link is refused, and nothing is written.
- A file under a link is refused, and nothing is written.
- An archive that needs a password is refused. When the fixture can't make one, the check is
  skipped and says so.
- An archive bigger than the free space is refused.

The app runs the contract on `HostArchives`, with real archives (`crates/app/testdata/`). The
test kit runs it on `FakeArchives`, which calls the same `plan_unpack`. The whole flow is tested
in `tests/tests/seams/release_archives.rs`, and plugin builds in `tests/tests/seams/unpack.rs`.

## PC games

The PC plugin is `emu-pc` (`plugins/emulators/pc/plugin.toml`). It runs on Linux, macOS and
Windows, with the capabilities `catalog`, `launcher`, `slots`, `game_installer`,
`configurator` and `backup`. Its backup item is each game's Wine prefix, as a large item.

PC games install from downloads only. The plugin's `candidates` returns nothing, so the plugin
offers no game from a packages folder.

### Install

`begin` in `plugins/emulators/pc/src/install.rs` installs from a download folder, which the core
has unpacked by then. The plugin works in these steps:

1. It scans the download (`Download::scan` in the emulator kit's `installs.rs`). A download
   deeper than `SCAN_DEPTH` (64 folders), or with more than `SCAN_FILES` (1,000,000) files, is
   refused.
2. It decides which archive sets are the release's packaging, and puts the files the core
   unpacked from them in their place:
   - A script installer is always packaging.
   - A set with nothing beside it is packaging.
   - A set beside a start script is packaging only when what is inside holds a program.
   - A set beside a program is not packaging. It is the game's own data, copied as it is.
3. It picks the game's name: GOG's `data/noarch/gameinfo` (`GOG_INFO`) first, else the
   installer's own folder, else the torrent's top folder, else the set's name, else "Game".
   `display_name` cleans tags and versions off the name.
4. It reads the first 4096 bytes (`probe::HEAD`) of up to 4000 files (`PROBED`), nearest the
   game's folder first, and ranks them. A portable Wine wrapper (Kron4ek's) names its game on
   the sixth line of `game_info/game_info.txt`, beside its start script.
5. It refuses a download with nothing that starts a game: "Nothing in this download starts a
   game".
6. It writes into `PC/<game>.part` and renames it to `PC/<game>` at the end. If `PC/<game>`
   exists, the install fails with "is already installed".
7. When one unpacked folder alone is the game, and the unpacked copy is the host's own, the
   plugin asks the host to move the whole folder instead of copying it. The host copies when it
   can't move. A bundled Proton needs the links and empty folders that a move keeps.
8. Off Windows, it sets the program flag on every file that should have it, because downloads
   drop the flag.
9. It writes the game file.

The game goes to the folder the player picked for the install, else to `PC/` in the first game
folder. With no game folder, the install fails with "add a game folder in Settings first".

### What a file is

The plugin reads a file's first bytes, not its name. `probe` in `probe.rs` tells:

| First bytes | Probe |
| --- | --- |
| `#!` | A script |
| ELF, x86 or x86-64, executable or position-independent with an interpreter | A Linux program |
| ELF with the AppImage marker | An AppImage |
| `MZ` with a `PE` header | A Windows program |
| Mach-O, or a universal binary | A Mac program |

An ELF for ARM, a big-endian ELF and a library are `Other`. Each file that can start the game
has a kind (`Kind`): `script`, `appimage`, `native`, `macapp` or `windows`.

### What starts the game

`rank` puts the files in order. Helpers come last: the names in `helper`, such as `setup`,
`install`, `uninstall`, `redist`, `crashhandler`, `update` and `easyanticheat`, mark one. Then
the order is:

1. A start script by its name (`START_NAMES`: `start`, `start.sh`, `start-game.sh`,
   `start_game.sh`, `play.sh`, `run.sh`, `launch.sh`, `game.sh`).
2. An AppImage.
3. A Mac program inside a `.app`.
4. A Linux program.
5. Another Mac program.
6. Another script.
7. A Windows program.

Within one group, a file nearer the game's folder comes first, a 64-bit program comes before a
32-bit one, and then the path decides. `rank` leaves out libraries (`.so`, `.dll`), and
programs in the folders of bundled runtimes (`RUNTIMES`, such as `lib/`, `wine/bin`,
`runtime/` and `jre/`), except start scripts.

### The game file

The game file is `launcher_game.json` (`GAME_FILE`; see
[architecture.md](architecture.md#data-folder)). The PC plugin's part of it is `title` and
`starts`: every file that can start the game, best first. The rest is the core's, and the plugin
keeps it as it is when it writes. The file is at most 256 KiB (`MAX_GAME_FILE`). The plugin
drops any path that is not a safe path inside the folder.

The default start on a system is the first start script that runs there, else the first file
that runs there, else the first file.

| Kind | Runs on |
| --- | --- |
| `script`, `appimage`, `native` | Linux |
| `macapp` | macOS |
| `windows` | Every system. Off Windows it needs Proton or Wine. |

### The library

The plugin lists the folders in `PC/` of each game folder. A folder is a game when it has a game
file. A folder whose name starts with `.` or ends with `.part` is never a game.

A player can copy a game into `PC/` by hand. The first time the library sees the folder, the
plugin probes its files as an install does, writes the game file, and sets the program flag off
Windows. A folder with nothing that starts a game is left alone, and is not probed again until
the next run.

A `cover.jpg`, `cover.png` or `cover.webp` in the game's folder is its cover, in that order.

### Settings

A game has these settings in its Game Hub (`settings.rs`):

- **Starts**: the file Play starts. It must be one of the game file's `starts`.
- **Resolution**: Linux only. The choices are "Your screen" (`screen`) and sizes from 640 × 480
  to 1920 × 1080. Any size runs the game inside gamescope.
- **Sandbox**: Linux only, for a Windows game. On by default. Off, the game runs with the
  player's access to every file.

### Play

`spec` in `launch.rs` builds the launch. The file's kind decides how it runs:

| Kind | How it runs |
| --- | --- |
| `script` | As it is. If it doesn't start with `#!`, through `sh`. |
| `appimage` | With `APPIMAGE_EXTRACT_AND_RUN=1`, so it needs no FUSE. |
| `native`, `macapp` | As it is. |
| `windows` on Windows | As it is. |
| `windows` elsewhere | Through umu-launcher (Proton) or Wine. |

The working folder is the folder of the file that starts the game. A file this system can't run
fails with a message such as "This is a Linux game: it can't run on Windows".

On Linux, two more wrappers can apply:

- With a Resolution other than "Your screen", the game runs inside
  `gamescope -f -w <width> -h <height> --`. If gamescope is not installed, Play fails and says to
  install it or reset the Resolution.
- If `gamemoderun` is installed, the game runs through it, unless it runs in the sandbox.

### Windows games off Windows

`runner` in `runner.rs` picks what runs a Windows game:

1. On Linux, the umu-launcher this plugin installed.
2. On Linux, `umu-run` on the system.
3. `wine`, then `wine64`.

On macOS only Wine is tried. If nothing is found, Play fails and says what to install.

Each game gets its own Wine prefix in the plugin's data folder, `prefixes/<game id>`. With
umu-launcher, the plugin sets `GAMEID=umu-default` and `PROTONPATH=GE-Proton`, and
umu-launcher downloads and updates Proton by itself. When the system can't run 32-bit programs,
the plugin also sets `PROTON_USE_WOW64=1`.

The plugin page has a "Windows games" section. On Linux it has the button Install umu-launcher,
or Update umu-launcher (`install_umu`):

1. It asks GitHub's API for the latest release of `Open-Wine-Components/umu-launcher`.
2. It picks the asset named `umu-launcher-*-zipapp.tar` and the SHA-256 GitHub lists for it. If
   GitHub lists none, the install stops.
3. It downloads the tar and checks the SHA-256. A mismatch stops the install.
4. It takes `umu/umu-run` out of the tar, writes it beside the old one as `umu/umu-run.part`,
   sets the program flag, and renames it into place. A failed install leaves the old one.
5. It saves the release's tag in `umu/version`.

The zipapp needs only Python 3.

### The sandbox

On Linux, a Windows game runs in a sandbox, unless its Sandbox setting is off. The plugin asks
for it in the launch (`in_sandbox` in `launch.rs`, the `sandbox` field of `LaunchSpec`). The
host runs the whole command through bubblewrap (`bwrap`), and builds the sandbox in
`crates/core/src/sandbox.rs`.

The game sees:

- the system's programs, libraries and settings (`/usr`, `/etc`, `/opt`), read only;
- `/sys`, `/proc` and every device: the GPU, controllers, `ntsync`. `/dev/shm` is empty, so
  other programs' shared memory stays out;
- the X11 sockets, and the Wayland, PipeWire and PulseAudio sockets of the player's runtime
  folder;
- an empty home folder and an empty `/tmp`, which go when the game ends;
- its own folder and its Wine prefix (`prefixes/<game id>`), read and write;
- with umu-launcher, the plugin's `proton/` folder, read and write. The plugin sets
  `UMU_FOLDERS_PATH` to it, so umu-launcher downloads Proton and its runtime there once;
- with the umu-launcher this plugin installed, its `umu/` folder, read only.

The game doesn't see the rest of the home folder, other games, the plugin's other folders, or
the session D-Bus. The host makes the folders the game writes before it starts.

The game's `DBUS_SESSION_BUS_ADDRESS` points to the path `NO_BUS` in `sandbox.rs` (the
`sandbox_no_bus` of `brand.toml`), which is `/dev/null` in the sandbox. Each try to reach the
bus fails at once. The variable is not left unset: without it, libdbus starts a new
`dbus-launch` at each try, and Proton keeps trying until the process table is full. The path
exists because Proton's own container binds the bus's socket into itself.

The game keeps the system's process ids, so the launcher finds its windows and follows its
processes as before. bubblewrap runs the game in a user namespace of its own, which blocks it
from reading other programs' files, memory and environment through `/proc`.

If `bwrap` is not installed, Play fails and says to install bubblewrap or turn off Sandbox. The
deb, rpm and Arch packages recommend bubblewrap.

### Permissions

The plugin declares these in `plugins/emulators/pc/plugin.toml`:

```toml
process = ["folder:games", "umu/umu-run", "system:umu-run", "system:wine", "system:wine64", "system:sh", "system:gamescope", "system:gamemoderun"]
network = ["api.github.com", "github.com", "*.githubusercontent.com"]
```

The host runs bubblewrap itself, so `bwrap` needs no entry.

## Limitations

- **Archives with a password don't unpack.** The launcher can't ask for one, and the unpack
  fails with `Password`.
- **Only RAR and 7z can be cut into parts.** A split zip or tar is not one set.
- **Sets deeper than one folder are left alone.** `scan` reads the top of the download and one
  folder down.
- **A RAR 5 with a big dictionary needs `unar`.** libarchive can't read it, and no other reader
  tries. Without `unar` and `lsar`, the unpack fails with `Missing`.
- **Most formats need libarchive.** RAR, tar.xz, tar.bz2, tar.zst, ISO and cab all go through
  it. The packages ship a copy. A build from source uses the system's, which must read RAR 5.
- **Links are not made on Windows.** The adapter writes symbolic links on Unix only.
- **Script installers only unpack.** The launcher doesn't run them. A script installer that
  doesn't state its archive's size, and is not a GOG installer, is not found.
- **PC games install from downloads only.** A folder copied by hand into `PC/` works as an
  installed game.
- **The PC plugin gives no description, developer or art.** It reports a cover only from
  `cover.jpg`, `cover.png` or `cover.webp` in the game's folder. Other art comes from the
  catalog.
- **Resolution needs gamescope, and works on Linux only.**
- **Only Windows games on Linux run in the sandbox.** Linux games, scripts and AppImages run
  with the player's access: they save in the home folder, which the sandbox hides. macOS and
  Windows have no sandbox.
- **The sandbox keeps the network open.** A game may need it, and umu-launcher downloads Proton
  through it.
- **A game in the sandbox runs without GameMode.** GameMode talks to its daemon over the
  session D-Bus, which the sandbox hides.
- **A game in the sandbox can still type into other X11 windows** on a desktop where they share
  one X server. In the whole-screen session, gamescope gives games an X server of their own.
- **The first play in the sandbox downloads Proton again**, into the plugin's `proton/` folder:
  several hundred MB, once.
- **No test runs a real game in the sandbox.**
  `bubblewrap_shows_the_game_its_folders_and_hides_the_rest` in `sandbox.rs` runs a shell
  through the real bubblewrap. It is opt in, and the weekly workflow runs it.
- **No test here runs a real game.** The seam tests in `tests/tests/seams/pc.rs` run the plugin
  on a fake host for each system. CI runs them on Linux, and the weekly workflow on Windows.
  Nothing runs them on macOS.
