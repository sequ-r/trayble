# Taskbar

A system tray for the GNOME panel. It shows the tray icons (StatusNotifierItem)
of running applications and services next to the clock, with their menus, in
GNOME Shell 48–51.

```
 GNOME Shell panel ──► taskbar@taskbar.dev (GJS, thin renderer)
                              │  D-Bus  dev.taskbar.Daemon
                              ▼
                      taskbar-daemon (Rust) ──► applications
                      org.kde.StatusNotifierWatcher
                      com.canonical.dbusmenu client
```

Everything that *decides* anything — which icon to show, how items are ordered,
what a menu entry means — lives in Rust. The part inside the compositor is
deliberately a renderer, because extensions run in GNOME Shell's process and
must not be able to take the desktop down with them.

## What works

- StatusNotifierItem icons (Steam, Telegram, Discord, OBS, Qt and
  libappindicator/libayatana applications)
- Per-item menus (`com.canonical.dbusmenu`), including submenus, checkmarks and
  radio entries
- Left click = activate, middle click = secondary activate, right click =
  menu, scroll = scroll, touch = menu
- Overlay and attention icons (blended in Rust, one image leaves the daemon)
- Hide, reorder and resize icons from a GTK4/libadwaita settings app
- Panel placement (left/centre/right area, slot, icon size)
- Icons survive application restarts: preferences are keyed by the
  application's own `Id`

## Layout

| Path | What | Language |
| --- | --- | --- |
| `crates/taskbar-core` | Pure domain logic: SNI/dbusmenu models, icon maths, the state reducer | Rust |
| `crates/taskbar-api` | D-Bus contract: wire types, proxies, variant decoding | Rust |
| `crates/taskbar-daemon` | `org.kde.StatusNotifierWatcher` + `dev.taskbar.Daemon` | Rust |
| `crates/taskbar-app` | Settings app (`taskbar-settings`) | Rust, GTK4, libadwaita |
| `crates/taskbar-testitem` | A fake tray application, for testing | Rust |
| `extension/taskbar@taskbar.dev` | Panel indicator and preferences shim | GJS |
| `data/` | D-Bus activation, systemd unit, desktop entry | — |

## Building

You need Rust (1.85+), `pkg-config`, GTK 4 and libadwaita development files
(`libgtk-4-dev`, `libadwaita-1-dev` on Debian/Ubuntu), and GNOME Shell 48 or
newer to use the extension.

```sh
cargo build --release
cargo test --workspace
```

## Installing

```sh
scripts/install.sh            # builds and installs into ~/.local
```

Then reload GNOME Shell (Alt+F2, `r` on X11; log out and back in on Wayland)
and enable the extension:

```sh
gnome-extensions enable taskbar@taskbar.dev
```

The daemon is D-Bus activated, so it starts when the extension first asks for
it. To have it running from login instead:

```sh
systemctl --user enable --now dev.taskbar.Daemon.service
```

`scripts/install.sh --uninstall` removes everything again.

## Trying it out

`taskbar-testitem` is a fake tray application with a changing icon, a status
that cycles and a real menu:

```sh
taskbar-testitem --animate 3 --flip-status 11
```

Without GNOME, the whole pipeline can be exercised on a private session bus:

```sh
dbus-run-session -- scripts/smoke.sh
```

That starts the daemon and a test item, then asks the same questions the panel
and the settings app ask.

## Configuration

`~/.config/taskbar/config.toml`, written by the daemon whenever the settings
app changes something:

```toml
icon_size = 16          # logical pixels on the panel
sort = "registered"     # "registered", "manual" or "title"
order = []              # manual order of item keys
hidden = []             # keys hidden from the panel
show_when_empty = false
panel_box = "right"     # "left", "center" or "right"
panel_position = 0
```

Item keys are the `Id` the application reports (e.g. `steam`), so preferences
survive restarts of the application.

## Notes

- **Legacy XEmbed tray icons are not supported yet** — only StatusNotifierItem.
  See `docs/DESIGN.md` for the plan.
- The panel code is `St`/`Clutter`, not GTK4: GNOME Shell's top bar cannot
  host GTK widgets and extensions must be GJS. GTK4 and libadwaita are used
  where they belong, in the settings app.

## License

GPL-3.0-or-later. The extension runs inside GNOME Shell, which is GPL-3.
