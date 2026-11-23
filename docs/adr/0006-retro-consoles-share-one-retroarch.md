# Retro consoles share one RetroArch

Status: accepted.

## Context

The NES, Genesis and PS1 plugins all run RetroArch, each with its own core. Each emulator plugin
had a data folder of its own (`plugins/<id>`), where its builds, `versions.json` and settings
files live. Three copies of RetroArch would be three downloads of about 450 MB, three updates,
and three places a player's RetroArch settings could be.

libretro's build server publishes no checksums, and its cores folder changes every night with
no version. The host checks every download against a SHA-256 the plugin gives.

## Decision

- A manifest can name a shared data folder (`shared_data = "retroarch"`). Every plugin that
  names it has `shared/<name>` as its data folder: one set of builds and one `versions.json`
  between them. The host runs one install among them at a time (`BuildsSubsystem::claim`), and
  after an install or uninstall it reads the builds of each.
- Only a built-in plugin may name a shared folder. A shared folder holds another plugin's
  programs, so a plugin from disk that names one is refused.
- The kit pins RetroArch's stable release (`emulator_kit::retroarch`): each file's address, size
  and SHA-256. A build is RetroArch, then the cores of every retro plugin out of the release's
  pack of cores, which is versioned with it. A 7z download can unpack only some of its paths
  (`Unpack::only`), so the pack's 1.6 GB gives about 20 MB.
- RetroArch runs with configs of the launcher's in the shared folder, and on Linux with
  `XDG_CONFIG_HOME` there too, so it never reads or writes the player's own
  `~/.config/retroarch`.

## Consequences

- One install serves every retro plugin, and an update reaches them all.
- An uninstall from one retro plugin removes RetroArch for all of them.
- A new RetroArch comes with a new release of the launcher, with its new checksums.
- macOS gets no retro build: libretro publishes no pack of cores for it.
- A new retro console is a small plugin: its `RetroConsole`, its manifest, and its core in the
  pinned pack.
