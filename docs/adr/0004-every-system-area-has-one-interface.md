# Every system area has one interface

Status: accepted

## Context

A system plugin managed eight parts of the machine: the network, VPNs, Bluetooth, sound, power,
the OS's updates, its installer and the drives. Each had its own trait (`NetworkSettings`,
`VpnSettings`, `BluetoothSettings`, `AudioSettings`, `PowerSettings`, `OsUpdates`, `OsInstall`,
`DriveSettings`), its own field in `Provided` and its own read in the core. The traits mirrored
one Linux tool each, with 2 to 8 methods. "Can't be used here" came in four forms: an async
`unavailable()`, the VPN's sync `problem()`, an `Err(NotFound)` from the OS updates, and a gate
on `Place::stick`. The view model had an `*_available`, `*_loading` and `*_reason` field for
some of them. An adapter for another OS had to write every method, even the ones that can't
work there, and a new part meant editing the `Capability` list, `Provided`, its
`capabilities()`, the core's reads and the view model by hand.

## Decision

- Each part is a system area, `SystemArea` in `crates/api/src/system.rs`. Every area has the
  same interface, `AreaSettings`:
  - `state()` returns `AreaState`: `Absent` (nothing to manage here), `Unsupported(reason)`
    or `Ready(AreaData)`, with one kind of data per area.
  - `act(action, job)` runs one `AreaAction`. Each action belongs to one area. A long action
    reports through the job, and a quick one leaves it alone. An adapter returns
    `PluginError::Unsupported` for an action it can't run.
- `Provided::areas` is a map keyed by `SystemArea`. `Provided::capabilities()` names each key's
  capability, so the manifest and the kind check of ADR-0001 still see them.
- Loading lives in the core, not in the plugins. The core has one reader, `system::read`. An
  area is loading until its first read ends, so no adapter reports it. The reader also makes an
  area that no plugin provides absent, and keeps the OS installer absent off its USB stick.
- A read that fails changes nothing. A failed first read makes the area unsupported, with the
  error, so it doesn't stay loading.
- What one area has of its own stays in the area's data, not in a fourth state. The power
  area's data has `refused`: restart and shut down can be refused while the battery still shows.
  The VPN area's data has `finds_vpns` and `can_import`.
- The core's ports stay: the drives' `Adapter` with `MountFolders` and `PluginDrives`, and the
  VPN's `Network` with `PluginVpn`. The plugin's area is one adapter behind each.
- The view model lists every area in `SystemVm::areas`, in a fixed order (`AREAS` in
  `crates/ui-api/src/vm.rs`, `Areas` in `types.slint`), each with its state, its reason and
  whether a job runs in it. Slint functions can't loop, so a theme reads an area by its place.
- A status icon whose area is absent or unsupported shows disabled: in place, dimmed, doing
  nothing when activated, still focusable, with its reason on hover and focus. The kit's
  `StatusState` draws it for every theme.

## Consequences

- `API_VERSION` goes from 0.3.5 to 0.4.0, and `UI_API_VERSION` from 0.6.0 to 0.7.0. Every
  `plugin.toml` `host_api` and every built-in theme's `ui_api` change in the same commits. A
  theme built for 0.6 is refused for its version
  (`a_theme_built_for_0_6_is_refused_for_its_version_and_not_compiled` in
  `tests/tests/seams/themes.rs`).
- A new area is one `SystemArea` variant, one `AreaData` variant, its actions, and an adapter.
  Its icon in the top bar is still the theme's.
- The VPN rule doesn't know the system's VPN facts until the area's first read. Before it,
  `PluginVpn` says there is no problem, so "Download only through a VPN" holds and the
  downloads wait for the read; the read reads the facts too. This keeps the boot free of a read
  of the machine before the first frame.
- One read of the drives area gives both its state and the list, and the list's read waits for
  a turn of the disk. So the drives area is read on its own tick and by the drive commands, not
  by a refresh of the other areas: a change of volume never waits for the disk
  (`a_tick_asks_once_why_drives_can_t_be_managed` in `tests/tests/seams/drives.rs`). A list read
  before an action read the drives again isn't kept over the newer one
  (`a_list_read_before_a_mount_doesn_t_undo_the_mount` in `crates/core/src/drives/tests.rs`).
- The view model keeps `network_available`, `audio_available`, `bluetooth_available` and
  `drives_available` as "the area is ready", and `power_available` as "restart and shut down
  work here". The shell's tests read `audio_available`.
- Tests guard it: `a_failed_read_keeps_the_last_state_and_a_failed_first_read_says_why`,
  `a_power_area_that_refuses_restart_still_shows_the_battery`,
  `the_vpn_area_s_reason_shows_after_its_first_read` and
  `the_view_lists_every_area_in_the_contract_s_order` in `tests/tests/seams/system.rs`;
  `the_drives_icon_shows_disabled_when_drives_turn_out_unavailable` and
  `a_disabled_icon_takes_the_focus_and_says_why_but_opens_nothing` in
  `tests/tests/seams/system_theme.rs`; and
  `the_areas_global_places_each_area_where_the_view_lists_it` in
  `crates/ui-api/src/contract.rs`.
- Not decided here: when the top bar shows the network, sound and Bluetooth icons. That is still
  `Place::whole_screen`, and splitting it is a later step.
