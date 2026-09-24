#!/usr/bin/env bash
# Smoke test for the tray: several items at once, ordering, hiding, menus.
#
# Runs on a private session bus, no GNOME needed:
#
#     dbus-run-session -- scripts/smoke.sh
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin="$here/target/debug"
config="$(mktemp -d)/config.toml"
keys_py="$(mktemp)"
items=()
trap 'kill "${daemon:-}" ${items[@]+"${items[@]}"} 2>/dev/null || true; rm -f "$keys_py"' EXIT

# Item keys, in the order the daemon reports them. Nested records also start
# with "('…" (an icon has no name), so empty matches are dropped.
cat > "$keys_py" <<'PY'
import re, sys
print(" ".join(x for x in re.findall(r"\('([^']*)'", sys.stdin.read()) if x))
PY

call() {
    gdbus call --session --dest dev.taskbar.Daemon --object-path /dev/taskbar/Daemon \
        --method "dev.taskbar.Daemon.$@"
}

keys() {
    call ListItems 24 | python3 "$keys_py"
}

expect() {
    local want="$1" got
    got="$(keys)"
    if [ "$got" = "$want" ]; then
        echo "  ok    [$want]"
    else
        echo "  FAIL  expected [$want]"
        echo "        got      [$got]"
        exit 1
    fi
}

echo "== daemon and three items (two share an id)"
"$bin/taskbar-daemon" --config "$config" & daemon=$!
sleep 1
"$bin/taskbar-testitem" --id alpha --title "Alpha" & items+=($!)
sleep 1
"$bin/taskbar-testitem" --id beta --title "Beta" & items+=($!)
sleep 1
# Same id as the first: the daemon has to give this one a key of its own.
"$bin/taskbar-testitem" --id alpha --title "Alpha again" & items+=($!)
sleep 1

echo "== registration order, duplicate ids disambiguated"
expect "alpha beta alpha#2"

# Note: gdbus parses a negative argument as an option, so this moves alpha
# down rather than beta up. zbus clients (the settings app) use both signs.
echo "== MoveItem swaps neighbours (alpha down)"
call MoveItem "alpha" 1 > /dev/null
expect "beta alpha alpha#2"

echo "== hiding one item leaves the others alone"
call SetItemHidden "alpha#2" true > /dev/null
expect "beta alpha"
call SetItemHidden "alpha#2" false > /dev/null
expect "beta alpha alpha#2"

echo "== every item has its own menu"
for key in alpha beta alpha#2; do
    printf "  %-8s " "$key"
    call GetMenu "$key" | python3 -c "
import re, sys
labels = [n for n in re.findall(r\"\(\d+, \d+, '[^']*', '([^']*)'\", sys.stdin.read()) if n]
print(', '.join(labels))
"
done

echo "== status and configuration"
call GetStatus | python3 -c "
import re, sys
text = sys.stdin.read()
print('  ' + re.sub(r'\[((?:byte 0x[0-9a-f]{2}, ){8,})byte 0x[0-9a-f]{2}\]',
    lambda m: '[%d bytes]' % (len(m.group(1).split(',')) + 1), text).strip())
"
sed 's/^/  /' "$config"

echo "== pass"
