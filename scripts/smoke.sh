#!/usr/bin/env bash
# Smoke test: run the daemon and a fake tray item on a private session bus and
# check what the panel indicator and the settings app would see.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin="$here/target/debug"
config="$(mktemp -d)/config.toml"

summarize() {
    python3 -c '
import re, sys
text = sys.stdin.read()
text = re.sub(r"\[(\d+)(, ){3}\.\.\.\]", r"[\1 bytes...]", text)
text = re.sub(r"\[((?:\d+, ){16,}\d+)\]", lambda m: f"[{len(m.group(1).split(chr(44)))} bytes]", text)
print(text)
'
}

"$bin/taskbar-daemon" --config "$config" &
daemon=$!
sleep 1

"$bin/taskbar-testitem" --animate 3 &
item=$!
sleep 1

call() {
    gdbus call --session --dest dev.taskbar.Daemon --object-path /dev/taskbar/Daemon \
        --method "dev.taskbar.Daemon.$@"
}

echo "== GetStatus"
call GetStatus | summarize

echo "== ListItems 24"
call ListItems 24 | summarize

echo "== GetMenu taskbar-testitem"
call GetMenu "taskbar-testitem" | summarize

echo "== watcher registry"
gdbus call --session --dest org.kde.StatusNotifierWatcher --object-path /StatusNotifierWatcher \
    --method org.freedesktop.DBus.Properties.Get \
    org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems | summarize

echo "== SetItemHidden taskbar-testitem true, then ListItems"
call SetItemHidden "taskbar-testitem" true | summarize
call ListItems 24 | summarize

echo "== SetItemHidden taskbar-testitem false, then GetConfig"
call SetItemHidden "taskbar-testitem" false | summarize
call GetConfig | summarize

echo "== config file"
cat "$config"

kill "$item" "$daemon" 2>/dev/null || true
wait 2>/dev/null || true
echo "== done"
