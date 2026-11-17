# Changelog

## [0.4.0](https://github.com/MarahHQ/marah-launcher/compare/v0.3.0...v0.4.0) (2026-11-17)


### ⚠ BREAKING CHANGES

* game folders hold launcher_game.json and launcher_art/. The launcher renames the marah_game.json and marah_art/ that 0.2.0 and 0.3.0 wrote the first time it reads a folder; an older build no longer reads a renamed folder.

### Features

* a game folder's file and art folder are launcher_game.json and launcher_art, names without the product's ([#19](https://github.com/MarahHQ/marah-launcher/issues/19)) ([55a89d2](https://github.com/MarahHQ/marah-launcher/commit/55a89d21a14f7788c02044816591e66d41f9b381))
* **kit:** Settings' sections go in their order of use: System, Games, Downloads, Launcher, Updates, Backup, This PC, Power ([#21](https://github.com/MarahHQ/marah-launcher/issues/21)) ([4a8399d](https://github.com/MarahHQ/marah-launcher/commit/4a8399da397b00e6c6f2f18c16da8ff070cc08b2))
* **release-catalog:** hide games without images, on by default; pictures on ibb.co, radikal.ru and postimg.cc load ([#23](https://github.com/MarahHQ/marah-launcher/issues/23)) ([9f08d3d](https://github.com/MarahHQ/marah-launcher/commit/9f08d3d42971db571aaec206a440033a4c2f82b6))


### Bug Fixes

* **core:** an installed game links to its offer across a plural, a DLC tail, a bracketed short name, and the numbers of its dashed folder ([#24](https://github.com/MarahHQ/marah-launcher/issues/24)) ([b54a4cd](https://github.com/MarahHQ/marah-launcher/commit/b54a4cdf37601ef6c13f70ee7db4ccaa14a6e2a1))
* **kit:** Settings' rail lights the section the mouse wheel scrolls into view ([#14](https://github.com/MarahHQ/marah-launcher/issues/14)) ([63a5526](https://github.com/MarahHQ/marah-launcher/commit/63a5526b777cc6008b9dc97058fe44438c9d1dca))
* **pc:** a game copied in by hand is named by its gameinfo in data/noarch, or by its dashed folder name without the version ([#16](https://github.com/MarahHQ/marah-launcher/issues/16)) ([24a1fdc](https://github.com/MarahHQ/marah-launcher/commit/24a1fdc3604b6b200f299fa4c3091ccbb10b30bb))
* **themes:** Settings is the top bar's last item; the clock sits with the machine's icons before it ([#20](https://github.com/MarahHQ/marah-launcher/issues/20)) ([a4ac31b](https://github.com/MarahHQ/marah-launcher/commit/a4ac31b97a4f61a9994861db8d14da801103e891))
* **themes:** the clock is the top bar's last item again, after Settings ([#22](https://github.com/MarahHQ/marah-launcher/issues/22)) ([adeb053](https://github.com/MarahHQ/marah-launcher/commit/adeb05353d8e7c7c36cd6013c1c54286c5b87e27))

## [0.3.0](https://github.com/MarahHQ/marah-launcher/compare/v0.2.0...v0.3.0) (2026-11-17)


### Features

* **kit:** Settings is one page of every section; the rail, L1 and R1 jump to one ([#13](https://github.com/MarahHQ/marah-launcher/issues/13)) ([f0e612d](https://github.com/MarahHQ/marah-launcher/commit/f0e612ded8b81d7c1d2573273ed2a4a964975f62))
* the macOS icon is the Arctic Ice logo, made with png2icns away from a Mac ([#10](https://github.com/MarahHQ/marah-launcher/issues/10)) ([d074e9e](https://github.com/MarahHQ/marah-launcher/commit/d074e9ea38e8a76ac52458faac4d9bfcc954423d))

## [0.2.0](https://github.com/MarahHQ/marah-launcher/compare/v0.1.0...v0.2.0) (2026-11-16)


### Features

* the icons and theme logos are the Arctic Ice logo ([#8](https://github.com/MarahHQ/marah-launcher/issues/8)) ([f0185fd](https://github.com/MarahHQ/marah-launcher/commit/f0185fd4ce8e7b04458ceb6fb6282ad1092fe93d))


### Bug Fixes

* names with a space: the disk image's volume name, and the session's desktop name is Marah ([#7](https://github.com/MarahHQ/marah-launcher/issues/7)) ([d04ae33](https://github.com/MarahHQ/marah-launcher/commit/d04ae33b02c97693a82ff12ed73b64fdc2bdc5f3))

## 0.1.0 (2026-11-16)


### Features

* Marah Launcher 0.1.0 ([845baeb](https://github.com/MarahHQ/marah-launcher/commit/845baeb4dc6d956fe79b89a5b224ff476897c591))

## Changelog
