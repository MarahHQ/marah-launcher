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

## System files

Some emulators need files besides their program and your games: firmware, keys, BIOS files or
system modules. The launcher doesn't download them. You put them in one archive per emulator,
and import it on the emulator's page.

| Emulator | Archive | What it holds |
| --- | --- | --- |
| PS4 (shadPS4) | `ps4-system-files.zip` | `sys_modules/` (the `.sprx` files), `fonts/` (`font/`, `font2/`) |
| Switch (Lichii) | `switch-firmware-keys.zip` | `firmware/` (the `.nca` files), `keys/` (`prod.keys`, `title.keys`). A `VERSION` file that names the version is fine: the launcher leaves it out. |
| PS1 | `ps1-bios.zip` | `system/` (`scph*.bin`, `psxonpsp660.bin`) |
| NES (Retro) | `nes-bios.zip` | `system/` (`disksys.rom` for Famicom Disk System games, `gamegenie.nes`) |
| Genesis (Retro) | `genesis-bios.zip` | `system/` (`bios_MD.bin`, `bios_*.sms`, `bios.gg`, and the lock-on carts `sk.bin`, `sk2chip.bin`, `areplay.bin`, `ggenie.bin`) |

The PS1 and Retro emulators run on RetroArch, and their BIOS files all go into
RetroArch's one `system` folder. Each takes only its own files from its archive. Most of their
games play without BIOS files, so their pages don't warn while the files are missing.

### Import an archive

1. Open the emulator's page from the Emulators screen.
2. In its system files block, select **Import…**. The folder browser opens at your Downloads
   folder, and lists folders and archives.
3. Pick the archive. The launcher unpacks it, checks it, and copies each folder where the
   emulator reads it. The block then says **Installed** and how many files it imported.

Each part replaces only the files it brings: your other files in those folders stay. The Switch
firmware and keys are the exception: each replaces its old folder whole, so two versions never
mix. Only one import runs at a time. The launcher writes
nothing until it has checked the whole archive. If any part can't be written, every part goes
back as it was, and the block says why. An archive that lacks a folder is refused with the
folders it should hold.

### Make an archive

Put the folders in the table at the top of the archive, or in one folder that holds them. A
`SHA256SUMS` file at the top is optional: when it is there, every file it lists must be in the
archive and match, or the import is refused. Other files, such as a `README`, are left out.

On Linux or macOS, for the PS4:

```sh
mkdir ps4 && cd ps4
# Copy sys_modules/ and fonts/ here, then:
find . -type f ! -name SHA256SUMS | sort | sed 's|^\./||' | xargs -d '\n' sha256sum > SHA256SUMS
zip -qr ../ps4-system-files.zip .
```

On macOS, use `shasum -a 256` in place of `sha256sum`, and leave out `-d '\n'`. On Windows, make
the zip from Explorer (**Send to → Compressed (zipped) folder**) and leave out `SHA256SUMS`.

### Where the files come from

- **PS4:** the `shadps4` folder of the retrobios collection, at
  <https://github.com/Abdess/retrobios/tree/f75b87448e52aa4f64385105583b6e937957d95d/bios/Sony/PlayStation%204/shadps4>.
  It holds `sys_modules/` and `fonts/` as the archive needs them.
- **Switch firmware:** the release of the version you want at
  <https://github.com/THZoria/NX_Firmware/releases>, for example
  <https://github.com/THZoria/NX_Firmware/releases/tag/22.5.0>. Unzip it: its `.nca` files go in
  `firmware/`.
- **Switch keys:** the keys of the same version, at
  `https://files.prodkeys.net/ProdKeys.NET-v<version>.zip`, for example
  <https://files.prodkeys.net/ProdKeys.NET-v22.5.0.zip>. Put `prod.keys` and `title.keys` in
  `keys/`.
- **BIOS files:** your own console's, or a RetroArch `system/` folder. The PS1 core reads
  `scph5500.bin`, `scph5501.bin`, `scph5502.bin`, `scph1001.bin` or `scph7001.bin`. The Genesis
  core reads `bios_MD.bin`, `bios_E.sms`, `bios_U.sms`, `bios_J.sms` and `bios.gg`. The NES core
  reads `disksys.rom` for Famicom Disk System games.

## PS1 and Retro: RetroArch

The **Retro** emulator plays the NES, the Genesis and the Atari 2600, on one RetroArch. The PS1
is an emulator of its own, on the same RetroArch.

### Consoles

The **Consoles** block on the Retro page has a switch for each console. A console that is off
has no Home entry and no Catalog, and its games are hidden. Nothing is deleted: turn it on, and
they are back. Each console's games are in its own folder of your game folders: `NES/`,
`GENESIS/` and `ATARI2600/`.

The Retro emulator took the place of the NES and Genesis emulators. At its first start, the
launcher moves their games' play time and settings to it. A NES or Genesis emulator you had
turned off is a console that is off.

The Atari 2600 plays on RetroArch's Stella core. RetroArch installed before the Retro emulator
lacks it: select **Uninstall** on the Retro page, then install RetroArch again. Your saves, save
states and RetroArch settings stay.

### Picture

The consoles draw small pictures (the NES draws 256×240 pixels), and RetroArch scales them up
to your screen. The **Picture** setting on the emulator's page sets how, and a game's own
setting wins over it:

- **Sharp pixels**: each pixel is the same size, so nothing shimmers when the game scrolls. A
  black border fills the rest of the screen.
- **Smooth edges** (the default): the hq2x shader smooths the steps on the edges of the pixel
  art, and keeps its texture. The picture fills the screen's height.
- **Old TV (CRT)**: the crt-geom shader draws the scanlines and the curved glass of an old TV.

The launcher picks the shader that RetroArch's video driver reads: GLSL for `gl`, slang for
`glcore`, `vulkan` and Direct3D 10 to 12. A driver that reads neither starts the game with no
shader. So does your own RetroArch build, and an official build installed before the launcher
kept the shaders. To get them, select **Uninstall** on the emulator's page, then install
RetroArch again. Only the build's folder goes: your saves, save states and RetroArch settings
stay.

## PS5: KytyPS5

A KytyPS5 build that knows the `--data-folder` option keeps its saves in one folder that every
such build shares: "KytyPS5 saves" on its page, `~/.local/share/KytyPS5` until you change it.

## PS4: shadPS4

### PS4 system files

Some PS4 games need the PS4's system modules and fonts: 14 `.sprx` files in `sys_modules`, and
45 fonts in `fonts/font` and `fonts/font2`, in **shadPS4 data** (`~/.local/share/shadPS4` until
you change it). Import them from `ps4-system-files.zip` on the PS4 page: see
[System files](#system-files). These are Sony's files: check that you may use them where you live.

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

Lichii plays Switch and Switch 2 games. It runs on Linux, on Windows, and on a Mac with Apple
Silicon. Lichii has no build for an Intel Mac: there, only your own build runs.

### Lichii's data

Lichii keeps its data in `~/.local/share/lichii` from build-150 on, on a Mac too. On Windows
it keeps its data in `%APPDATA%\lichii`, and its settings in `%APPDATA%\lichii\config`.

If you played with an older build, open Lichii from its page once and import your Eden data
(`~/.local/share/eden`). Until then, Lichii starts no game. An older build, or your own build
from before build-150, still reads `~/.local/share/eden`: set **Lichii data** on Lichii's page
to that folder.

### Firmware and keys

Switch games need the Switch firmware and the keys of the same version. Import both from
`switch-firmware-keys.zip` on Lichii's page: see [System files](#system-files). The firmware goes
into `nand/system/Contents/registered` in **Lichii data**, in place of the firmware there, so two
versions never mix. `prod.keys` and `title.keys` go into `keys`, in place of the keys there.
Lichii uses them from its next start. The keys must match the firmware: firmware newer than its keys can't be read, and Lichii
hangs on its start.

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

Steam runs on Linux, macOS and Windows. The launcher reads Steam's folder and every library
Steam lists. Steam's folder is `~/Library/Application Support/Steam` on a Mac, and
`C:\Program Files (x86)\Steam` on Windows. Set **Steam libraries** only to read other folders.
On Windows, one of them must be Steam's own folder, with `steam.exe`: Play starts Steam from
there. In the Catalog, the Steam system lists the games in your Steam collections. Install and
Uninstall ask Steam, in Steam's own dialogs. Play hands the game to Steam.
