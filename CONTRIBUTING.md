# Contributing to the Linux/X11 fork

This fork is an unofficial Linux port of [Snowhush](https://github.com/mbezruchko/Snowhush). The
macOS and Windows code is upstream; fixes that are not Linux-specific should go to the upstream
project instead.

## Build and test

The Linux crate is in `linux/` and requires Rust 1.89 or newer.

```sh
cd linux
cargo test --locked
cargo clippy --locked --all-targets
./build.sh
```

Runtime dependencies are listed in [`linux/README.md`](linux/README.md). Development and testing
need an **X11/Xorg** session; Wayland is not supported.

## Keep the upstream sources untouched

`linux/src/lib.rs` includes `windows/src/{geometry,layer,simulation,version}.rs` through `#[path]`.
Do not rewrite those files for Linux. In particular, do not run a global `cargo fmt`: from `linux/`
it also formats the shared `windows/src/` files. Format only the Linux sources:

```sh
rustfmt --edition 2024 linux/src/*.rs
```

## Pull requests

- Keep changes focused and explain the user-visible behavior.
- Add or update tests for parsing, geometry, settings or security-sensitive changes.
- For X11 changes, state the desktop environment, compositor, GPU, display layout and whether the
  session is X11.
- Do not commit build output. `linux/target/` and `dist/` are ignored.
- By contributing, you agree that your changes are distributed under the MIT license.

## AI assistance

AI-assisted contributions are welcome when disclosed. See [`AI_DISCLOSURE.md`](AI_DISCLOSURE.md).
The maintainer remains responsible for reviewing and testing the result.
