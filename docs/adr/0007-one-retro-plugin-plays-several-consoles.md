# One Retro plugin plays several consoles

Status: accepted. Changes ADR 0006's "a new retro console is a small plugin".

## Context

The NES and Genesis plugins ran the same shared RetroArch, each with its own core. Every new
retro console meant one more plugin row, one more page, and one more on/off switch, all for one
program. The player asked for one Retro plugin, with a list of the consoles it plays that they
turn on or off, starting with the NES, the Genesis and the Atari 2600. The PS1 stays a plugin of
its own.

## Decision

- A manifest can list several consoles in `[[consoles]]` (API 0.7.0), each with its console, its
  system look, keymap, install note and system files. The host keeps which ones the player turned
  on in `app.consoles.<plugin>`, and gives each console that is on its Home entry, its catalog,
  its games and its system-files block.
- `emu-retro` plays the NES (FCEUmm), the Genesis (Genesis Plus GX) and the Atari 2600 (Stella).
  In the kit, each console is still a `RetroConsole`. `RetroConsoles` routes the host's calls to
  each console by the game id, which starts with the console's code: `NES/Contra`.
- Each console's games stay in its own folder of the game folders: `NES/`, `GENESIS/` and
  `ATARI2600/`.
- `[[replaces]]` names the plugins `emu-retro` took the place of (`emu-nes`, `emu-genesis`). At
  each start, the core moves what is still kept under their ids: play time, hidden games, art,
  folders, results, settings, on/off state, game installs and downloads. A restore of an older
  backup puts their items where `emu-retro` keeps the same items.

## Consequences

- A new retro console is one `RetroConsole`, one `[[consoles]]` entry, and its core in the
  pinned pack.
- The Stella core comes with the build: an install made before this change lacks it, and needs
  an uninstall and an install of RetroArch.
- A launcher older than this one doesn't know `emu-retro`: after the move, going back to it
  loses the NES and Genesis records it kept.
