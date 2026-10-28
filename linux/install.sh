#!/usr/bin/env bash
# Installs Snowhush for the current user: binary, menu entry and icon.
# PREFIX can point somewhere else, e.g. sudo PREFIX=/usr/local ./install.sh
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(dirname -- "$here")

if [ -n "${PREFIX:-}" ]; then
    prefix=$PREFIX
elif [ "$(id -u)" -eq 0 ]; then
    # sudo normally resets HOME to /root; installing to /root/.local would hide the app from the
    # person who asked for it. Root with no PREFIX means a system-wide install.
    prefix=/usr/local
else
    prefix=$HOME/.local
fi
bin="$prefix/bin"
apps="$prefix/share/applications"
icon_dir="$prefix/share/icons/hicolor/64x64/apps"
license_dir="$prefix/share/licenses/snowhush"
binary="$here/target/release/Snowhush"

if [ ! -x "$binary" ]; then
    if [ "$(id -u)" -eq 0 ]; then
        echo "snowhush: build first as your user (linux/build.sh), then run this installer as root." >&2
        exit 1
    fi
    "$here/build.sh"
fi

# Quote an executable path for the desktop entry's Exec key: double-quote it, escape characters the
# desktop-entry grammar treats specially, and turn a literal % into the %% escape (a bare %f would be
# expanded as a field code and the entry would launch the wrong command).
desktop_quote() {
    local value=$1
    if [[ $value == *$'\n'* || $value == *$'\r'* ]]; then
        echo "snowhush: the install path contains a newline; choose a different PREFIX." >&2
        return 1
    fi
    value=${value//\\/\\\\}
    value=${value//\"/\\\"}
    value=${value//\`/\\\`}
    value=${value//\$/\\\$}
    value=${value//%/%%}
    printf '"%s"' "$value"
}

exec_value=$(desktop_quote "$bin/Snowhush")

install -Dm755 "$binary" "$bin/Snowhush"
install -Dm644 "$root/site/assets/icon-64.png" "$icon_dir/snowhush.png"
install -Dm644 "$root/LICENSE" "$license_dir/LICENSE"
install -Dm644 "$root/NOTICE" "$license_dir/NOTICE"
mkdir -p -- "$apps"
{
    printf '%s\n' '[Desktop Entry]'
    printf '%s\n' 'Type=Application'
    printf '%s\n' 'Name=Snowhush'
    printf '%s\n' 'GenericName=Snow'
    printf '%s\n' 'Comment=Soft snow on your desktop'
    printf 'Exec=%s\n' "$exec_value"
    printf '%s\n' 'Icon=snowhush'
    printf '%s\n' 'Terminal=false'
    printf '%s\n' 'Categories=Utility;'
    printf '%s\n' 'Keywords=snow;winter;desktop;'
    printf '%s\n' 'StartupNotify=false'
} > "$apps/snowhush.desktop"
chmod 644 -- "$apps/snowhush.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$apps" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$prefix/share/icons/hicolor" >/dev/null 2>&1 || true
fi

echo "installed $bin/Snowhush"
echo "Snowhush is in the menu; it also lives in the notification area while it runs."
case ":$PATH:" in
    *":$bin:"*) ;;
    *) echo "Note: $bin is not in PATH, but the menu entry uses the full path." ;;
esac
