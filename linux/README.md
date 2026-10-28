# Snowhush for Linux (X11)

Soft snow drifts across your desktop, settles along the tops of your windows and slips off when
you drag them. Sweep the drift aside with your cursor.

This is the **unofficial Linux/X11 fork** of Snowhush. It uses the same simulation, snow layers and
geometry as the Windows app, so the snow behaves the same; only the windowing, drawing and tray code
is new. It draws one transparent, click-through overlay per monitor with Vulkan (or OpenGL) and runs
in the notification area. The Linux port is currently a **beta**.

The port and this document were developed with the help of an AI coding assistant (DeepSeek V4.1
Flash Free via OpenCode), under the direction and testing of the fork maintainer. See
[../AI_DISCLOSURE.md](../AI_DISCLOSURE.md).

## Requirements

- An **X11/Xorg** session. Wayland does not let an application place snow over other windows, so
  Snowhush refuses to start there; pick the "X11" or "Xorg" session in your login screen.
- A compositing window manager for transparent windows. Cinnamon, Xfce, KDE Plasma, MATE and GNOME
  on Xorg all work out of the box.
- A working Vulkan or OpenGL driver. On AMD/Intel the Mesa drivers in your distribution are enough.
- Rust 1.89 or newer to build (edition 2024). On Debian, Ubuntu and Linux Mint the build dependency
  is `libdbus-1-dev`; a C toolchain and `pkg-config` are normally already present.
- At runtime the binary needs `libX11`, `libXcursor`, `libXi`, `libX11-xcb` (which pulls in `libxcb`),
  `libxkbcommon`, `libxkbcommon-x11`, `libdbus-1`, and either the Vulkan loader/driver or
  EGL/OpenGL. A normal desktop installation provides all of these.

## Build and run

```sh
cd linux
./build.sh              # release binary + dist/snowhush-<version>-linux-<arch>.tar.gz and .sha256
./target/release/Snowhush
```

For a debug build with symbols: `cargo build && ./target/debug/Snowhush`.

## Install

Prebuilt Linux archives are on the
[releases page](https://github.com/Akyzariys/Snowhush_unofficial_Linux-X11_fork/releases). To build
and install from source:

```sh
cd linux
./install.sh            # ~/.local/bin/Snowhush, menu entry, icon
sudo ./install.sh       # /usr/local/bin, for all users
sudo PREFIX=/opt/snowhush ./install.sh
```

The installer also copies the MIT licence to `$PREFIX/share/licenses/snowhush/`. Uninstall with
`Snowhush --uninstall` (removes the settings and the login entry), then delete the binary, menu
entry, icon and licence if you wish.

## Controls

The tray icon has the same menu as the other platforms:

- **Snowfall** — Off, Light, Steady, Heavy, Variable.
- **Accumulate** — whether new snow piles up.
- **Melt Snow** — let drifts thaw slowly so they never take over the screen.
- **Snow on Windows** — settle snow on the tops of windows, not just the screen edge.
- **Clear Snow** — blow the snow away (melts it where it lies when system animations are off).
- **Open at Login** — writes `$XDG_CONFIG_HOME/autostart/snowhush.desktop`
  (`~/.config/autostart/snowhush.desktop` normally).
- **Quit**.

The overlays are click-through: clicks, drags and scrolls reach the windows below, and the overlay
never takes keyboard focus. Snow pauses while the screen is locked (through the desktop's
ScreenSaver D-Bus service) and is hidden while a full-screen window covers a display.

## Differences from macOS and Windows

- **Behind Windows** is not available: X11 has no reliable way to paint between other applications'
  windows.
- **Show in Screenshots** is not available: X11 offers no supported way to exclude one window from a
  screen capture.
- There is no welcome window, no update checker and no wallpaper colour sampling; the snow uses the
  same daylight tint as the other builds.
- One instance runs at a time. A second launch bows out and points at the tray icon.

## Known limits

- Full-screen hiding relies on the EWMH `_NET_WM_STATE_FULLSCREEN` hint. A game or video player
  that bypasses the window manager and does not set it can leave the overlay visible.
- Lock detection uses the Cinnamon, GNOME or freedesktop ScreenSaver D-Bus service. On a desktop
  that offers none of them, the snow cannot pause for the lock screen.
- Work areas come from EWMH. On a multi-monitor desktop whose panels span only part of the desktop,
  a panel can leave a small unused strip on another monitor.
- X11 has no equivalent of the Windows "Show in Screenshots" exclusion, so the snow appears in
  screen captures.

## Files

- Settings: `$XDG_CONFIG_HOME/snowhush/config` (`~/.config/snowhush/config`).
- Log: `$XDG_STATE_HOME/snowhush/snowhush.log` (`~/.local/state/snowhush/snowhush.log`).
- Login entry: `$XDG_CONFIG_HOME/autostart/snowhush.desktop`.

The log is rewritten on every launch and records the adapter, each display's work area, the windows
snow can settle on, and a frame-rate summary every 30 seconds. It is the first place to look if the
snow does not appear.

## How it works

The crate reuses `windows/src/{geometry,layer,simulation,version}.rs` directly with `#[path]`, so
the ageing, wind, piles, melting and cursor interaction are the same code the Windows build runs.
On Linux:

- `winit` opens one undecorated, override-redirect window per monitor, sized to the EWMH work area,
  and `wgpu` draws into it with premultiplied alpha.
- `x11rb` reads the EWMH window list, frame extents, window types and work area, and tracks the
  pointer; the same connection sets an empty Shape input region so the overlay passes input through.
- `ksni` publishes the StatusNotifierItem icon and menu on D-Bus.

The shaders in `src/*.wgsl` are WGSL ports of `windows/src/shaders.hlsl`.

## Troubleshooting

- **"Snowhush needs an X11 display"** — start it inside the graphical session, or check `DISPLAY`.
- **"Snowhush draws on the X11 desktop"** — you are in a Wayland session. Log out and pick Xorg.
- **No snow, but the tray icon is there** — check the log; usually the GPU adapter could not be
  opened. `vulkaninfo` (package `vulkan-tools`) shows whether Vulkan works.
- **The desktop looks black where snow should be** — the compositor is off. Enable compositing in
  the desktop's settings.
- **No tray icon** — the desktop needs a StatusNotifierItem host (Cinnamon, KDE, Xfce with the
  StatusNotifier plugin, GNOME with the AppIndicator extension).
- **Two copies** — only the first runs; quit the tray icon before starting a new build.

## License

MIT, like the rest of Snowhush. See `../LICENSE` and `../NOTICE`.
