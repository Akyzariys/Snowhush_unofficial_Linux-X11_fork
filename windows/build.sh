#!/bin/zsh
# Builds the Windows app and its installer on macOS, into dist/Snowhush-Setup.exe or the folder given.
# The installer carries the app, so the app is built first and handed to it.
# Needs: rustup target add x86_64-pc-windows-msvc, cargo install cargo-xwin.
set -euo pipefail
cd "$(dirname "$0")"

out=${1:-../dist}
out=${out:A}
target=x86_64-pc-windows-msvc
release=$PWD/target/$target/release

cargo xwin build --release --target $target --bin Snowhush
SNOWHUSH_APP=$release/Snowhush.exe cargo xwin build --release --target $target --bin Snowhush-Setup --features setup
mkdir -p "$out"
cp "$release/Snowhush-Setup.exe" "$out/Snowhush-Setup.exe"
echo "$out/Snowhush-Setup.exe"
