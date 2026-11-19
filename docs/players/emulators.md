# Emulators

This page is for players. It covers the Emulators screen, an emulator's page, and what to know
about each emulator the launcher ships.

## The Emulators screen

Emulators lists the emulator plugins, then the service plugins (Game Catalog and PlayStation
Store). Each row says whether the plugin is installed, and whether it is turned on. Press ✕ to
open a plugin's page beside the list.

A plugin's page has:

- **Turned on**: turns the plugin on or off.
- The build, for an emulator the launcher installs: install it, update it, or **Uninstall** it.
- The plugin's actions, such as installing a package from your packages folders.
- Its settings, and its folders.

The page is one page, as Settings is. Each section is under its own header: **General**, then
one section for each group of settings (one named **Settings** when there is one group), then
**Folders** when no group of settings holds them. With two sections or more, a rail of their
icons is beside the page. L1, R1 and the rail jump to a section. Up and Down go on across the
sections.

## Builds

For KytyPS5, shadPS4 and Lichii, the launcher downloads the official builds and keeps the build
before the current one. To run a build of your own, map its folder on the emulator's page:
"Your own KytyPS5 build", "Your own shadPS4 build" or "Your own Lichii build". The launcher never
updates your own build.

Settings → Updates says when the launcher installs and updates builds. See
[Settings](settings.md#updates).

## PS5: KytyPS5

A KytyPS5 build that knows the `--data-folder` option keeps its saves in one folder that every
such build shares: "KytyPS5 saves" on its page, `~/.local/share/KytyPS5` until you change it.

## PS4: shadPS4

### PS4 system files

Some PS4 games need the PS4's system modules and fonts: 14 `.sprx` files in `sys_modules`, and
45 fonts in `fonts/font` and `fonts/font2`, in **shadPS4 data** (`~/.local/share/shadPS4` until
you change it).

- If any of them are missing, a question comes up over the boot screen: **Install PS4 system
  files?** **Install** downloads the missing files, about 38 MB for all of them, from the
  [retrobios collection](https://github.com/Abdess/retrobios) on GitHub, at a fixed commit. The
  boot screen shows the download and waits for it. **Not now** downloads nothing, and the next
  start asks again.
- The launcher checks the size and SHA-256 of each file before it puts the file in place. If a
  download fails, or a file doesn't match, the block says why, the boot goes on, and no
  half-written file is left in shadPS4's folders.
- The **System files** block on the PS4 page shows the state: **Installed**, **Missing** and how
  many, or **Your own files**. **Install now** installs the missing files at once. Turn off
  **Ask at start** in the same block, and the start no longer asks.
- Files you put there yourself are never replaced, and files that aren't on the list are never
  touched.

These are Sony's files: check that you may use them where you live.

### The touchpad button

A PS1 game on PS4, such as Tekken 3, reads a click on the right half of the touchpad as START,
and on the left half as SELECT. A controller with no touchpad clicks with its **Back** (or
**View**) button, and shadPS4's own key map clicks the centre, which is neither. The game then
waits at "Press START".

So before each PS4 game, the launcher sets shadPS4's key map (`input_config` in **shadPS4
data**): **Back** and **Space** click the right half (START), and **Backspace** the left half
(SELECT). It changes only shadPS4's own touchpad lines. A touchpad you mapped to other keys
stays as it is.

In a PS1 game on PS4 the Options button does nothing. So before a PS1 game the launcher also
makes the controller's **Start** START and **Back** SELECT. Before a PS4 game, Start is Options
again. In a PS1 game, Options is on the keyboard only (**Enter**). A PS1 game is a PS4 game
folder with `config-title.txt` and a `.cue` disc in `data/`.

### The Save Migration box

At its first start, shadPS4 asks what to do with saves in its old place (`savedata/1`), in a box
a controller can't answer. When there are no such saves, the launcher makes shadPS4's first
user (`home/1000`) before the first PS4 game, so the box doesn't come up. When there are,
shadPS4 asks: **Copy** keeps the old saves as well, **Do nothing** leaves them where they are.

## Switch: Lichii

Lichii plays Switch and Switch 2 games. It runs on Linux only.

### Lichii's data

Lichii keeps its data in `~/.local/share/lichii` from build-150 on. If you played with an older
build, open Lichii from its page once and import your Eden data (`~/.local/share/eden`). Until
then, Lichii starts no game. An older build, or your own build from before build-150, still
reads `~/.local/share/eden`: set **Lichii data** on Lichii's page to that folder.

### Firmware and keys

The **Firmware** block on Lichii's page shows the firmware installed and the update that runs,
with a bar while it downloads.

- With **Ask at start** on, as it is by default, each start checks for the set release. If no
  firmware is there, or the launcher installed an older one, a question comes up over the boot
  screen. **Install** downloads it, and the boot screen shows the download and waits for it.
  **Not now** downloads nothing, and the next start asks again.
- A visit to Lichii's page never downloads. The block shows **Available** and the firmware it
  would install. **Update now** installs it at once.
- Firmware you put there yourself is left alone.

The launcher downloads the **Firmware release** set in that block from
[THZoria/NX_Firmware](https://github.com/THZoria/NX_Firmware), checks its SHA-256 against the one
GitHub gives, and puts it in place of the firmware in that folder. Lichii uses it from its next
start. `prod.keys` and any `title.keys` from the archive of the same version, at
`https://files.prodkeys.net/ProdKeys.NET-v<firmware_release>.zip`, go into the `keys` folder.
The keys always match the firmware: firmware newer than its keys can't be read, and Lichii hangs
on its start.

**Firmware release** is `22.5.0` by default. To use another one, select the row, press Enter,
type a version that both sites have, such as `22.1.0`, then press **Update now**. The badge
beside the button names the firmware version the launcher installed. If an update fails, the old
firmware stays.

This is Nintendo's firmware: check that you may use it where you live.

### Airplane mode

Games have internet access: **Airplane mode** in the Switch page's **Settings** section is off.
Turn it on to keep every Switch game offline. This applies to the games the launcher starts with
an official build. Your own build and Lichii's window use the airplane mode in Lichii's own
settings file (`qt-config.ini`), which is on until you turn it off in Lichii's window. If that
file can't be read, the game doesn't start and the launcher says why.

## PC

A Windows game runs through umu-launcher (Proton) or Wine. On Linux, the PC page has **Install
umu-launcher**, and then **Update umu-launcher**. A game's own settings (Game settings in its
menu) have:

- **Starts**: the file Play starts.
- **Resolution** (Linux only): anything but your screen runs the game inside gamescope, which
  must be installed. Use it for an old game that shows in a corner of the screen.
- **Sandbox** (Linux, Windows games only): see below.

### PC games and your files

A game from a download can hold malware. On Linux, the launcher runs each Windows game in a
sandbox made with bubblewrap. The game sees its own folder and its Wine prefix. It doesn't see
your home folder, your other games or your other files. Sound, the display, controllers and the
network work as usual.

- The **Sandbox** setting is on by default. Turn it off only for a game that doesn't run in the
  sandbox. The game then runs with your access to every file.
- The first Windows game you play in the sandbox downloads Proton into the launcher's own
  folder. That takes a few minutes, once.
- A game in the sandbox runs without GameMode.
- Linux games, scripts and AppImages don't run in the sandbox: they keep their saves in your
  home folder. Install only releases you trust.

## Steam

Steam runs on Linux only. The launcher reads Steam's folder and every library Steam lists. Set
**Steam libraries** only to read other folders. In the Catalog, the Steam system lists the games
in your Steam collections. Install and Uninstall ask Steam, in Steam's own dialogs. Play hands
the game to Steam.
