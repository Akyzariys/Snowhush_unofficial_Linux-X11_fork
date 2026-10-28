<p align="center">
  <img src="site/assets/icon-512.png" width="128" height="128" alt="Snowhush icon">
</p>

> **Unofficial Linux/X11 fork.** This fork adds the Linux desktop port to
> [Snowhush](https://github.com/mbezruchko/Snowhush) by mbezruchko. The original macOS and Windows
> builds and download links remain upstream; the Linux build lives here. The upstream project is
> MIT-licensed; the original copyright notice is retained in [LICENSE](LICENSE), and the fork attribution is in [NOTICE](NOTICE).

<h1 align="center">Snowhush</h1>

<p align="center">It's snowing on your desktop.</p>

<p align="center">
  <a href="https://github.com/mbezruchko/Snowhush/releases/latest/download/Snowhush.dmg"><b>Download for macOS</b></a>
  &nbsp;·&nbsp;
  <a href="https://github.com/mbezruchko/Snowhush/releases/latest/download/Snowhush-Setup.exe"><b>Download for Windows</b></a>
  &nbsp;·&nbsp;
  <a href="linux/README.md"><b>Linux (X11)</b></a>
  &nbsp;·&nbsp;
  <a href="https://snowhush.pages.dev">snowhush.pages.dev</a>
  <br>
  <sub>macOS 14 or later · Windows 10 or 11 · Linux with X11 · free and open source</sub>
</p>

https://github.com/user-attachments/assets/291c3cd5-3563-4b86-8b66-dfd924f980fc

A tiny app that lives in the menu bar on macOS and in the tray on Windows and Linux. Soft snow drifts across your screen, settles along the tops of your windows and slips off when you drag them. Sweep the drift aside with your cursor.

## What the snow does

- **Falls.** Every flake keeps its own pace and sways in a slow breeze. Variable lets the snowfall rise and ease on its own over the day.
- **Settles.** It piles up and slumps to a natural slope along the bottom of the screen, on any window with room above it, and on the Dock.
- **Moves.** Push the cursor through a drift and it parts and sprays. Drag or shake a window and its snow slides off and lands below.
- **Drops.** Close or minimize a window and its snow drops at once, in lumps that burst into powder where they land.
- **Melts.** With Melt Snow on, the drift thaws slowly, so it never takes over your screen.
- **Hides.** On macOS and Windows, screenshots and screen sharing show your desktop without it, unless you turn on Show in Screenshots. X11 has no supported way to do that, so the Linux build cannot promise it.
- **Rests.** A few percent of one processor core and modest memory, and less on battery or in Low Power Mode. It pauses while the screen locks or a full-screen window covers it; the macOS and Windows builds also pause when the display sleeps.

Everything else lives in the menu bar: Snowfall (Off, Light, Steady, Heavy, Variable), Accumulate, Melt Snow, Snow on Windows, Clear Snow, Behind Windows, Show in Screenshots and Open at Login.

On Linux the snow lives in the notification area and draws in transparent, click-through overlays on X11. Behind Windows and Show in Screenshots are left out there; see [linux/README.md](linux/README.md) for the differences and requirements.

## Building

```sh
./build.sh            # build/Snowhush.app, signed ad hoc
open build/Snowhush.app
windows/build.sh      # dist/Snowhush-Setup.exe, cross-compiled from macOS
linux/build.sh        # dist/snowhush-<version>-linux-<arch>.tar.gz
linux/install.sh      # ~/.local/bin, menu entry, icon and licence
```

## AI assistance

The Linux/X11 port and its documentation were developed with the help of an AI coding assistant
(**DeepSeek V4.1 Flash Free** via **OpenCode**), under the direction and testing of the fork
maintainer. The upstream macOS/Windows sources are reused unchanged. AI systems can make mistakes;
review the code before relying on it in a sensitive environment. See [AI_DISCLOSURE.md](AI_DISCLOSURE.md)
for the full disclosure.

## License

[MIT](LICENSE). The original Snowhush copyright notice is retained; see [NOTICE](NOTICE).
