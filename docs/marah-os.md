# Marah Launcher and Marah OS

This page is for maintainers of Marah OS. Marah OS is a PC image that starts straight into
Marah Launcher, with no desktop behind it. So the screens the OS needs (Wi-Fi, Bluetooth, sound,
system updates, the install from the USB stick) are part of Marah Launcher, and they show only where
the OS needs them.

This page lists what Marah Launcher expects from the OS and what the OS expects from Marah Launcher. Change both
sides together. The OS's files named here (`Containerfile`, `files/…`, `live/…`) are in
[MarahHQ/marah-os](https://github.com/MarahHQ/marah-os); its `check-brand.sh` checks the paths in `brand.toml`'s `[os]`. How the images and the USB stick are built, tested and released is in
[MarahHQ/marah-os](https://github.com/MarahHQ/marah-os). Run the OS screens on a desktop with the session switches in
[development.md](development.md#session-switches).

## The parts

| Part | Where | What it does |
| --- | --- | --- |
| The image | `Containerfile` | Fedora bootc, minimal set, with gamescope, cage, greetd, NetworkManager, BlueZ, PipeWire and the Marah Launcher rpm. No desktop. |
| The NVIDIA edition | `nvidia/` | The image plus NVIDIA's driver |
| The USB stick | `live/`, `build-iso.sh` | The image started live from a stick, with an installer |
| The Marah Launcher session | `packaging/linux/marah-session` | Runs Marah Launcher as the whole screen. In the Marah Launcher rpm. |
| The system plugin | `plugins/system/host/` | Runs the system's tools for Settings → System |
| `Place` | `crates/core/src/system.rs` | Tells Marah Launcher where it runs |

## Where Marah Launcher runs

`Place::detect` reads three facts at start. Each variable counts only when its value is
exactly `1`. The variables are for tests and screenshots; the Marah Launcher session sets
`MARAH_SESSION=1` for real.

| `Place` field | True when | Pretend with |
| --- | --- | --- |
| `own_os` | `/usr/lib/marah-os/release` exists, or `stick` is true | `MARAH_OS=1` |
| `stick` | `own_os` is true, `/proc/cmdline` has `rd.live.image`, and `/usr/libexec/marah-os/install` exists | `MARAH_USB=1` |
| `whole_screen` | `MARAH_SESSION=1`, or `own_os`, or `stick` | `MARAH_SESSION=1` |

The view model carries them as `Vm.system.whole-screen`, `Vm.system.own-os` and
`Vm.system.stick`. The kit and the built-in themes read them:

| Field | What changes |
| --- | --- |
| `whole-screen` | The top bar shows the network, sound and Bluetooth buttons. Settings has the System tab, with Restart and Shut down when the plugin allows power. "Quit Marah Launcher" reads "Restart Marah Launcher". The power menu has Log out. |
| `marah-os` | Log out goes away (`kit/power-menu.slint`, `kit/settings-layout.slint`). greetd would log the player in again at once. |
| `stick` | "Welcome to Marah OS" and the install screens show (`kit/install-os.slint`). Settings has the install row. |

### The files the image writes

The image's build writes two files (`Containerfile`):

| File | Holds | Who reads it |
| --- | --- | --- |
| `/usr/lib/marah-os/release` | One line: `marah-launcher --version` of the Marah Launcher in the image | Marah Launcher tests that it exists (`MARAH_OS_MARKER` in `launcher_update.rs`, `MARAH_OS` in `plugins/system/host/src/linux.rs`). It does not read the text. |
| `/usr/lib/marah-os/image` | Where the PC updates from, without a tag: `ghcr.io/<owner>/marah-os` | The OS: `marah-os/update`, the stick's installer, and the stick's build, which writes it into Anaconda's kickstart. Not Marah Launcher. |

## What the OS asks of Marah Launcher

`marah-launcher --supports <feature>` exits 0 when this Marah Launcher has the feature, else 1 (`supports` in
`crates/app/src/args.rs`). An older Marah Launcher without the option exits with "unknown option"
and code 2, which a caller counts as no.

| Feature | Means | Checked by |
| --- | --- | --- |
| `gamescope` | Marah Launcher runs in gamescope and tags the windows gamescope shows | `marah-session` before each start; `os.yml` when it checks an image |
| `system-settings` | Settings → System for Marah OS | Nothing in the repository |
| `os-install` | The stick's welcome and install screens | `build-iso.sh`: it stops when the Marah Launcher in the image answers 1 |

`marah-launcher --version` prints one line, `marah-launcher <version>`. The image stores it in the marker file.

A new capability gets a new `--supports` name. The OS checks for it before it relies on it.
Marah Launcher must keep working on an older OS: a missing tool, file or rule shows as unavailable in
Settings, with the reason.

## The Marah Launcher session

greetd logs in the user `player` at boot and runs `/usr/bin/marah-session` on terminal 1, for
the initial and the default session (`files/etc/greetd/config.toml`). The user comes from
`files/usr/lib/sysusers.d/marah-os.conf`: uid 1000, no password, in the `gamemode` group.
The Marah Launcher rpm installs the session (`packaging/linux/nfpm.yaml`), and
`packaging/linux/marah-session.desktop` lists it as "Marah Launcher" for display managers on any Linux.

`packaging/linux/marah-session` does this:

1. Reads `/usr/share/marah-launcher/session.conf`, then `/etc/marah-launcher/session.conf`.
2. Moves a log over `MARAH_LOG_MAX_MB` to `<name>.previous.log`.
3. Exports `MARAH_SESSION=1`, and `SDL_VIDEO_DRIVER=x11`, `SDL_VIDEODRIVER=x11`,
   `QT_QPA_PLATFORM=xcb` and `GDK_BACKEND=x11`. Games inherit them: Marah Launcher and the games run on
   gamescope's X server.
4. Waits for the network with `nm-online`, up to `MARAH_WAIT_NETWORK` seconds, then starts
   anyway.
5. Runs `gamescope <MARAH_GAMESCOPE_ARGS> -- <MARAH_BIN>`, with all output appended to the log.
6. Starts Marah Launcher again when it ends, unless Marah Launcher wrote `log out`.

It runs Marah Launcher in `cage -s` instead of gamescope when gamescope is not installed,
`marah-launcher --supports gamescope` fails, or the last two gamescope runs failed (the watchdog ended
them, or they ended in under 15 seconds). With neither gamescope nor cage installed, it runs
Marah Launcher in the current desktop when there is one, else it exits with an error.

The settings are in `packaging/linux/session.conf`. A player copies a line to
`/etc/marah-launcher/session.conf`, which package upgrades leave alone.

| Name | Default | Meaning |
| --- | --- | --- |
| `MARAH_BIN` | `/usr/bin/marah-launcher` | The launcher to run |
| `MARAH_GAMESCOPE_ARGS` | `-e --xwayland-count 1` | gamescope's options. `-e` is its Steam mode: Marah Launcher and the games swap with gamescope's focus atoms. |
| `MARAH_WATCHDOG` | `90` | Seconds to wait for `window shown` before starting Marah Launcher again |
| `MARAH_WAIT_NETWORK` | `5` | Seconds to wait for the network at the start |
| `MARAH_LOG` | `${XDG_CACHE_HOME:-$HOME/.cache}/marah-session.log` | Where the session and Marah Launcher write |
| `MARAH_LOG_MAX_MB` | `10` | The size at which the log moves to `marah-session.previous.log` |

### Lines from Marah Launcher to the session

Marah Launcher writes two lines to standard output (`Platform::announce`, used in
`crates/ui-host/src/shell.rs`). The session reads them in the log.

| Line | Marah Launcher writes it | The session |
| --- | --- | --- |
| `window shown` | Once, on the first tick where the window has a size | Marks the start as good. Without it after `MARAH_WATCHDOG` seconds, it ends the display server and starts again. |
| `log out` | When the player picks Log out, once the running game and Steam's client (when a launch started it) closed, just before Marah Launcher quits | Ends the session, back to the login screen. When gamescope is still up `MARAH_LOGOUT_GRACE` seconds after Marah Launcher ended, logind ends the Marah Launcher session (`loginctl terminate-session`); outside a Marah Launcher login session the script ends gamescope itself. |

After `window shown`, the app shell asks the platform to claim the window on each tick until
that works. Under gamescope, `crates/app/src/desktop.rs` claims it with `xprop`: it tags
the windows with a `STEAM_GAME` id and sets `GAMESCOPECTRL_BASELAYER_APPID` on the root
window. A game in front keeps the windows it opens after it showed: each new window of the
game's processes gets the game's id (`keep_game`). The image installs `xprop` and `xdotool`
for this.

## Settings → System

The built-in system plugin (`plugins/system/host/`) does the work. It declares `network`,
`vpn`, `bluetooth`, `audio`, `power`, `os_updates` and `os_install`. On macOS and Windows it
provides only `vpn` and `power`. The programs it may run are `permissions.process` in
`plugins/system/host/plugin.toml`.

| Marah Launcher feature | Marah Launcher runs | The image provides |
| --- | --- | --- |
| Internet, Wi-Fi | `nmcli` | NetworkManager, `wpa_supplicant`, and a polkit rule |
| Bluetooth | `bluetoothctl` | BlueZ |
| Sound output | `pactl` | PipeWire with its PulseAudio server, `pulseaudio-utils` |
| Restart, Shut down | `systemctl reboot`, `systemctl poweroff` | systemd |
| System update state | `rpm-ostree status --json` | `rpm-ostree` |
| System update, "check now" | `systemctl start --no-block marah-os-update.service` | The update service and a polkit rule |
| The install from the stick | `marah-os-install` | The stick's installer |

A missing tool shows as unavailable with its name, such as "NetworkManager (nmcli) isn't
installed" (`plugins/system/host/src/linux.rs`).

### polkit rules

Each rule applies only to `player` in a local, active session.

| Action | Allowed | File |
| --- | --- | --- |
| `org.freedesktop.NetworkManager.*` | Everything | `files/usr/share/polkit-1/rules.d/50-marah-os.rules` |
| `org.freedesktop.systemd1.manage-units` | Unit `marah-os-update.service`, verb `start` | The same file |
| `org.freedesktop.policykit.exec` | Program `/usr/bin/marah-os-install` or `/usr/libexec/marah-os/install` | `live/files/usr/share/polkit-1/rules.d/50-marah-os-live.rules` (stick only) |

Joining a new Wi-Fi network saves a system-wide connection, which polkit keeps for an
administrator. The first rule lifts that for `player`. `systemctl start` asks polkit itself,
so it needs no `pkexec`. For `pkexec`, current polkit reports the resolved path as `program`
and older polkit the given path, so the rule allows both.

The plugin lists `system:marah-os-install` under `permissions.polkit`, so Marah Launcher runs the
installer through `pkexec`.

## System updates

The OS updates as a whole image with bootc. The previous version stays in the boot menu.

`marah-os-update.timer` starts `marah-os-update.service` 3 minutes after boot and then every
day, with up to 10 minutes of random delay. A failed run (no internet, say) runs again after
15 minutes. The service does not run on the stick (`ConditionKernelCommandLine=!rd.live.image`).

The service runs `/usr/libexec/marah-os/update`. It only stages the next version and never
restarts the PC: a game may be running. For the same reason the image masks Fedora's
`bootc-fetch-apply-updates.timer`, which restarts the PC by itself. The script also picks the
edition by the graphics card. [MarahHQ/marah-os](https://github.com/MarahHQ/marah-os) has the NVIDIA rules.

Marah Launcher reads the state with `rpm-ostree status --json` (`OsUpdates::status`). Settings shows:

| State | Marah Launcher shows |
| --- | --- |
| A deployment is staged | "Restart to update to `<version>`" |
| None is staged | "Marah OS is up to date" |
| The output can't be read | "Couldn't look for an update: rpm-ostree's status couldn't be read" |

Without the marker file, `OsUpdates::status` fails with "this isn't Marah OS", and the update
row does not show.

`crates/core/src/view.rs` has the text "Looking for an update…" for `OsUpdate::Checking`,
but the Linux plugin never returns that state. A running check shows no state of its own.

### Marah Launcher's own update

On Marah OS, Marah Launcher comes with the image and never downloads its own update.
`Install::detect` in `crates/core/src/launcher_update.rs` returns `MarahOs` when the file
`packaged` sits beside the binary and the marker file exists. Then `package_key` returns
nothing, and `apply_launcher_update` in `crates/app/src/update.rs` does nothing.

The "Marah Launcher updates" row in Settings still reads `latest.json`. When a newer Marah Launcher exists, it
shows "Marah Launcher `<version>` is out · it comes with the next system update", and the core starts
the same check as "check now" (`check_launcher_update` in `crates/core/src/system.rs`).

The image installs the Marah Launcher rpm of a release (`fetch-marah-launcher.sh` and
`marah-launcher.txt` in MarahHQ/marah-os). `package.yml` names each new Marah Launcher release in
that `marah-launcher.txt`, with the Marah Bot's token, and starts that repository's `os.yml`, so the
images are rebuilt with it. See [development.md](development.md#commits-and-releases).

## The install from the USB stick

The stick holds a copy of the minimal image, so an install needs no internet. It installs in
two ways: erase a disk, from Marah Launcher, or choose partitions, in Fedora's installer (Anaconda).
[MarahHQ/marah-os](https://github.com/MarahHQ/marah-os) covers the stick's build, its boot menu and the Anaconda path.
This section is the protocol between Marah Launcher and the installer.

The installer is `live/files/usr/libexec/marah-os/install`. Plugins run only `system:<name>`
programs, so Marah Launcher calls it as `marah-os-install`, a link in `/usr/bin` to
`../libexec/marah-os/install`.

### `marah-os-install --disks`

Runs as `player`. It prints one line per disk it can install on:

```
disk PATH SIZE_BYTES WINDOWS(0/1) USB(0/1) MODEL…
```

- It leaves out the stick, read-only disks, and names that start with `zram`, `loop` or `sr`.
- It leaves out disks smaller than `MARAH_OS_MIN_DISK` bytes (default 40 × 10^9).
- `WINDOWS` is 1 when the disk has an NTFS or BitLocker partition.
- `USB` is 1 when the disk's transport is USB.
- `MODEL` is the rest of the line and may hold spaces. It is `Disk` when the disk has none.

The plugin reads the lines with `parse::disks` in `plugins/system/host/src/parse.rs`. Without
`/usr/libexec/marah-os/install`, it lists no disks.

### `marah-os-install DISK`

Marah Launcher runs it through `pkexec`. It must run as root, and it refuses a disk that `--disks` does
not list. It erases the disk and installs with `bootc install to-disk --wipe`. It prints one
line at a time:

| Line | Meaning |
| --- | --- |
| `progress PERCENT TEXT` | Progress |
| `done TAG [mok] [nvidia-later]` | The install ended. `TAG` is the edition, `minimal` or `minimal-nvidia`. `mok`: Secure Boot's key screen comes at the restart. `nvidia-later`: the PC has an NVIDIA card and gets the driver on the installed system. |
| `error TEXT` | The install failed. The exit code is not 0. |

Marah Launcher ignores every other line (`parse::install_line`). bootc's own output goes to standard
error. When the installer ends without `done` or `error`, Marah Launcher shows "The installer stopped
without finishing". Exit codes 126 and 127 show "The installer wasn't allowed to run".

The installer also copies the Wi-Fi connections saved on the stick into the new system.

### `marah-os-install --choose-partitions`

Marah Launcher's "Choose partitions instead" runs it through `pkexec`. It switches the running stick to
`marah-os-anaconda.target` with `systemctl --no-block isolate`, and exits 0 once the switch has
started. greetd and Marah Launcher stop right after. When it can't (not root, not on the stick, or the
switch refused), it prints one `error TEXT` line and exits non-zero. Marah Launcher shows that text.

### Changing the protocol

The installer may add words to a line. Marah Launcher ignores words it does not know. Do not change or
remove the words above: an installed Marah Launcher must read a newer stick, and a newer Marah Launcher an older
one.

## What the image sets

[MarahHQ/marah-os](https://github.com/MarahHQ/marah-os) lists the image's files. These are the ones Marah Launcher depends on:

| What | Where |
| --- | --- |
| Packages Marah Launcher runs or loads: gamescope, cage, `xdotool`, `xprop`, `libarchive`, `mpv-libs`, `yt-dlp`, `rpm-ostree`, `gamemode`, `pciutils` | `Containerfile` |
| Services: `greetd`, `NetworkManager`, `bluetooth`, `chronyd`, `marah-os-update.timer` | `systemctl enable` in `Containerfile` |
| os-release: `NAME="Marah OS"`, `PRETTY_NAME="Marah OS <version> (<build date>)"` | `sed` in `Containerfile` |
| The user `player` and its home | `files/usr/lib/sysusers.d/`, `files/usr/lib/tmpfiles.d/` |

The image installs packages with weak dependencies off. The Marah Launcher rpm only recommends `unar`
(`packaging/linux/nfpm.yaml`), so the image has no `unar`. See
[reference/archives-and-pc-games.md](reference/archives-and-pc-games.md) for what that means
for archives.

`os.yml` checks each image before it pushes it (the "Check the image" step in
MarahHQ/marah-os's `.github/workflows/os.yml`):

- the programs `gamescope`, `cage`, `nmcli`, `bluetoothctl`, `pactl`, `bootc`, `rpm-ostree`,
  `mokutil`, `xprop`, `xdotool`, `marah-launcher` and `marah-session`;
- the libraries `libmpv.so.2`, `libarchive.so.13`, `libxkbcommon-x11.so.0`, `libvulkan.so.1`,
  `libpulse.so.0`, `libXss.so.1`, `libasound.so.2` and `libEGL.so.1`;
- the services, the masked timer, the greetd config, `50-marah-os.toml`, the release and image
  files, the polkit rule and `os-release`;
- `marah-launcher --version` and `marah-launcher --supports gamescope`.

It doesn't check `yt-dlp`, `gamemode` or `pciutils`.

## Limitations

- **x86-64 only.** `fetch-marah-launcher.sh` and `build-image.sh` take `marah-launcher-*.x86_64.rpm`.
- **Updates are staged, never applied.** The PC changes version only when the player restarts
  it.
- **The stick runs with `enforcing=0`.** Every entry in
  `live/files/usr/lib/image-builder/bootc/iso.yaml` passes it.
- **Not tested for this page.** It was written from the code and the build files. No image,
  ISO or VM was built or started for it.
