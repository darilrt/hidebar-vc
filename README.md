# hidebar

A minimal status overlay for **Hyprland**, **Niri** and **Windows 11**, written in Rust.

It is not a bar: it reserves no screen space, never takes the cursor (clicks pass through it) and only shows up
when you call it with a keybinding or when something relevant happens (you switch workspace, the network drops,
you plug in the charger…). The rest of the time nothing is drawn.

![hidebar with every module pinned over the desktop](docs/screenshot.jpg)

![Close-up of the top strip: workspaces, clock, and network, CPU, RAM, mute and battery](docs/strip.png)

> [!WARNING]
> **This project is 100% vibe coded.** All of the code, icons and documentation were written by an AI agent
> (Claude) following natural-language instructions; no human wrote or reviewed the code line by line. It works on
> my machine and I use it daily, but take it for what it is: a personal experiment, with no warranty, no support and
> no stability promises. Read it before you run it.

## What it shows

Each module is drawn in its own semi-transparent black pill near the top edge of the screen.

| Module       | Where             | Content                                    | Shows up on its own when…                   |
|--------------|-------------------|--------------------------------------------|---------------------------------------------|
| `workspaces` | top left          | one dot per workspace; the active one filled | you switch workspace (1.5 s)              |
| `clock`      | top center        | time and date                              | —                                           |
| `status`     | top right         | network · cpu · ram · mute · battery       | — (see notices)                             |

### Notices

When something changes, hidebar does not open the whole `status` pill: it shows **only the icon of what changed**,
in a small pill in the same top-right spot, for 3 s.

- **Network:** the network icon, when connectivity changes.
- **Audio:** a crossed-out speaker when the output goes silent, a regular speaker when sound comes back.
- **Charger:** the battery icon when you plug in (yellow) or unplug (white) the charger.
- **Low battery:** the battery icon, pulsing, for as long as the battery is low (≤ 25%, white; ≤ 10%,
  orange-red) and discharging.

Simultaneous notices stack in the same pill, in `status` order (network · mute · battery); repeating one restarts
its timer. While `status` is visible (`show status` or pinned) there are no notices, since it already contains
everything.

### `status` in detail

- **Network:** white wifi icon with internet access; orange with a local network but no way out; crossed out in
  orange-red with no default route. Checks `/proc/net/route` every 5 s and, if there is a route, tries a TCP
  connection to `1.1.1.1:443` (Cloudflare) every 30 s, without sending any data.
- **CPU and RAM:** usage percentage read from `/proc/stat` and `/proc/meminfo` every 3 s.
- **Mute:** crossed-out speaker only when the default audio output is muted or at 0%. Uses `pactl` (PipeWire or
  PulseAudio) and updates instantly through `pactl subscribe`.
- **Battery:** `/sys/class/power_supply/BAT*` every 10 s. White normally, pulsing when low, pulsing orange-red when
  critical, yellow while charging. Omitted when there is no battery.

### Calendar

`hidebar toggle clock` opens the current month under the clock, with today highlighted. `‹` `›` or the ←/→ arrow
keys change month; clicking the title or pressing `Home` goes back to the current month. Close it with `Esc`,
another `toggle clock`, or by clicking another window. It is the only part of the overlay that takes cursor and
keyboard input.

The interface (dates, CLI messages) is in Spanish.

## Requirements

### Linux

- Hyprland or Niri (uses `wlr-layer-shell`; workspaces come from the compositor's IPC).
- Rust 1.88 or later (2024 edition).
- Optional: `pactl` for the mute icon and audio notices.
- Vulkan drivers for your GPU (e.g. `vulkan-radeon`, `vulkan-intel`, `nvidia-utils`). The daemon uses Vulkan by
  default, which keeps memory low (~120 MB RSS instead of ~275 MB when GL/EGL loads every installed driver).
  Set `WGPU_BACKEND=gl` to override it.

### Windows 11

- Windows 11 on x64.
- To build: Rust 1.88+ with the **MSVC** toolchain and Visual Studio Build Tools
  (workload "Desktop development with C++", which includes the Windows SDK).
- Nothing else at runtime: no GPU, no drivers, no admin. The overlay is rendered with
  `tiny-skia` on the CPU and presented with `UpdateLayeredWindow`; it uses ~20 MB RSS and no
  wgpu/Vulkan/DX12.

## Installation

```sh
git clone https://github.com/darilrt/hidebar-vc.git hidebar
cd hidebar
cargo install --path . --locked --root ~/.local
```

- Builds in release mode and puts the binary at `~/.local/bin/hidebar`.
- `--locked` is **required**: it uses the exact versions in `Cargo.lock`. Without it, cargo picks a newer
  `winit-core` that one dependency (`iced_exdevtools`) fails to build against.
- `--root ~/.local` is optional; without it the binary goes to `~/.cargo/bin`.

To update, run the same command again and restart the daemon. To uninstall:
`cargo uninstall hidebar --root ~/.local`.

### Windows 11

```powershell
cargo build --release
.\target\release\hidebar.exe daemon
```

The binary is `target\release\hidebar.exe` (or
`target\x86_64-pc-windows-msvc\release\hidebar.exe` if you pass `--target` explicitly).
`--locked` is recommended here too.

To start it at login (current user, no admin):

```powershell
.\hidebar.exe install
.\hidebar.exe uninstall
```

`install` writes a single value under
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. When the daemon owns its own console
(started by the Run key or a shortcut) it hides it; set `HIDEBAR_FOREGROUND=1` to keep it
visible for debugging. The daemon logs to `%LOCALAPPDATA%\hidebar\hidebar.log` (rotated at 1 MB).

## Usage

```text
hidebar daemon             start the daemon
hidebar show <module>      show the module for a few seconds
hidebar toggle <module>    depends on the module:
                             workspaces, status → pin / unpin (stays visible until the next toggle)
                             clock              → open / close the calendar
hidebar show all           all three modules for a few seconds
hidebar toggle all         if anything is pinned, unpin everything; otherwise pin all three
hidebar quit               stop the daemon
```

A pinned module never hides on its own and is not affected by automatic appearances.

The client talks to the daemon over a Unix socket at `$XDG_RUNTIME_DIR/hidebar.sock` on Linux and
over a named pipe (`hidebar-<username>`) on Windows, then exits immediately. If the daemon is not
running, it says so and exits with an error.

## Windows 11

The same strip and calendar, without a compositor: two layered Win32 windows rendered with
`iced_tiny_skia` and presented with `UpdateLayeredWindow`.

![hidebar on Windows 11: clock, status and calendar over a dark desktop](docs/windows.png) The gaps between pills are fully
transparent and click-through, the strip is always on top, does not appear in the taskbar or
Alt+Tab, never steals focus, and hides itself while a game or presentation is fullscreen.

Global hotkeys (registered with `RegisterHotKey`):

| Keys | Action |
|---|---|
| `Ctrl+Alt+Win+T` / `B` / `A` | show clock / status / all |
| `Ctrl+Alt+Win+1` | open or close the calendar |
| `Ctrl+Alt+Win+2` / `3` | pin-unpin status / all |

They deliberately avoid `Ctrl+Shift+T/W/B` (reopen tab, close window, bookmarks bar) and are not
AltGr combinations, so they do not block typing. There is no workspaces hotkey because Windows has
no source for it (see notes). Stop the daemon with `hidebar quit`. Set
`HIDEBAR_NO_HOTKEYS=1` to disable all hotkeys without recompiling, or edit
`src/platform/windows/hotkeys.rs` and rebuild to change the defaults.

Notes:

- `workspaces` is Linux-only for now: Windows has no equivalent compositor IPC, so
  `show workspaces` does nothing and it has no hotkey.
- The calendar opens with `hidebar toggle clock` or `Ctrl+Alt+Win+1` and closes when it loses
  focus (clicking another window), by design. When opened from a terminal, Windows may not give
  it keyboard focus (foreground-lock rules); click it or use the hotkey, which does focus it.
- v1 draws on the primary monitor only.
- There is no GPU dependency; idle RSS is ~20 MB and measurements are done with
  `GetSystemTimes`, `GlobalMemoryStatusEx`, `GetSystemPowerStatus`, `GetAdaptersAddresses` and
  Core Audio (`IAudioEndpointVolume`). Mute is polled every ~2 s (the default endpoint is
  re-enumerated every 10 s), so the mute icon/notice can take up to 2 s.
- The daemon uses a named pipe (`hidebar-<username>`); `HIDEBAR_SOCKET` overrides it with another
  name (not a filesystem path). `hidebar install`/`uninstall` manage the per-user autostart.

## Hyprland configuration

In `~/.config/hypr/hyprland.conf`:

```ini
# Full path: Hyprland's PATH may not include ~/.local/bin or ~/.cargo/bin.
$hidebar = ~/.local/bin/hidebar

exec-once = $hidebar daemon

# Ctrl+Shift = peek; Ctrl+Shift+Alt = toggle.
bind = CTRL SHIFT, T, exec, $hidebar show clock
bind = CTRL SHIFT ALT, T, exec, $hidebar toggle clock         # calendar
bind = CTRL SHIFT, B, exec, $hidebar show status
bind = CTRL SHIFT ALT, B, exec, $hidebar toggle status        # pin network/cpu/ram/battery
bind = CTRL SHIFT, W, exec, $hidebar show workspaces          # optional: they appear on their own when switching
bind = CTRL SHIFT ALT, W, exec, $hidebar toggle workspaces
bind = CTRL SHIFT, A, exec, $hidebar show all
bind = CTRL SHIFT ALT, A, exec, $hidebar toggle all
```

To apply without logging out: `hyprctl reload` and, the first time, start the daemon from Hyprland (so it does not
depend on your terminal; `exec-once` only runs at login):

```sh
hyprctl dispatch exec ~/.local/bin/hidebar daemon
```

Note: a Hyprland `bind` takes priority over applications. With these bindings, `Ctrl+Shift+T`, `W`, `B` and `A` no
longer reach your browser (reopen tab, close window, bookmarks bar, add-ons). If you use them, pick other keys.

Restart the daemon after updating the binary:

```sh
pkill -x hidebar; hyprctl dispatch exec ~/.local/bin/hidebar daemon
```

## Niri configuration

In `~/.config/niri/config.kdl`, add a startup and binds (adjust the path):

```kdl
spawn-at-startup "~/.local/bin/hidebar" "daemon"

binds {
    Ctrl+Shift+T { spawn "~/.local/bin/hidebar" "show" "clock"; }
    Ctrl+Shift+Alt+T { spawn "~/.local/bin/hidebar" "toggle" "clock"; }
    Ctrl+Shift+B { spawn "~/.local/bin/hidebar" "show" "status"; }
    Ctrl+Shift+Alt+B { spawn "~/.local/bin/hidebar" "toggle" "status"; }
    Ctrl+Shift+W { spawn "~/.local/bin/hidebar" "show" "workspaces"; }
    Ctrl+Shift+Alt+W { spawn "~/.local/bin/hidebar" "toggle" "workspaces"; }
    Ctrl+Shift+A { spawn "~/.local/bin/hidebar" "show" "all"; }
    Ctrl+Shift+Alt+A { spawn "~/.local/bin/hidebar" "toggle" "all"; }
}
```

Restart the daemon after updating the binary with `pkill -x hidebar` and start it again from Niri.

## Development

```sh
# Linux / WSL
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo test --locked -- --ignored   # real Hyprland/Niri, battery, /proc, audio and network

# Windows 11 (MSVC)
cargo test --target x86_64-pc-windows-msvc
cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings
cargo build --release --target x86_64-pc-windows-msvc
```

Useful environment variables:

- `HIDEBAR_SOCKET`: a different IPC name, to run a development daemon next to your usual one.
  On Linux it is a Unix socket path (keep it short; ~100 characters max); on Windows it is a
  named-pipe name, not a filesystem path. Set it for the client too:

  ```sh
  export HIDEBAR_SOCKET=$XDG_RUNTIME_DIR/hidebar-dev.sock
  ./target/debug/hidebar daemon &
  ./target/debug/hidebar show all
  ```

- `HIDEBAR_BATTERY_PATH` (Linux): a directory with `capacity` (e.g. `8`) and `status`
  (e.g. `Discharging`) files, to simulate battery levels without waiting for it to drain.
- `HIDEBAR_FOREGROUND` (Windows): keep the console visible even when the daemon owns it
  (useful when started by the Run key or a shortcut).
- `HIDEBAR_NO_HOTKEYS` (Windows): disable all global hotkeys without recompiling.
- `WGPU_BACKEND` (Linux): graphics backend for wgpu. The daemon sets it to `vulkan` when it is
  unset; any explicit value (e.g. `gl`) is respected. Windows does not use wgpu.

Layout:

```text
src/main.rs                 CLI and platform dispatch
src/command.rs              command parsing and formatting
src/state.rs                platform-independent state machine and tests
src/ui/mod.rs               Surface/Message and shared view entry point
src/ui/view.rs              pills, notices and calendar drawing
src/calendar.rs             Spanish dates and month grid
src/event.rs                events emitted by the sources
src/platform/mod.rs         Linux/Windows selection
src/platform/linux/         layer-shell shell + Hyprland/Niri/proc/pactl sources
src/platform/windows/       Win32 layered shell + native sources
  render.rs                 iced tiny-skia -> Pixmap -> DIB -> UpdateLayeredWindow
  mod.rs                    message loop, timers, hotkeys, calendar input
  battery.rs system.rs      native metrics
  network.rs audio.rs       native sources
  fullscreen.rs             QUNS-based auto-hide
  ipc.rs hotkeys.rs         named pipe and global shortcuts
  startup.rs logging.rs     HKCU autostart and rotating log
assets/icons/               custom SVG icons, embedded in the binary
```

## License

[MIT](LICENSE).
