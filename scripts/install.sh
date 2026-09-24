#!/usr/bin/env bash
# Build and install everything: daemon, settings app, test item, GNOME Shell
# extension and the D-Bus activation files.
#
#   scripts/install.sh [--prefix DIR] [--release|--debug] [--uninstall]
#
# The default prefix is ~/.local, which needs no root and is where the shell
# and the session bus look for per-user files anyway.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
prefix="${HOME}/.local"
profile=release
uninstall=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --prefix) prefix="${2:?--prefix needs a directory}"; shift 2 ;;
        --release) profile=release; shift ;;
        --debug) profile=debug; shift ;;
        --uninstall) uninstall=1; shift ;;
        -h|--help)
            sed -n '2,8p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
            exit 0 ;;
        *) echo "install.sh: unknown argument '$1' (try --help)" >&2; exit 2 ;;
    esac
done

bindir="$prefix/bin"
datadir="$prefix/share"
extension_dir="$datadir/gnome-shell/extensions/taskbar@taskbar.dev"

bins=(taskbar-daemon taskbar-settings taskbar-testitem)

if [[ $uninstall == 1 ]]; then
    for bin in "${bins[@]}"; do rm -f "$bindir/$bin"; done
    rm -rf "$extension_dir"
    rm -f "$datadir/dbus-1/services/dev.taskbar.Daemon.service"
    rm -f "$datadir/systemd/user/dev.taskbar.Daemon.service"
    rm -f "$datadir/applications/dev.taskbar.Settings.desktop"
    echo "removed taskbar from $prefix"
    echo "reload the shell (Alt+F2, 'r' on X11; log out on Wayland) and run:"
    echo "  systemctl --user daemon-reload"
    exit 0
fi

echo "building ($profile)..."
if [[ $profile == release ]]; then
    cargo build --release --manifest-path "$here/Cargo.toml"
else
    cargo build --manifest-path "$here/Cargo.toml"
fi

echo "installing to $prefix..."
install -d "$bindir" "$extension_dir" \
    "$datadir/dbus-1/services" "$datadir/systemd/user" "$datadir/applications"

for bin in "${bins[@]}"; do
    install -m 755 "$here/target/$profile/$bin" "$bindir/$bin"
done

# Only the extension itself; the tests stay out of the install.
install -m 644 "$here"/extension/taskbar@taskbar.dev/*.js \
    "$here"/extension/taskbar@taskbar.dev/*.json \
    "$here"/extension/taskbar@taskbar.dev/*.css "$extension_dir/"

# `data/` mirrors the layout under share/, so nothing has to be computed.
for unit in "$here"/data/dbus-1/services/* "$here"/data/systemd/user/*; do
    destination="$datadir/${unit#"$here"/data/}"
    install -d "$(dirname "$destination")"
    sed "s|@BINDIR@|$bindir|g" "$unit" > "$destination"
done
install -m 644 "$here/data/applications/dev.taskbar.Settings.desktop" \
    "$datadir/applications/"

command -v systemctl >/dev/null && systemctl --user daemon-reload || true

echo
echo "installed:"
echo "  daemon, settings app and test item in $bindir"
echo "  GNOME Shell extension in $extension_dir"
echo "  D-Bus activation in $datadir/dbus-1/services"
echo
echo "next steps:"
echo "  1. make sure $bindir is on your PATH"
echo "  2. reload GNOME Shell (Alt+F2 then 'r' on X11, or log out on Wayland)"
echo "  3. gnome-extensions enable taskbar@taskbar.dev"
echo "  4. optionally start the tray at login:"
echo "       systemctl --user enable --now dev.taskbar.Daemon.service"
echo
echo "try it without any real tray application:"
echo "  taskbar-testitem --animate 3 --flip-status 11"
