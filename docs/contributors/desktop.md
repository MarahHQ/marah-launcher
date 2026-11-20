# The desktop

This page says what the launcher does with windows, focus, controllers, start at login and
links on Linux, macOS and Windows, and which tool or permission each system needs. It is for
contributors who change that code.

The layers around this code are in [architecture.md](architecture.md). The terms are in
[the glossary](../../GLOSSARY.md).

## Where the code is

The core never calls the system. It calls four ports in `crates/core/src/ports.rs`, and the app
plugs in the real adapters in `crates/app/src/main.rs`.

| Port | Real adapter | Stand-in when none is plugged in |
| --- | --- | --- |
| `Windows` | `SystemWindows` in `desktop.rs` | `NoWindows`: the front is `Front::Unknown`, and nothing comes to the front |
| `LoginSession` | `SystemLoginSession` in `desktop.rs` | `NoLoginSession`: refuses start at login and package installs |
| `Opener` | `SystemOpener` in `desktop.rs` | `NoOpener`: refuses links and paths |
| `ProcessTree` | `SystemProcesses` in `processes.rs` | `GroupTree` in `crates/core/src/process.rs`: the root's process group, and nothing on Windows |

The sessions subsystem (`crates/core/src/sessions/`, with the watch in `watch.rs`) uses
`Windows` and `ProcessTree` to follow a running game. `preferences.rs` uses `LoginSession`.

Two more parts run on the UI thread and are not ports of the core:

- `platform.rs` is the app shell's `Platform`. It raises the launcher's window with winit,
  moves it to a monitor, and tells the shell whether the window has the focus.
- `gamepad.rs` reads controllers and sends each press to the app shell.

## What each system needs

The launcher finds a game's window by the game's process ids. Each system does that with its
own tool.

| System | Tool | The player needs |
| --- | --- | --- |
| Linux, X11 | `xdotool` | `xdotool` on the `PATH`, and `DISPLAY` set |
| Linux, Gamescope | `xdotool` and `xprop` | Both on the `PATH`. Gamescope sets `GAMESCOPE_WAYLAND_DISPLAY`. |
| Linux, KDE Plasma on Wayland | KWin scripting over D-Bus | A session bus and KWin. Nothing to install. |
| Linux, another Wayland desktop | `xdotool`, for games under XWayland only | `DISPLAY` set (XWayland) |
| macOS | `osascript`, driving System Events | The Accessibility permission, and permission to control System Events |
| Windows | Win32 calls through `windows-sys` | Nothing to install |

On Linux, `desktop.rs` tries the ways in this order (`system::front`, `system::show`,
`system::has_window`):

1. KWin, when `kwin::Kwin::get()` finds it.
2. Gamescope, when `GAMESCOPE_WAYLAND_DISPLAY` is set.
3. `xdotool`.

`kwin::Kwin::get()` connects once per run. It returns `None` unless all of these hold:
`WAYLAND_DISPLAY` is set, `XDG_CURRENT_DESKTOP` has a `KDE` part, `GAMESCOPE_WAYLAND_DISPLAY`
is not set, a session bus is there, and KWin answers a first test script within `ANSWER_TIME`
(500 ms). With `None`, the `xdotool` way stands in.

### What the player is told

`Windows::problem` returns a sentence that names the fix. The sessions subsystem asks once per
run, when the first game starts, and shows the answer as a notice.

- Linux: "Install xdotool so games come to the front and the launcher comes back", with
  "xdotool and xprop" under Gamescope. No notice when KWin works, when `DISPLAY` is not set, or
  when the tools are on the `PATH`.
- macOS: a sentence about the Accessibility permission and Automation. The launcher tests the
  permission by counting the windows of its own process through System Events.
- Windows: never.

## Bring a window to the front

`Windows::show_game(ids)` brings a window of one of the processes `ids` to the front. It returns
`false` when none has a window yet, or when the system refuses.

**Linux, X11.** `xdotool search --onlyvisible --pid <id>` lists the windows of each process. The
launcher runs `xdotool windowactivate` on the last window found. `front()` runs
`xdotool getactivewindow getwindowpid`.

**Linux, Gamescope.** Gamescope has no window manager. It shows the windows whose `STEAM_GAME`
id comes first in the root property `GAMESCOPECTRL_BASELAYER_APPID`.

- For a game, the launcher finds its windows with `xdotool`, sets `STEAM_GAME` on each with
  `xprop` to the id of the game's first process, and sets `GAMESCOPECTRL_BASELAYER_APPID` to
  that id, then `LAUNCHER_APPID` (7), so the launcher stays behind it.
- For the launcher, it sets `LAUNCHER_APPID` on the launcher's window and puts it alone in the
  list.
- `front()` reads `GAMESCOPE_FOCUSED_WINDOW` from the root window with `xprop`, then asks
  `xdotool getwindowpid` for its process.
- While a game is in front, `keep_game` gives each new window of the game's processes the
  game's id, so the windows a game opens after it showed stay on screen.

Gamescope's Steam mode shows no window that has no id. `desktop::claim_gamescope` sets the
launcher's id once its window exists. The app shell calls it through `Platform::claim_window`
on each tick until it returns `true`. Outside Gamescope, `claim_window` returns `true` at once.

**Linux, KDE Plasma on Wayland.** `kwin::Kwin::ask` runs one question at a time:

1. It writes a JavaScript file in the temp folder. `kwin::script` puts the prelude in front of
   the question.
2. It loads the file with `org.kde.kwin.Scripting.loadScript` on `org.kde.KWin`, and runs it.
   The run path is `/Scripting/Script<id>` on KWin 6 and `/<id>` on KWin 5.
3. The script calls `answer` on an object the launcher serves on the session bus. The launcher
   waits `ANSWER_TIME` for it.
4. It unloads the script and deletes the file.

The prelude uses the KWin 6 names (`windowList`, `activeWindow`) and falls back to the KWin 5
names (`clientList`, `activeClient`). A window counts when its pid is in `ids` and it is a
normal window or a dialog. `show` un-minimizes the last matching window, then makes it active.
KWin sees Wayland and XWayland windows alike.

**Linux, another Wayland desktop.** No program may raise another program's window. The launcher
raises its own window through winit (`platform::raise_launcher`). A game under XWayland still
comes to the front through `xdotool`, because `DISPLAY` is set.

**macOS.** Every call is an `osascript -e` command to System Events:

- `front()`: `unix id of first process whose frontmost is true`.
- `has_window(ids)`: `count windows of (first process whose unix id is <id>)` for each id.
- `show(ids)`: for the first process with a window, `set frontmost of ... to true`.

Without the Accessibility permission every call fails. `front()` is then `Front::Unknown`, and
the core goes by the last window it showed. macOS keeps a program running after its last window
closes, so `SystemWindows::window_close_ends_game` returns `true` on macOS only.

**Windows.** `windows()` calls `EnumWindows` and keeps the visible windows with no owner (the
windows of the taskbar) whose process is in `ids`. `front()` calls `GetForegroundWindow`.
Windows lets only the program in front give the front away. `bring_to_front` restores a
minimized window, joins the input of the window in front with `AttachThreadInput` for the call,
runs `BringWindowToTop` and `SetForegroundWindow`, and detaches again.

### The launcher's own window

`SystemWindows::show_launcher` calls its `raise` closure first. In the app, that is
`platform::raise_launcher`: on the UI thread, it un-minimizes the window (not on Wayland, which
can't tell and warns on every try), then calls `focus_window`. Then it calls the system's
`show` for the launcher's own process id. Gamescope needs that call. Elsewhere it is a second
try, and a failure logs a debug line.

## Follow a game

A game is its processes, and a session lasts until none of them is left.
`SystemProcesses::members(root)` finds them:

- **Unix (Linux and macOS).** `pgrep -g <root>` lists the process group. The launcher starts
  each game in a group of its own (`process_group(0)` in `crates/core/src/process.rs`), so the
  group holds the programs the game started. `root` is added while `kill -0` says it runs. Then
  `ps -A -o pid=,ppid=` gives the parents, and `and_below` adds every process below the ones
  found. A sandbox that starts its programs in a session of its own, as Proton's
  pressure-vessel does, stays covered.
- **Windows.** A ToolHelp snapshot gives the parents. Windows has no process groups, so the
  game is the root and every process below it. An empty list means the root is gone.

A start script that starts the game in the background and exits keeps its session, because the
background program is still in the group.

### A game another program starts

A launch can set `LaunchSpec::game_process`, as the Steam plugin does for
`steam steam://rungameid/...`. The launcher can't follow the process it started, so it follows
another one:

1. Before the launch, `ProcessTree::find(text)` lists the processes that already have those
   words in their command line. Windows has no command lines there: `find` reads each
   program's path (`QueryFullProcessImageNameW`) and compares it in any case and with either
   slash.
2. After the launch, the watch calls `find` on each look and takes the first new match. A
   process counts only when its parent doesn't match too (`topmost`), so Steam's first process
   of the game is taken, not one it started.
3. The program the launch started is only waited for in the background. It is never stopped,
   because it may have become Steam's client.
4. If nothing shows within `HostConfig::hand_over_timeout`, or Stop comes first, the session
   ends with nothing played.

On Unix, `find` reads `ps -A -ww -o pid=,ppid=,args=`. On Windows it returns nothing: the
ToolHelp snapshot has no command lines.

A launch can also set `LaunchSpec::game_env`, as the Steam plugin sets `SteamAppId`. Steam's
runtime runs the game outside its reaper's tree, so the tree alone misses the game's window and
ends the session when the reaper exits. With `game_env`, each look adds every process whose
environment has that whole entry (`ProcessTree::with_env`, from `/proc/<pid>/environ` on Unix),
and those below it. On Windows `with_env` returns nothing.

### The watch loop

`sessions/watch.rs` runs the loop. Each look asks the desktop through `spawn_blocking`, because
the tools are programs.

| Stage | What happens |
| --- | --- |
| Until the game shows | Every `SHOW_EVERY` (250 ms), or `HostConfig::watch_every` when that is shorter, the watch lists the processes and calls `show_game`. |
| The game shows | `show_game` returned `true`, or `HostConfig::show_timeout` passed. The session is then shown. |
| Shown, but the desktop can't say | After the timeout, the watch asks `front()`. If it is `Front::Unknown`, the game counts as in front: its own window took the screen. |
| After it shows | Every `HostConfig::watch_every`, the watch reads the processes, `front()` and `has_window`, and calls `keep_game` while the game is in front. |
| The end | The game ends when no process is left and the first one exited. |

After each look, the watch sets `game_in_front`:

- `Front::Launcher`: false.
- `Front::Other(id)`: true when `id` is one of the game's processes.
- `Front::Unknown`: no change. The last window the launcher showed counts.

On macOS, a game also ends when its window is gone for `WINDOW_GONE_FOR` (3) looks in a row.
The watch counts only after it has seen a window of the game, so a desktop that can't see the
window never ends a game this way.

When the first process of the game exits, the watch kills what is left in its group. On Windows
it also runs a PowerShell walk (`orphans_of` in `process.rs`) over the processes made since the
game started.

### Stop

Stop asks the game to close, and kills it if it doesn't:

1. `ProcessHandle::terminate` signals the group: `kill -TERM -- -<group>` on Unix,
   `taskkill /PID <id> /T` on Windows. `ProcessTree::end(ids, false)` sends `kill -TERM` to each
   process in the list, or `taskkill /PID` on Windows.
2. The watch looks every `STOP_EVERY` (50 ms) until `HostConfig::stop_grace` passes.
3. It then kills: `ProcessHandle::kill`, then `ProcessTree::end(left, true)` (`kill -KILL`, or
   `taskkill /F`).

When the game ends and the player turned on `return_on_exit`, the core calls `show_launcher`.

## The controller

`gamepad.rs` reads controllers with `gilrs` on a thread named `gamepad`. DualSense, DualShock 4,
Xbox and most pads with a standard mapping work. If `gilrs` can't start, the launcher logs
`no controller support` and goes on without a controller.

`gamepad::button` maps the `gilrs` buttons to `PadButton`. `PadButton::action` in
`crates/ui-host/src/input.rs` maps those to the keys a theme gets. The table of buttons and keys
is in [the theme reference](../themes/reference.md). The PS or Xbox button (`Guide`) is no key:
it is a host action.

- Only the left stick moves. It counts as a press past `PUSH` (0.6) and lets go below
  `RELEASE` (0.3). A value between the two keeps the last direction.
- A held button or stick repeats after `FIRST_REPEAT` (380 ms), then every `NEXT_REPEAT`
  (85 ms). Each repeat is marked, so the host sends it to the theme as a repeated key press.
  The reader waits at most `LONGEST_WAIT` (50 ms) for an event before it checks the held
  buttons.
- The Guide button never repeats, and nothing goes out when it goes down. A release before
  `GUIDE_HOLD` (600 ms) sends the tap. Still down at `GUIDE_HOLD`, it sends the hold once
  (`GuideHold`).
- A controller that comes or goes clears every held button. `gamepad::spawn` also reports the
  name of the first connected controller, for the launch splash.

`ThemeHost::pad` (`crates/ui-host/src/host.rs`) drops every press but the Guide tap and hold
when a game's window is in front, or when the launcher's window doesn't have the focus. The
focus comes from `Platform::window_focused`, which asks winit. With no window to ask, it is
`true`.

- Tap: go to Home. While a game runs, nothing happens.
- Hold: while a game runs, Stop every running session. With none running, open the power menu.

`SessionCmd::ToggleFocus` switches between the launcher and the game, but nothing sends it: no
button, and no kit file.

**SIGCHLD on Unix.** Each program the launcher runs ends with `SIGCHLD`. On Linux it broke the
wait in `gilrs`, with an error in the log. `ignore_child_exits` blocks `SIGCHLD` on the
controller thread and on the thread `gilrs` starts from it. Tokio still gets the signal on its
own threads.

## Full screen and monitors

`Platform::move_to_monitor` (`platform.rs`) moves the window to the monitor named by the
`monitor` setting, or by `--monitor`. It turns full screen off for the move and on again after,
so the window fills the new monitor. The app shell calls it on each tick when the wanted monitor
changes.

On macOS, full screen is winit's simple full screen (`platform::simple_fullscreen`): the window
fills the screen in the current Space. In a Space of its own, bringing the app to the front
after a game closed didn't switch back, and the launcher stayed out of sight.

## Start at login

`desktop::login::set` writes and removes the entry on each system. `SystemLoginSession` uses it
only when `update::login_program` finds the installed launcher: the AppImage file, the binary
of a package, or the binary in the macOS app. A build from `cargo run` has none, so
`can_start_at_login` is `false`, and the setting fails with "this build can't start when you
log in".

| System | Entry | Notes |
| --- | --- | --- |
| Linux | The file named by `IDS_AUTOSTART_FILE` in `$XDG_CONFIG_HOME/autostart`, else `~/.config/autostart` | `XDG_CONFIG_HOME` counts only as an absolute path. `Exec` is quoted as the Desktop Entry specification says. |
| macOS | A launch agent named by the app id (`IDS_APP_ID`, `desktop::login::AGENT`) in `~/Library/LaunchAgents` | `RunAtLoad` is true. XML characters in the path are escaped. |
| Windows | The value named by `IDS_WINDOWS_RUN_VALUE` in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` | Written with `reg.exe add`, removed with `reg.exe delete`. The path is quoted. |

The names come from `brand.toml`, as `launcher_api::brand` constants. Turning the setting off
when no entry exists is not an error. When the system refuses, the error text goes to the view
as `start_at_login_error`, and the setting doesn't change.

### Install the launcher's own package

On Linux only, `SystemLoginSession::install_package` runs
`pkexec <dnf install -y | apt-get install -y | pacman -U --noconfirm> <file>`. Exit code 126 or
127 from `pkexec` means the player closed the prompt or typed a wrong password
(`PackageError::Cancelled`). Any other failure carries the last line of the error output that
isn't empty. On other systems the trait's default refuses. When the launcher uses this is in
[releasing.md](releasing.md).

## Open links and folders

`SystemOpener` starts the program and doesn't wait. A thread reaps it.

| System | Link | Path |
| --- | --- | --- |
| Linux | `xdg-open` | `xdg-open` |
| macOS | `open` | `open` |
| Windows | `rundll32 url.dll,FileProtocolHandler <link>` | `explorer` |

On Windows no shell reads the link. Each space, quote and control character in it is
percent-encoded (`windows_link_command`).

## The whole-screen session

On Linux, a session script runs the launcher in Gamescope with no desktop behind it. The
launcher's side of it is two lines on standard output, from `Platform::announce`:
`window shown` once the window has a size, and `log out` when the player chose Log out. Before
`log out`, `crates/core/src/closing.rs` closes what runs: the running games, as Stop closes
them, then what their launches left running, through the plugin's
`Launcher::close_left_running` (for Steam's client, `steam -shutdown`). `Vm.closing` names each
step. The script itself, its settings and what it does with each line are in
[the OS contract](os-contract.md).

The session changes how the launcher leaves, not what the player can manage. In a window on a
desktop the top bar and its tray have the same network, sound, Bluetooth and drives items, and Settings has the
same System section. Restart and Shut down show there too, when the system plugin allows them:
on Linux through `systemctl`, on Windows through `shutdown`, and on macOS through System Events
(`osascript`), which macOS asks the player to allow once.
They act on the whole PC, so on a desktop they ask in a dialog first (`kit/power-ask.slint`); in
the session they ask by a second press.

## Tests

The app's unit tests open no windows. They run with the rest (see
[development.md](development.md#tests)), and cover:

- `desktop.rs`: the KWin script text, the autostart entry, the launch agent, the package
  commands, the `pkexec` exit codes, and writing and removing an entry in a temp folder.
- `processes.rs`: `and_below`, `topmost`, the `ps` line reader, and (Unix) finding a real
  `sleep` by its command line and following a script's background program.
- `gamepad.rs`: the repeat times, the Guide tap and hold, the stick.

Two tests need a real desktop. Both are opt in, with the variables in
[development.md](development.md#tests):

- The desktop test, `crates/app/tests/desktop.rs`. It opens real windows, changes which one is
  in front, and writes and removes this user's start-at-login entry. It puts back the value it
  found. Run it on a desktop you can spare for a minute. It has its own `main`, because a
  window must run on the main thread of a process, and it starts itself again as a helper
  window. Its tests: `a_window_shown_comes_to_the_front`,
  `a_window_that_closed_is_gone_while_its_program_runs` and
  `start_at_login_writes_the_system_s_entry_and_takes_it_out`. A test skips, and says why, when
  the desktop doesn't answer, such as macOS without the Accessibility permission. A second
  variable turns a skip into a failure. Weekly CI runs it under Xvfb and openbox.
- The KWin test, `kwin_says_which_window_is_in_front_and_whose_windows_there_are` in
  `desktop.rs`. It asks the real KWin and activates a real window, so it needs a KDE Plasma
  session on Wayland. Without its variable it returns and passes.

## Limitations

- The launcher can't bring a native Wayland game to the front on a Wayland desktop other than
  KDE Plasma: no program may raise another's window there. `xdotool` sees XWayland windows
  only. On such a desktop with no `DISPLAY`, `has_window` and `front` return nothing.
- macOS needs the Accessibility permission. Without it, `front()` is `Front::Unknown`, no game
  comes to the front, and the launcher doesn't come back by itself.
- On Windows, a game another program starts can't be found: `ProcessTree::find` returns
  nothing, because the process snapshot has no command lines.
- `Windows::problem` on Windows never warns. A refused `SetForegroundWindow` shows only as
  `false` from `show_game`.
- `SessionCmd::ToggleFocus` has no sender.
- The controller thread keeps only the first connected controller's name for the launch splash,
  though `gilrs` reads all of them.
- The desktop test doesn't cover `SystemProcesses`, `SystemOpener` or the Gamescope way.
- The macOS and Windows parts of this page come from the code. They were not run for it.
