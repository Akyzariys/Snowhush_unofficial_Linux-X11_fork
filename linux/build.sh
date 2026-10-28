#!/usr/bin/env bash
# Builds the Linux app and packs a runnable tarball into ../dist.
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(dirname -- "$here")
export PATH="$HOME/.cargo/bin:$PATH"

cd "$here"
cargo build --release --locked

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
machine=$(uname -m)
case "$machine" in
    x86_64|amd64) arch=x86_64 ;;
    aarch64|arm64) arch=aarch64 ;;
    armv7l|armv7) arch=armv7l ;;
    *) arch=$machine ;;
esac
name="snowhush-${version}-linux-${arch}"
stage="$here/target/package/$name"

rm -rf -- "$stage"
mkdir -p -- "$stage"
install -m755 target/release/Snowhush "$stage/Snowhush"
install -m644 assets/snowhush.desktop "$stage/snowhush.desktop"
install -m644 "$root/site/assets/icon-64.png" "$stage/snowhush.png"
install -m644 "$root/LICENSE" "$stage/LICENSE"
install -m644 "$root/NOTICE" "$stage/NOTICE"
install -m644 "$root/AI_DISCLOSURE.md" "$stage/AI_DISCLOSURE.md"
install -m644 "$root/RELEASE_NOTES.md" "$stage/RELEASE_NOTES.md"
install -m644 README.md "$stage/README.md"
# The repository README links one directory up; in the flat tarball the disclosure sits beside it.
sed -i -e 's#\.\./AI_DISCLOSURE\.md#AI_DISCLOSURE.md#g' -e 's#\.\./LICENSE#LICENSE#g' -e 's#\.\./NOTICE#NOTICE#g' "$stage/README.md"

mkdir -p -- "$root/dist"
tar -C "$here/target/package" -czf "$root/dist/$name.tar.gz" "$name"
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$root/dist" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
    echo "built dist/$name.tar.gz (checksum: dist/$name.tar.gz.sha256)"
else
    echo "built dist/$name.tar.gz"
fi
