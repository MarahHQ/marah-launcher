# Troubleshooting

This page is for players. It says what to do when something goes wrong, where the launcher
keeps its data, and which options it takes on the command line.

## When something goes wrong

**A theme doesn't load.** At the start, the launcher shows the Default theme in its place. When
you switch themes, the current theme stays. To start with the Default theme and the built-in
plugins only, run `marah-launcher --safe-mode`.

**An emulator says "Didn't load" on Home.** Its plugin failed to start. Run the launcher from a
terminal to see why: the log goes to the terminal. Set `RUST_LOG` to change how much it says
(`info` by default).

**A game doesn't start, or crashes.** Open the game's menu and choose **View emulator log**.

**A Windows game doesn't start.** Install umu-launcher from the PC page (Emulators → PC), or
install Wine. If the message says to install bubblewrap, install the `bubblewrap` package. If
the game starts but fails in the sandbox, turn off **Sandbox** in its Game settings. See
[Emulators](emulators.md#pc).

**An old PC game shows in a corner of the screen** (Linux). Set its **Resolution** in its Game
settings. It then runs inside gamescope, which must be installed.

**A RAR or another archive doesn't unpack.** The error names what is missing: libarchive, or
`unar` for some RAR files. The release packages ship libarchive.

**A trailer opens in the browser.** The launcher plays trailers in its window with libmpv.
Without libmpv, it opens them in the browser.

**A backup or a restore doesn't start.** The message says why. See
[Restore](backup.md#restore).

**A backup in the folder browser can't be restored.** A newer version of the launcher made it.
Update the launcher first. See [The folder browser](backup.md#the-folder-browser).

**The Drives panel says "Mounting needs UDisks", or a drive doesn't mount.** Install the
`udisks2` package; the launcher's packages recommend it. Without it, the launcher lists only the
drives the system mounted already. A drive inside the PC needs the administrator to mount: the
launcher's package lets a member of the `wheel` or `sudo` group mount it at the PC's own
screen. For other players, the message says "Not authorized"; mount the drive another way, or
add the player to that group.

**On Windows, the Wi-Fi list is empty.** Windows 11 shows the Wi-Fi networks to a program only
when you allow it your location. Choose Scan to see the message. Then allow location in
Settings → Privacy & security → Location, for desktop apps too.

**A drive doesn't eject.** The message says why: close the game that runs from it, or wait for
the download or the install that writes to it.

**Everything is too small or too big on the TV.** Change Settings → Launcher → **Size of
everything**. `MARAH_SCALE=<0.8 to 1.4>` sets it for one run.

**The Marah Launcher session restarts or shows a black screen.** The session writes its log to
`~/.cache/marah-session.log`. To change the session's settings, copy a line from
`/usr/share/marah-launcher/session.conf` to `/etc/marah-launcher/session.conf`.

## Where the launcher keeps its data

The launcher keeps its settings and the plugins' data in one folder:

| System | Folder |
| --- | --- |
| Linux | `~/.local/share/marah-launcher` (or `$XDG_DATA_HOME/marah-launcher`) |
| macOS | `~/Library/Application Support/marah-launcher` |
| Windows | `%APPDATA%\marah-launcher` |

`marah-launcher --data-dir <path>` uses another folder for one run. Inside the folder:

| Path | What it holds |
| --- | --- |
| `config.json` | Your settings |
| `plugins/<plugin>/` | Each plugin's data, such as the emulator builds |
| `logs/<plugin>/` | Each game's log, which **View emulator log** opens |
| `torrents/` | The state of the Catalog's downloads |
| `cache/` | Thumbnails, and images downloaded from the network |
| `updates/` | A launcher update that waits for the next start |
| `extensions/` | Themes you add |
| `restored-settings.json` | The settings of a restore, until the next start puts them back |

Your games, packages and backups are in the folders you chose in Settings, not in this folder.
The emulators keep some data in their own folders, such as "KytyPS5 saves" and "shadPS4 data".
Each one is on its emulator's page.

## Command-line options

`marah-launcher --help` lists them:

| Option | What it does |
| --- | --- |
| `--theme <id>` | Show this theme now. It isn't saved. |
| `--safe-mode` | Load the built-in plugins and the default theme only. |
| `--dev` | Reload theme files when they change. |
| `--windowed` | Open in a normal window for this run. Full screen is the default. |
| `--monitor <name>` | Open on this monitor for this run, such as `DP-2`. It isn't saved, and a screen picked in Settings wins. |
| `--data-dir <path>` | Where settings and plugin data live. |
| `--extensions <path>` | The folder of third-party plugins and themes. The default is `<data-dir>/extensions`. |
| `--folder <name>=<path>` | Map a named folder, such as `games=/srv/games`. It is saved. Repeat it for more folders under one name. |
| `--supports <feature>` | Exit 0 when this launcher has the feature (`gamescope`, `system-settings`, `os-install`), else 1. |
| `--version` | Show the version, on one line. |
| `--help` | Show this help. |

One environment variable counts too: `MARAH_SCALE=<n>` draws everything at this size for this
run, from 0.8 to 1.4. It overrides **Size of everything** in Settings.
