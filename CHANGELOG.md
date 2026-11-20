# Changelog

## [0.17.1](https://github.com/MarahHQ/marah-launcher/compare/v0.17.0...v0.17.1) (2026-11-20)


### Bug Fixes

* each pad press counts once while Steam runs, and the top bar's focus marks are right ([5f3f911](https://github.com/MarahHQ/marah-launcher/commit/5f3f911bee652f60b16af5ace3c8fcc66943aa56))

## [0.17.0](https://github.com/MarahHQ/marah-launcher/compare/v0.16.1...v0.17.0) (2026-11-20)


### Features

* Switch and Steam on macOS and Windows, and PS5 builds from upstream on macOS ([8048218](https://github.com/MarahHQ/marah-launcher/commit/8048218594fa4e08f4cbbd4a1b6b7348d339e021))

## [0.16.1](https://github.com/MarahHQ/marah-launcher/compare/v0.16.0...v0.16.1) (2026-11-20)


### Bug Fixes

* the Game Hub shows the card's cover while its own loads, and a livelier demo ([37e96d1](https://github.com/MarahHQ/marah-launcher/commit/37e96d1805c67d3c7598667324c64687392a77fb))

## [0.16.0](https://github.com/MarahHQ/marah-launcher/compare/v0.15.0...v0.16.0) (2026-11-20)


### ⚠ BREAKING CHANGES

* the top bar shows what matters, and a tray holds the rest ([#107](https://github.com/MarahHQ/marah-launcher/issues/107))
* the player manages the machine in every mode and on every OS ([#106](https://github.com/MarahHQ/marah-launcher/issues/106))
* drives, a folder browser, and a smoother Settings ([#103](https://github.com/MarahHQ/marah-launcher/issues/103))

### Features

* drives, a folder browser, and a smoother Settings ([#103](https://github.com/MarahHQ/marah-launcher/issues/103)) ([5e4e3b2](https://github.com/MarahHQ/marah-launcher/commit/5e4e3b264a35e3baf590f3d3818320866482e40e))
* the player manages the machine in every mode and on every OS ([#106](https://github.com/MarahHQ/marah-launcher/issues/106)) ([adebb04](https://github.com/MarahHQ/marah-launcher/commit/adebb048809727a509e47b188bbb0c0cad4b3898))
* the top bar shows what matters, and a tray holds the rest ([#107](https://github.com/MarahHQ/marah-launcher/issues/107)) ([8f7ea82](https://github.com/MarahHQ/marah-launcher/commit/8f7ea82aac9da0faaa83a4d54e726502a84b5267))


### Bug Fixes

* a cancel never claims to stop a program it can't stop, and asks the installer to stop ([#105](https://github.com/MarahHQ/marah-launcher/issues/105)) ([73774cc](https://github.com/MarahHQ/marah-launcher/commit/73774cc3d01d6863a565bf36aba4799d062adc6f))

## [0.15.0](https://github.com/MarahHQ/marah-launcher/compare/v0.14.0...v0.15.0) (2026-11-19)


### Features

* the follow-ups from the reviews of [#69](https://github.com/MarahHQ/marah-launcher/issues/69) to [#97](https://github.com/MarahHQ/marah-launcher/issues/97) ([#98](https://github.com/MarahHQ/marah-launcher/issues/98)) ([03ecec7](https://github.com/MarahHQ/marah-launcher/commit/03ecec79b1e98fe1ad91fe3809846c1b9ae7e96e))


### Bug Fixes

* **assets:** the logo's gamepad has the icon's dark body, as on the website ([#100](https://github.com/MarahHQ/marah-launcher/issues/100)) ([ee45286](https://github.com/MarahHQ/marah-launcher/commit/ee452862d86a498b4c2276206da40c3807885b26))
* **installs:** the follow-ups from the review of [#98](https://github.com/MarahHQ/marah-launcher/issues/98) ([#101](https://github.com/MarahHQ/marah-launcher/issues/101)) ([d192a22](https://github.com/MarahHQ/marah-launcher/commit/d192a22a70bff2a1e3d090482b33621f09fb30e3))

## [0.14.0](https://github.com/MarahHQ/marah-launcher/compare/v0.13.0...v0.14.0) (2026-11-19)


### ⚠ BREAKING CHANGES

* **themes:** the shell and game-list behaviour live in the kit, and the theme contract is 0.5.0 ([#81](https://github.com/MarahHQ/marah-launcher/issues/81))

### Features

* **themes:** the core words a download's state and a release's Download button, and the kit shows them ([#91](https://github.com/MarahHQ/marah-launcher/issues/91)) ([0a3c399](https://github.com/MarahHQ/marah-launcher/commit/0a3c399edaeb8c32c6f28aeea3ae77c0b91120dc))
* **themes:** the core words a release's title, its peers and its download sheet line, and the kit shows them ([#93](https://github.com/MarahHQ/marah-launcher/issues/93)) ([f358e59](https://github.com/MarahHQ/marah-launcher/commit/f358e59552cb391b8b0db0f6e3df613ab7f3334b))


### Bug Fixes

* **archives:** an archive whose links chain out of the folder, or with a file under a link, is refused before anything is written ([#69](https://github.com/MarahHQ/marah-launcher/issues/69)) ([d918b81](https://github.com/MarahHQ/marah-launcher/commit/d918b814f4df72aeff70b469cddcc1496bfc61fc))
* **build:** the vendored oozextract is its own workspace, so the check runs in a worktree inside the checkout ([#82](https://github.com/MarahHQ/marah-launcher/issues/82)) ([4326b7b](https://github.com/MarahHQ/marah-launcher/commit/4326b7b344560d742f45bd049b2825627f70294a))
* **catalog:** the catalog sources are fetched in the plugin's own task, and a failed fetch waits before it tries again ([#86](https://github.com/MarahHQ/marah-launcher/issues/86)) ([e716f09](https://github.com/MarahHQ/marah-launcher/commit/e716f0927557957a90e2ea612cba3836c25f3c62))
* **core:** a plugin's file is written whole, so a reader or a crash never finds it half written ([#95](https://github.com/MarahHQ/marah-launcher/issues/95)) ([7258267](https://github.com/MarahHQ/marah-launcher/commit/725826795e8bbd11e45ab9fef40a9dfb651ebff9))
* **core:** every plugin call at close down and in the system settings goes through the guard ([#84](https://github.com/MarahHQ/marah-launcher/issues/84)) ([46d065e](https://github.com/MarahHQ/marah-launcher/commit/46d065e2884825939e44d5d5aee03c47e15669c8))
* **core:** the Catalog's off-screen grid is kept by navigation and the catalog read, not by the view ([#89](https://github.com/MarahHQ/marah-launcher/issues/89)) ([7c3fd23](https://github.com/MarahHQ/marah-launcher/commit/7c3fd230ad307acab337d212892541ce222ee499))
* **emulators:** an emulator build doesn't install twice at once ([#71](https://github.com/MarahHQ/marah-launcher/issues/71)) ([c761e80](https://github.com/MarahHQ/marah-launcher/commit/c761e802c906d6b5417fcd70d8504d214d23b3e9))
* **hub:** the Game Hub lights a button only while the keys are on the button row ([#92](https://github.com/MarahHQ/marah-launcher/issues/92)) ([f32db9a](https://github.com/MarahHQ/marah-launcher/commit/f32db9a2fc0141f197f13b7a9a4192e05552cb89))
* **themes:** the shell and game-list behaviour live in the kit, and the theme contract is 0.5.0 ([#81](https://github.com/MarahHQ/marah-launcher/issues/81)) ([4f3f63f](https://github.com/MarahHQ/marah-launcher/commit/4f3f63faa42d44c740eb304a7f27abdc108352b2))

## [0.13.0](https://github.com/MarahHQ/marah-launcher/compare/v0.12.0...v0.13.0) (2026-11-18)


### Features

* **torrents:** a download with files left out shares the files it has ([#65](https://github.com/MarahHQ/marah-launcher/issues/65)) ([0c42b83](https://github.com/MarahHQ/marah-launcher/commit/0c42b83ccacfba82080bebbff0bdab78e9b2e0c9))


### Bug Fixes

* **hub:** a version's Game Hub shows the player's result for the installed game it is ([#63](https://github.com/MarahHQ/marah-launcher/issues/63)) ([48ac94d](https://github.com/MarahHQ/marah-launcher/commit/48ac94d678500d5e7f2795dd3268646ecc5e6f43))

## [0.12.0](https://github.com/MarahHQ/marah-launcher/compare/v0.11.1...v0.12.0) (2026-11-18)


### Features

* **catalog:** offers you don't have get their emulator's community result by title id ([#61](https://github.com/MarahHQ/marah-launcher/issues/61)) ([a744e40](https://github.com/MarahHQ/marah-launcher/commit/a744e40d8674bf87a4c84e6240c3a299208f2228))

## [0.11.1](https://github.com/MarahHQ/marah-launcher/compare/v0.11.0...v0.11.1) (2026-11-18)


### Bug Fixes

* **catalog:** counts match the cards, hide one version or every one, and versions read the installed game's community result ([#60](https://github.com/MarahHQ/marah-launcher/issues/60)) ([614d559](https://github.com/MarahHQ/marah-launcher/commit/614d559abaa67506aa9c8cce8d9b4de6014f8791))
* Send ratings on Windows, and download and install on one drive and after a restart ([#58](https://github.com/MarahHQ/marah-launcher/issues/58)) ([4649009](https://github.com/MarahHQ/marah-launcher/commit/4649009be360658bb761ce5a157c8e2f1c91bec0))

## [0.11.0](https://github.com/MarahHQ/marah-launcher/compare/v0.10.1...v0.11.0) (2026-11-18)


### Features

* a forgiving game search, with best matches and recent searches on the keyboard ([#54](https://github.com/MarahHQ/marah-launcher/issues/54)) ([de394ff](https://github.com/MarahHQ/marah-launcher/commit/de394ffee8f146a30379da0978bc8a100ac6b4ae))
* a notice tells the player when the battery falls to 20% and to 10% ([#51](https://github.com/MarahHQ/marah-launcher/issues/51)) ([8f60c68](https://github.com/MarahHQ/marah-launcher/commit/8f60c68a50a5bcd26a0f90a346345170b5e67072))
* **catalog:** one card per game across consoles, with the version best for this PC ([#56](https://github.com/MarahHQ/marah-launcher/issues/56)) ([bfb6b9e](https://github.com/MarahHQ/marah-launcher/commit/bfb6b9eb05902f3a9f87b7e8a0096c312a4d16cf))
* **core:** an AppImage update downloads only the blocks that changed ([#53](https://github.com/MarahHQ/marah-launcher/issues/53)) ([8df3592](https://github.com/MarahHQ/marah-launcher/commit/8df359293b1db0a1aa06e2aa2c8ec551e1c0796a))
* download and install in one step, with a choice of files and the drives' free space ([#57](https://github.com/MarahHQ/marah-launcher/issues/57)) ([e4403fa](https://github.com/MarahHQ/marah-launcher/commit/e4403fa3f57fc3049a5d363f956724b3de21d8ba))
* Send ratings sends the player's results to the community compatibility lists ([#55](https://github.com/MarahHQ/marah-launcher/issues/55)) ([17e3d3e](https://github.com/MarahHQ/marah-launcher/commit/17e3d3e239edd6f8e143f0e7c8536224776f32ed))

## [0.10.1](https://github.com/MarahHQ/marah-launcher/compare/v0.10.0...v0.10.1) (2026-11-18)


### Bug Fixes

* **shadps4:** in a PS1 game on PS4, the controller's Start is START ([#49](https://github.com/MarahHQ/marah-launcher/issues/49)) ([254935d](https://github.com/MarahHQ/marah-launcher/commit/254935df9b2d5a1acdf4eaf42015bcb6a5b201d7))

## [0.10.0](https://github.com/MarahHQ/marah-launcher/compare/v0.9.0...v0.10.0) (2026-11-18)


### Features

* **shadps4:** an Ask at start switch for the PS4 system files; backups leave them out ([#45](https://github.com/MarahHQ/marah-launcher/issues/45)) ([86993a4](https://github.com/MarahHQ/marah-launcher/commit/86993a4be78998f5703b278d148218fb15b73907))


### Bug Fixes

* **shadps4:** no Save Migration box at shadPS4's first start when there are no old saves ([#48](https://github.com/MarahHQ/marah-launcher/issues/48)) ([ad384e1](https://github.com/MarahHQ/marah-launcher/commit/ad384e13dd9f2bd6a38a5b876862a0cdda5eef8a))
* **shadps4:** the touchpad button clicks the right half, so PS1 games on PS4 start ([#47](https://github.com/MarahHQ/marah-launcher/issues/47)) ([60359ce](https://github.com/MarahHQ/marah-launcher/commit/60359ce9d75a0c3b2d60d5e96d5cb2ec0164d95e))

## [0.9.0](https://github.com/MarahHQ/marah-launcher/compare/v0.8.0...v0.9.0) (2026-11-17)


### Features

* **shadps4:** the start asks to install the missing PS4 system files ([#43](https://github.com/MarahHQ/marah-launcher/issues/43)) ([3a6c83a](https://github.com/MarahHQ/marah-launcher/commit/3a6c83a048cc294df0119d15eaab632ccc30cccf))

## [0.8.0](https://github.com/MarahHQ/marah-launcher/compare/v0.7.1...v0.8.0) (2026-11-17)


### Features

* the Switch firmware asks before it installs; consoles on the boot screen; parallel installs ([#40](https://github.com/MarahHQ/marah-launcher/issues/40)) ([5e2a4ec](https://github.com/MarahHQ/marah-launcher/commit/5e2a4ec94b64b164f29b1519f3a1233ecd4224fd))

## [0.7.1](https://github.com/MarahHQ/marah-launcher/compare/v0.7.0...v0.7.1) (2026-11-17)


### Bug Fixes

* the boot screen says when no own build is picked, and that a first firmware download could not install ([#38](https://github.com/MarahHQ/marah-launcher/issues/38)) ([fdd856e](https://github.com/MarahHQ/marah-launcher/commit/fdd856e0f46e781a394f80fd4fd9e6793589df45))

## [0.7.0](https://github.com/MarahHQ/marah-launcher/compare/v0.6.0...v0.7.0) (2026-11-17)


### Features

* the boot screen shows and waits for the start's installs; Switch firmware updates on its own ([#37](https://github.com/MarahHQ/marah-launcher/issues/37)) ([2d76d95](https://github.com/MarahHQ/marah-launcher/commit/2d76d95144e5fe80f3944caf7419524e979ddfbf))


### Bug Fixes

* **core:** Home's systems and the Emulators list go by name; services last ([#35](https://github.com/MarahHQ/marah-launcher/issues/35)) ([ceee1fc](https://github.com/MarahHQ/marah-launcher/commit/ceee1fc835ccab734ea7a49e725d779e1a0fb450))

## [0.6.0](https://github.com/MarahHQ/marah-launcher/compare/v0.5.2...v0.6.0) (2026-11-17)


### Features

* **core:** emulators with no build install a minute after the start ([#32](https://github.com/MarahHQ/marah-launcher/issues/32)) ([992479a](https://github.com/MarahHQ/marah-launcher/commit/992479ae680e0b7644f9e1911f3a54b594856b2c))


### Bug Fixes

* **core:** emulators install and update right after the start, not a minute later ([#34](https://github.com/MarahHQ/marah-launcher/issues/34)) ([ec1ad77](https://github.com/MarahHQ/marah-launcher/commit/ec1ad7789eb6cabe7c8c5f07f5718ee91247e0f3))

## [0.5.2](https://github.com/MarahHQ/marah-launcher/compare/v0.5.1...v0.5.2) (2026-11-17)


### Bug Fixes

* **deny:** accept the chrono unmaintained advisory that comes in through Slint ([#29](https://github.com/MarahHQ/marah-launcher/issues/29)) ([6a2d6ab](https://github.com/MarahHQ/marah-launcher/commit/6a2d6ab73f8f5073727482464f0b7746eee18db9))
* **packaging:** the AppImage takes libxkbcommon from the system; its own copy crashed at start on Fedora 45 ([#31](https://github.com/MarahHQ/marah-launcher/issues/31)) ([5e01eb8](https://github.com/MarahHQ/marah-launcher/commit/5e01eb87d1c34c35f9cc4a1e5897a556b9f5078f))

## [0.5.1](https://github.com/MarahHQ/marah-launcher/compare/v0.5.0...v0.5.1) (2026-11-17)


### Bug Fixes

* **kit:** a plugin's page ends with About (its blocks and facts), and another plugin's page opens at its top ([#27](https://github.com/MarahHQ/marah-launcher/issues/27)) ([b0ca42b](https://github.com/MarahHQ/marah-launcher/commit/b0ca42b0b80e23aaa2caebaaad821ebcb2036eca))

## [0.5.0](https://github.com/MarahHQ/marah-launcher/compare/v0.4.0...v0.5.0) (2026-11-17)


### Features

* **kit:** a plugin's page and a game's own settings are one page like Settings ([#25](https://github.com/MarahHQ/marah-launcher/issues/25)) ([1d87954](https://github.com/MarahHQ/marah-launcher/commit/1d87954ea47beb4184b98123616a41f8193c7db9))

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
