# AI assistance disclosure

This repository is an unofficial Linux/X11 fork of
[Snowhush](https://github.com/mbezruchko/Snowhush) by mbezruchko. The upstream project and its
macOS/Windows code are MIT-licensed; the original copyright notice is retained in `LICENSE`.

## What was assisted

The Linux/X11 port, its build/install scripts, tests and documentation were written with the help of
an AI coding assistant:

- Tool: **OpenCode** coding agent
- Model: **DeepSeek V4.1 Flash Free**
- The work was directed, run and tested by the fork maintainer.
- The shared simulation/geometry sources under `windows/src/` are reused from upstream and were not
  rewritten for this fork.

The assistant also reviewed the Linux code for error handling, input pass-through, lock-screen
behavior, single-instance locking, and unsafe parsing of X11 properties. The maintainer is
responsible for the final code, releases and support.

## Caveats

AI systems can make mistakes. The Linux build is a beta; it has been tested on Linux Mint with X11
and one AMD GPU, but not across every compositor, GPU, monitor layout or desktop environment.
Review the code and test it in your environment before using it where correctness or security is
critical.

The fork remains distributed under the MIT license. See `LICENSE`.
