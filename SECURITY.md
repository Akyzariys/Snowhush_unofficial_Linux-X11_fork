# Security policy

This is an unofficial Linux/X11 fork. The upstream macOS/Windows project has its own security
process; report upstream-only issues to
[mbezruchko/Snowhush](https://github.com/mbezruchko/Snowhush).

## Reporting a vulnerability

Please **do not open a public issue** for a security problem in the Linux port. Use the repository's
**Security → Report a vulnerability** flow (GitHub private security advisory) and include:

- the Snowhush version and the exact commit;
- distribution, desktop environment and whether the session is X11;
- GPU/driver and monitor layout if relevant;
- reproduction steps and the expected/actual result;
- relevant log lines from `$XDG_STATE_HOME/snowhush/snowhush.log`, with window titles or other
  personal data removed.

The maintainer will acknowledge the report and coordinate a fix and disclosure. The Linux port is a
beta and does not promise a fixed response time.

## Scope examples

Relevant: input pass-through failures, focus stealing, overlay remaining visible over a lock screen
or full-screen window, symlink/permission bugs in settings or the instance lock, and X11 property
parsing that can crash or exhaust the process.

Not a vulnerability by itself: the documented Linux/X11 limitations (no screenshot exclusion,
EWMH-only full-screen detection, approximate multi-monitor work areas).
