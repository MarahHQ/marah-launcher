# The launcher and Marah OS

This page is the contract between the launcher and Marah OS: what each one expects from the
other. It is for the people who maintain either side. Change both sides together.

Marah OS is a PC image that starts straight into the launcher, with no desktop behind it. So the
screens the OS needs (Wi-Fi, Bluetooth, sound, system updates, the install from the USB stick)
are part of the launcher, and they show only where the OS needs them. To try those screens on a
desktop, use the [session switches](development.md#session-switches).

The OS's files named here (`Containerfile`, `files/…`, `live/…`, `build-iso.sh`, `os.yml`) are
in [MarahHQ/marah-os](https://github.com/MarahHQ/marah-os). That repository also says how the
images and the USB stick are built, tested and released. Its `check-brand.sh` checks the paths
in the `[os]` section of `brand.toml`.

## The parts

| Part | Where | What it does |
| --- | --- | --- |
| The image | `Containerfile` (OS) | Fedora bootc, minimal set, with gamescope, cage, greetd, NetworkManager, BlueZ, PipeWire and the launcher's rpm. No desktop. |
| The NVIDIA edition | `nvidia/` (OS) | The image, with NVIDIA's driver. |
| The USB stick | `live/`, `build-iso.sh` (OS) | The image started live from a stick, with an installer. |
| The session script | `packaging/linux/marah-session` | Runs the launcher as the whole screen. In the launcher's rpm. |
| The system plugin | `plugins/system/host/` | Runs the system's tools for Settings → System and the Drives panel. The image needs `udisks2` for the drives. |
| The drives' polkit rule | `packaging/linux/drives.rules`, installed as `/usr/share/polkit-1/rules.d/50-marah-launcher-drives.rules` | Lets a member of `wheel` or `sudo`, in an active session at the PC's own screen, mount a drive inside the PC without a password (`org.freedesktop.udisks2.filesystem-mount-system`). In the launcher's packages. |
| `Place` | `crates/core/src/system.rs` | Tells the launcher where it runs. |

## Where the launcher runs

`Place::detect` reads three facts at the start. A variable counts only when its value is exactly
`1`. The variables are for tests and screenshots: only the session script sets
`MARAH_SESSION=1` for real.

| `Place` field | True when | Pretend with |
| --- | --- | --- |
| `own_os` | `/usr/lib/marah-os/release` exists, or `stick` is true | `MARAH_OS=1` |
| `stick` | `own_os` is true, `/proc/cmdline` has `rd.live.image`, and `/usr/libexec/marah-os/install` exists | `MARAH_USB=1` |
| `whole_screen` | `MARAH_SESSION=1`, or `own_os`, or `stick` | `MARAH_SESSION=1` |

The view carries them as `Vm.system.whole-screen`, `Vm.system.own-os` and `Vm.system.stick`.
They are facts of the session only. They don't decide which of the machine's controls show: the
network, sound, Bluetooth and drives icons, Settings → System, and Restart and Shut down show
wherever the launcher runs, on a desktop too. Whether each works comes from its system area's
state ([ADR-0004](../adr/0004-every-system-area-has-one-interface.md)). The kit reads the facts:

| Field | What changes |
| --- | --- |
| `whole-screen` | "Quit" becomes "Restart" for the launcher. The power menu and Settings have Log out (not on Marah OS). Restart and Shut down ask by a second press, not in a dialog. The game menu opens nothing outside the launcher. A drive that comes is mounted by the launcher. |
| `own-os` | Log out goes away (`kit/power-menu.slint`, `kit/settings-layout.slint`): greetd would log the player in again at once. |
| `stick` | "Welcome to Marah OS" and the install screens show (`kit/install-os.slint`). Settings has the install row. |

### The files the image writes

| File | Holds | Who reads it |
| --- | --- | --- |
| `/usr/lib/marah-os/release` | One line: `marah-launcher --version` of the launcher in the image. | The launcher tests that it exists (`OS_RELEASE_MARKER` in `brand.toml`). It doesn't read the text. |
| `/usr/lib/marah-os/image` | Where the PC updates from, without a tag: `ghcr.io/<owner>/marah-os`. | The OS: its update script, the stick's installer and the stick's build. Not the launcher. |

## What the OS asks of the launcher

`marah-launcher --supports <feature>` exits 0 when this launcher has the feature, else 1
(`supports` in `crates/app/src/args.rs`). An older launcher without the option exits with
"unknown option" and code 2, which a caller counts as no.

| Feature | Means | Checked by |
| --- | --- | --- |
| `gamescope` | The launcher runs in gamescope and tags the windows gamescope shows. | The session script before each start; `os.yml` when it checks an image. |
| `system-settings` | Settings → System for the OS. | Nothing yet. |
| `os-install` | The stick's welcome and install screens. | `build-iso.sh`: it stops when the launcher in the image answers 1. |

`marah-launcher --version` prints one line, `marah-launcher <version>`. The image keeps it in
the release file.

A new feature gets a new `--supports` name, and the OS checks for it before it relies on it.
The launcher must keep working on an older OS: a missing tool, file or rule shows as unavailable
in Settings, with the reason.

## The session

greetd logs in the user `player` at boot and runs `/usr/bin/marah-session` on terminal 1, as the
initial and the default session (`files/etc/greetd/config.toml` in the OS). The user comes from
`files/usr/lib/sysusers.d/marah-os.conf`: uid 1000, no password, in the `gamemode` group.

The launcher's rpm installs the session script (`packaging/linux/nfpm.yaml`).
`packaging/linux/marah-session.desktop` lists it for display managers on any Linux, with
`DesktopNames=Marah`. So the session works without the OS too.

`packaging/linux/marah-session` does this:

1. It reads `/usr/share/marah-launcher/session.conf`, then `/etc/marah-launcher/session.conf`.
2. It moves a log over `MARAH_LOG_MAX_MB` to `<name>.previous.log`.
3. It exports `MARAH_SESSION=1`, and `SDL_VIDEO_DRIVER=x11`, `SDL_VIDEODRIVER=x11`,
   `QT_QPA_PLATFORM=xcb` and `GDK_BACKEND=x11`. Games inherit them: the launcher and the games
   run on gamescope's X server, which the focus rules in [desktop.md](desktop.md) need.
4. It waits for the network with `nm-online`, up to `MARAH_WAIT_NETWORK` seconds, then starts
   anyway.
5. It runs `gamescope <MARAH_GAMESCOPE_ARGS> -- <MARAH_BIN>`, with all output added to the log.
6. It starts the launcher again when it ends, unless the launcher wrote `log out`.

It runs the launcher in `cage -s` instead of gamescope when gamescope is not installed,
`marah-launcher --supports gamescope` fails, or the last two gamescope runs failed (the watchdog
ended them, or they ended in under 15 seconds). With neither gamescope nor cage installed, it
runs the launcher in the current desktop when there is one, else it exits with an error.

The defaults are in `packaging/linux/session.conf`. A player copies a line to
`/etc/marah-launcher/session.conf`, which package upgrades leave alone.

| Name | Default | Meaning |
| --- | --- | --- |
| `MARAH_BIN` | `/usr/bin/marah-launcher` | The launcher to run. |
| `MARAH_GAMESCOPE_ARGS` | `-e --xwayland-count 1` | gamescope's options. `-e` is its Steam mode: the launcher and the games switch with gamescope's focus atoms. |
| `MARAH_WATCHDOG` | `90` | Seconds to wait for `window shown` before the script starts the launcher again. |
| `MARAH_LOGOUT_GRACE` | `3` | Seconds after the launcher ended (after `log out`) before the script ends the session itself. |
| `MARAH_WAIT_NETWORK` | `5` | Seconds to wait for the network at the start. |
| `MARAH_LOG` | `${XDG_CACHE_HOME:-$HOME/.cache}/marah-session.log` | Where the session and the launcher write. |
| `MARAH_LOG_MAX_MB` | `10` | The size at which the log moves to `marah-session.previous.log`. |

### Lines from the launcher to the session

The launcher writes two lines to standard output (`Platform::announce`, called from
`crates/ui-host/src/shell.rs`). The script reads them in the log.

| Line | The launcher writes it | The script |
| --- | --- | --- |
| `window shown` | Once, on the first tick where the window has a size. | Marks the start as good. Without it after `MARAH_WATCHDOG` seconds, it ends the display server and starts again. |
| `log out` | When the player chose Log out, once the running games and what their launches left running are closed, just before the launcher quits. | Ends the session, back to the login screen. |

After `log out`, the display server should end with the launcher, but gamescope can hang on its
way out. So `MARAH_LOGOUT_GRACE` seconds after the launcher ended (or 20 seconds after
`log out`, should the launcher not end), the script ends the session. In a logind session whose
desktop is `Marah`, it asks logind (`loginctl terminate-session`). Elsewhere (no logind, or
the script started from a desktop's terminal) it ends the display server itself.

After `window shown`, the app shell asks the platform to claim the window on each tick until that
works. Under gamescope, the launcher tags its windows with `xprop`, as
[desktop.md](desktop.md) says. The image installs `xprop` and `xdotool` for this.

## Settings → System

The built-in system plugin (`plugins/system/host/`) does the work. It declares `network`,
`vpn`, `bluetooth`, `audio`, `power`, `os_updates`, `os_install` and `drives`, each a system
area behind the same interface (`AreaSettings`, see
[ADR-0004](../adr/0004-every-system-area-has-one-interface.md)). On macOS it provides
`network`, `vpn`, `bluetooth`, `audio`, `power` and `drives` through the tools macOS ships. On
Windows it provides `network`, `vpn`, `bluetooth`, `audio`, `power` and `drives` through
Windows' own calls. The [plugin reference](../plugins/reference.md#the-built-in-plugins) says
what works on each. The programs it may run are `permissions.process` in
`plugins/system/host/plugin.toml`.

| Feature | The launcher runs | The image gives |
| --- | --- | --- |
| Internet, Wi-Fi | `nmcli` | NetworkManager, `wpa_supplicant`, and a polkit rule |
| Bluetooth | `bluetoothctl` | BlueZ |
| Sound output | `pactl` | PipeWire with its PulseAudio server, and `pulseaudio-utils` |
| Restart, Shut down | `systemctl reboot`, `systemctl poweroff` | systemd |
| System update state | `rpm-ostree status --json` | `rpm-ostree` |
| System update, "check now" | `systemctl start --no-block marah-os-update.service` | The update service, and a polkit rule |
| The install from the stick | `marah-os-install` | The stick's installer |

A missing tool makes its area unsupported, with the tool's name, such as "NetworkManager (nmcli)
isn't installed". The area's status icon then shows disabled, with that reason on hover and
focus. The table `AREAS` in `plugins/system/host/src/linux.rs` lists each area's tools.

Every tool runs through the plugin's tool runner (`plugins/system/host/src/tools.rs`), with a
time limit on each call. At the limit the launcher kills the tool, and the call fails with "lsblk
took too long", so a tool that hangs, such as `lsblk` on a dead drive, doesn't keep running. A
read and a quick change have 10 seconds. A call that takes longer has its own limit: joining a
Wi-Fi network, pairing, and UDisks writing what is left to a slow drive before it unmounts it.
The install from the stick and "Choose partitions instead" have none: they wait on the player,
who can cancel the install.

### polkit rules

Each rule applies only to `player`, in a local, active session.

| Action | Allowed | File (OS) |
| --- | --- | --- |
| `org.freedesktop.NetworkManager.*` | Everything | `files/usr/share/polkit-1/rules.d/50-marah-os.rules` |
| `org.freedesktop.systemd1.manage-units` | Unit `marah-os-update.service`, verb `start` | The same file |
| `org.freedesktop.policykit.exec` | Program `/usr/bin/marah-os-install` or `/usr/libexec/marah-os/install` | `live/files/usr/share/polkit-1/rules.d/50-marah-os-live.rules` (stick only) |

Joining a new Wi-Fi network saves a connection for the whole system, which polkit keeps for an
administrator. The first rule lifts that for `player`. `systemctl start` asks polkit itself, so
it needs no `pkexec`. For `pkexec`, current polkit reports the resolved path as `program` and
older polkit the given path, so the rule allows both.

The plugin lists `system:marah-os-install` under `permissions.polkit`, so the launcher runs the
installer through `pkexec`.

## System updates

The OS updates as a whole image, with bootc. The version before stays in the boot menu.

`marah-os-update.timer` starts `marah-os-update.service` 3 minutes after boot and then every
day, with up to 10 minutes of random delay. A failed run (no internet, say) runs again after 15
minutes. The service doesn't run on the stick (`ConditionKernelCommandLine=!rd.live.image`).

The service runs `/usr/libexec/marah-os/update`. It only stages the next version and never
restarts the PC, because a game may be running. For the same reason the image masks Fedora's
`bootc-fetch-apply-updates.timer`, which restarts the PC by itself. The script also picks the
edition by the graphics card.

The launcher reads the state with `rpm-ostree status --json`, as the OS updates area's state
(`SystemArea::OsUpdates`). Settings shows:

| State | Settings shows |
| --- | --- |
| A deployment is staged | "Restart to update to `<version>`" |
| None is staged | "Marah OS is up to date" |
| The output can't be read | "Couldn't look for an update: rpm-ostree's status couldn't be read" |

Without the release file, the OS updates area is absent (`AreaState::Absent`), and the update
row doesn't show.

`crates/core/src/view.rs` has the text "Looking for an update…" for `OsUpdate::Checking`, but
the Linux plugin never returns that state. A running check shows no state of its own.

### The launcher's own update on the OS

On Marah OS, the launcher comes with the image and never downloads its own update.
`Install::detect` (`crates/core/src/launcher_update.rs`) returns `MarahOs` when the file
`packaged` sits beside the binary and the release file exists. Then `package_key` returns
nothing, and nothing downloads. The "updates" row in Settings still reads `latest.json`. When a
newer launcher is out, it says so and that it comes with the next system update, and the core
starts the same check as "check now" (`check_launcher_update` in `crates/core/src/system.rs`).
The rest of the update protocol is in [releasing.md](releasing.md).

The image installs the launcher's rpm of a release (`fetch-marah-launcher.sh` and
`marah-launcher.txt` in the OS). `package.yml` names each new launcher release in that
`marah-launcher.txt` and starts the OS's `os.yml`, so the images are built again with it. The
release steps are in [releasing.md](releasing.md).

## The install from the USB stick

The stick holds a copy of the minimal image, so an install needs no internet. It installs in two
ways: erase a disk, from the launcher, or choose partitions, in Fedora's installer (Anaconda).
The OS repository covers the stick's build, its boot menu and the Anaconda way. This section is
the protocol between the launcher and the installer.

The installer is `live/files/usr/libexec/marah-os/install` in the OS. Plugins run only
`system:<name>` programs, so the launcher calls it as `marah-os-install`, a link in `/usr/bin`
to `../libexec/marah-os/install`.

### `marah-os-install --disks`

It runs as `player`, and prints one line per disk it can install on:

```text
disk PATH SIZE_BYTES WINDOWS(0/1) USB(0/1) MODEL…
```

- It leaves out the stick, read-only disks, and names that start with `zram`, `loop` or `sr`.
- It leaves out disks smaller than `MARAH_OS_MIN_DISK` bytes (default 40 × 10^9).
- `WINDOWS` is 1 when the disk has an NTFS or BitLocker partition.
- `USB` is 1 when the disk's transport is USB.
- `MODEL` is the rest of the line and may hold spaces. It is `Disk` when the disk names none.

The plugin reads the lines with `parse::disks` (`plugins/system/host/src/parse.rs`). Without
the installer, it lists no disks.

### `marah-os-install DISK`

The launcher runs it through `pkexec`. It must run as root, and it refuses a disk that
`--disks` doesn't list. It erases the disk and installs with `bootc install to-disk --wipe`. It
prints one line at a time:

| Line | Meaning |
| --- | --- |
| `progress PERCENT TEXT` | Progress. |
| `done TAG [mok] [nvidia-later]` | The install ended. `TAG` is the edition, `minimal` or `minimal-nvidia`. `mok`: Secure Boot's key screen comes at the restart. `nvidia-later`: the PC has an NVIDIA card and gets the driver on the installed system. |
| `error TEXT` | The install failed. The exit code is not 0. |

The launcher ignores every other line (`parse::install_line`). bootc's own output goes to
standard error. When the installer ends without `done` or `error`, the launcher shows "The
installer stopped without finishing". Exit codes 126 and 127 show "The installer wasn't allowed
to run".

The installer also copies the Wi-Fi connections saved on the stick into the new system.

### `marah-os-install --choose-partitions`

The launcher's "Choose partitions instead" runs it through `pkexec`. It switches the running
stick to `marah-os-anaconda.target` with `systemctl --no-block isolate`, and exits 0 once the
switch has started. greetd and the launcher stop right after. When it can't, it prints one
`error TEXT` line and exits with another code. The launcher shows that text.

### Changing the protocol

The installer may add words to a line, and the launcher ignores words it doesn't know. Don't
change or remove the words above: an installed launcher must read a newer stick, and a newer
launcher an older one.

## What the image sets

These are the image's settings the launcher depends on:

| What | Where (OS) |
| --- | --- |
| Packages the launcher runs or loads: gamescope, cage, `xdotool`, `xprop`, `libarchive`, `mpv-libs`, `yt-dlp`, `rpm-ostree`, `gamemode`, `bubblewrap`, `pciutils` | `Containerfile` |
| Services: `greetd`, `NetworkManager`, `bluetooth`, `chronyd`, `marah-os-update.timer` | `systemctl enable` in `Containerfile` |
| os-release: `NAME="Marah OS"`, `PRETTY_NAME="Marah OS <version> (<build date>)"` | `sed` in `Containerfile` |
| The user `player` and its home | `files/usr/lib/sysusers.d/`, `files/usr/lib/tmpfiles.d/` |

The image installs packages with weak dependencies off. The launcher's rpm only recommends
`unar` (`packaging/linux/nfpm.yaml`), so the image has no `unar`. What that means for archives
is in [archives-and-pc-games.md](archives-and-pc-games.md).

`os.yml` checks each image before it pushes it (the step "Check the image"):

- the programs `gamescope`, `cage`, `nmcli`, `bluetoothctl`, `pactl`, `bootc`, `rpm-ostree`,
  `mokutil`, `xprop`, `xdotool`, `marah-launcher` and `marah-session`;
- the libraries `libmpv.so.2`, `libarchive.so.13`, `libxkbcommon-x11.so.0`, `libvulkan.so.1`,
  `libpulse.so.0`, `libXss.so.1`, `libasound.so.2` and `libEGL.so.1`;
- the services, the masked timer, the greetd config, `50-marah-os.toml`, the release and image
  files, the polkit rule and `os-release`;
- `marah-launcher --version` and `marah-launcher --supports gamescope`.

It doesn't check `yt-dlp`, `gamemode`, `bubblewrap` or `pciutils`.

## Limitations

- **x86-64 only.** `fetch-marah-launcher.sh` and `build-image.sh` take
  `marah-launcher-*.x86_64.rpm`.
- **Updates are staged, never applied.** The PC changes version only when the player restarts
  it.
- **The stick runs with `enforcing=0`.** Every entry of its boot menu
  (`live/files/usr/lib/image-builder/bootc/iso.yaml`) passes it.
- **Not run for this page.** It was written from the code of both repositories. No image, ISO
  or VM was built or started for it.
