# Design

Why Taskbar is shaped the way it is.

## The constraint that shapes everything

Code that touches the GNOME *panel* runs inside the `gnome-shell` compositor
process. That surface is `St` (a Clutter toolkit styled with CSS) — GTK4 and
libadwaita cannot be used there, and GNOME's extension review guidelines
require extensions to be written in GJS. A bug in that code does not crash an
app, it takes the session with it.

So "Rust and GTK4/libadwaita where possible" splits along that line, and the
split happens to be the right one anyway:

| Component | Technology | Why |
| --- | --- | --- |
| Tray daemon | Rust, `zbus`, `tokio` | All the protocol work and all decisions live here |
| Panel indicator | GJS, `St`/`Clutter` | The only language and toolkit the compositor allows |
| Settings app | Rust, GTK4, libadwaita | Out of process, exactly what libadwaita is for |
| Preferences shim | GJS + GTK4 + libadwaita | So `gnome-extensions prefs` still works, linking to the app |

The extension is a renderer: it paints icons the daemon already resolved and
forwards input back. It has no knowledge of StatusNotifierItem, pixmaps or
menu protocols.

### Platform notes worth remembering

- **St lives in gnome-shell, not in mutter.** Its sources are `src/st/*.h` in
  the *gnome-shell* tree (mutter 51 has no `src/st` at all), and its APIs
  differ from the GTK functions of the same name: `St.IconTheme.lookup_icon()`
  takes `(name, size, flags)` where GTK's takes
  `(name, fallbacks, size, scale, direction, flags)`. Verify St calls against
  gnome-shell's headers — getting this wrong throws at best and aborts the
  compositor at worst, since a wrong argument into St is a wrong argument
  into C.

## Functional core, imperative shell

`taskbar-core` is pure: no I/O, no D-Bus, no GUI types, no interior
mutability. Its heart is one fold:

```rust
pub fn update(state: State, event: Event) -> (State, Vec<Effect>)
```

Everything that changes the tray — registration, an application vanishing, new
properties, a menu patch, a configuration change — is an [`Event`]. The fold
returns the next state and the [`Effect`]s to carry out: announce the change,
persist the configuration. The tests cover it without a session bus or a
display.

The daemon is then a thin shell around that fold:

1. D-Bus messages from applications become [`Event`]s (this is the only place
   `org.kde.StatusNotifierItem` and `com.canonical.dbusmenu` are spoken);
2. `Store::apply` folds the event through `update`;
3. the returned [`Effect`]s are carried out (emit a signal, write the config).

The settings app is the same shape in miniature: `model::update(&State, Msg)
-> (State, Vec<Call>)` is pure and unit tested; GTK widgets produce [`Msg`]s
and a worker thread carries out [`Call`]s.

## Data over the wire

`ListItems` returns a complete, self-contained picture of the tray — every
visible item, in order, with its icon already resolved. Signals carry no data;
they say "ask again".

Why: clients never assemble state from deltas, so they cannot end up with a
stale or half-applied view, and the JavaScript side stays trivial. The cost is
one round trip per change, which is nothing at tray-icon rates.

### Icons travel as images, not pixels

An item's icon leaves the daemon as a PNG (`icon::to_png`, a pure function
with a round-trip test). The panel shows it through `Gio.BytesIcon` — the same
`GLoadableIcon` path `Gio.FileIcon` takes — and the settings app decodes it
with `gdk::Texture::from_bytes`.

An earlier version forwarded the application's raw ARGB32 buffers straight
into `St.ImageContent.set_bytes(...)`: it saved an encode and kept the pixels
byte-for-byte as the application sent them. But it puts a pixel format, a row
stride and (on GNOME 48+) a Cogl context across the border into the
compositor, where a mistake in any of them does not break one icon — it takes
the whole session down. That is the wrong trade for a tray icon.

A PNG is self-describing: gdk-pixbuf decodes it in managed code, a malformed
image degrades to the fallback icon, and neither toolkit ever agrees with the
other about pixel formats. The encode is trivially cheap at these sizes (a
44×44 icon is a couple of hundred bytes) and it happens in Rust, where it is
unit tested.

Decisions made *before* the pixels travel, in Rust, with tests:

- which pixmap of the application's list fills the requested size
  (`pick_best`: the smallest one that is at least `target`, so no needless
  upscaling);
- which icon applies at all: attention icons take over while
  `Status = "NeedsAttention"` (`ItemProps::effective_icon`);
- overlay icons are alpha-blended onto the base icon (`icon::composite`), so
  the two toolkits never have to agree on compositing;
- theme names are only a fallback for applications that name their icon but
  send no pixels.

Menus are flattened to a pre-order list of nodes with parent ids rather than a
recursive tree: `a(sss…)` keeps the D-Bus signature simple for the JavaScript
side, and `view::inflate_menu` builds the tree back for anyone who wants it.

## Protocol notes and quirks

Things `taskbar-daemon` handles because real applications need them:

- **Three registration spellings.** `RegisterStatusNotifierItem` accepts
  `"/Some/Path"`, `"bus.name/Some/Path"` or `"bus.name"`; the path-only form
  resolves against the caller's connection (`ServiceRef::resolve`).
- **Registry strings.** `RegisteredStatusNotifierItems` and the registration
  signals use `bus.name/Path`, which is not the same as `bus.name` + `/` +
  `Path` (that gives a double slash). `ServiceRef::registration_id` is the one
  place that formats it.
- **Applications that unexport but stay on the bus.** Some (Electron among
  them) export their item and later remove the object without closing their
  bus name. A probe every 30 seconds reads the properties; two failed probes
  in a row drop the item.
- **Applications that re-register.** Re-registration replaces the previous
  incarnation of the same service instead of showing two icons.
- **`/NO_DBUSMENU`.** libappindicator uses this marker to say "no menu"; it
  must not be looked up.
- **Ayatana extensions.** `XAyatanaLabel` and `XAyatanaLabelGuide` are read
  and used as a fallback title.
- **`ItemIsMenu`.** When set, a left click opens the menu instead of
  activating the item.
- **Watcher conflicts.** Only one `org.kde.StatusNotifierWatcher` can exist.
  When Plasma or another tray owns it, Taskbar keeps running, reports the
  owner in the settings app's Status page, and does not fight over the name.

## Interaction model

Following the StatusNotifierItem semantics and the GNOME HIG:

| Input | Action |
| --- | --- |
| Left click | `Activate` (or the menu when `ItemIsMenu` is set) |
| Middle click | `SecondaryActivate` |
| Right click | The item's menu, or `ContextMenu` when it has none |
| Scroll | `Scroll` |
| Touch | Open the menu |

Menu events are reported to the application (`opened`, `closed`, `clicked`),
as the menu protocol requires.

## Settings app

GTK4 + libadwaita, following the HIG: `Adw.PreferencesPage` with grouped
`Adw.SpinRow`, `Adw.ComboRow` and `Adw.SwitchRow` controls, `Adw.ActionRow`
per tray item with a switch for visibility, toasts for errors, an
`Adw.AboutDialog`. Rows are built once and updated in place — a switch never
gets rebuilt under the pointer — while the item list is rebuilt only when the
set of keys changes.

Preferences are *not* GSettings: the daemon owns `config.toml`, and the app,
the panel and any future client read and write it through the daemon's API.
One writer, one source of truth, and the file is written atomically.

Reordering uses explicit up/down buttons rather than drag and drop: they are
discoverable and keyboard accessible. Drag reordering is a candidate for
later.

## Testing

- **Pure core** — unit tests over icons, menus, rules and the reducer,
  including property round trips and the failure cases (broken pixmaps,
  colliding ids, duplicate config keys).
- **Wire format** — decoding of real D-Bus variant shapes (`a(iiay)` pixmaps,
  `(sa(iiay)ss)` tooltips, `aas` shortcuts, recursive menus) is tested against
  values built the way they arrive on the bus.
- **Pipeline** — `scripts/smoke.sh` runs the daemon and `taskbar-testitem` on
  a private session bus (`dbus-run-session`) and drives the same API the panel
  and settings app use. It runs anywhere, no GNOME required.
- **Settings app** — the reducer is pure and tested; the widgets are thin.

## Roadmap

- Legacy XEmbed tray icons (XWayland window embedding, or an XEmbed→SNI proxy
  like KDE's `xembedsniproxy`) — the daemon is the natural home for a proxy.
- Hover tooltips with the item title.
- Drag and drop reordering in the settings app.
- Per-application rules (force a different icon size, group icons).
- Label support in the panel for `XAyatanaLabel`.
