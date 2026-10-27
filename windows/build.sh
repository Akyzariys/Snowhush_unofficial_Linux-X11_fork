#!/bin/zsh
# Builds the Windows app and its installer on macOS, into dist/Snowhush-Setup.exe or the folder given.
# Needs: rustup target add x86_64-pc-windows-msvc, cargo install cargo-xwin, brew install makensis.
set -euo pipefail
cd "$(dirname "$0")"

out=${1:-../dist}
out=${out:A}
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
exe=target/x86_64-pc-windows-msvc/release/Snowhush.exe

cargo xwin build --release --target x86_64-pc-windows-msvc
mkdir -p "$out"
size=$(( ($(stat -f%z "$exe") + 1023) / 1024 ))
makensis -V2 -DVERSION="$version" -DEXE="$exe" -DSIZE="$size" -DOUTFILE="$out/Snowhush-Setup.exe" installer.nsi
echo "$out/Snowhush-Setup.exe"
