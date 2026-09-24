//! The window: GTK4 and libadwaita widgets driven by [`State`].
//!
//! The controls are built once and [`apply`] brings them in line with the
//! state, so moving a switch never rebuilds the row under the pointer. Only
//! the item list is rebuilt, and only when the list of keys actually changes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use taskbar_core::view::{IconView, ItemView};

use crate::model::{Msg, State};

/// What the window is made of, kept so [`apply`] can update it.
pub struct Widgets {
    pub window: adw::ApplicationWindow,
    pub toast: adw::ToastOverlay,
    pub icon_size: adw::SpinRow,
    pub sort: adw::ComboRow,
    pub show_when_empty: adw::SwitchRow,
    pub panel_box: adw::ComboRow,
    pub panel_position: adw::SpinRow,
    pub items_group: adw::PreferencesGroup,
    pub status_rows: Vec<(adw::ActionRow, gtk::Label)>,
    /// Messages go to the state machine, never straight to the daemon.
    send: async_channel::Sender<Msg>,
    /// Set while [`apply`] writes to the widgets, so handlers stay quiet.
    applying: Rc<Cell<bool>>,
    /// The rows on screen and the keys they belong to.
    shown: RefCell<Vec<ItemRow>>,
    keys: RefCell<Vec<String>>,
}

struct ItemRow {
    widget: adw::ActionRow,
    up: gtk::Button,
    down: gtk::Button,
    visible: gtk::Switch,
}

const SORT_MODES: [&str; 3] = ["registered", "manual", "title"];
const SORT_LABELS: [&str; 3] = ["Registration order", "Manual order", "By title"];
const PANEL_BOXES: [&str; 3] = ["left", "center", "right"];
const PANEL_LABELS: [&str; 3] = ["Left", "Center", "Right"];
const ITEM_ICON_SIZE: i32 = 24;

/// Build the window. `send` delivers user intents to the state machine.
pub fn build(application: &adw::Application, send: async_channel::Sender<Msg>) -> Widgets {
    let window = adw::ApplicationWindow::builder()
        .application(application)
        .default_width(640)
        .default_height(640)
        .title("Taskbar Settings")
        .build();

    // -- header -----------------------------------------------------------

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        "Taskbar",
        "System tray for the GNOME panel",
    )));

    let menu = gio::Menu::new();
    menu.append(Some("Refresh"), Some("win.refresh"));
    menu.append(Some("_About Taskbar"), Some("win.about"));
    let menu_button = gtk::MenuButton::new();
    menu_button.set_icon_name("open-menu-symbolic");
    menu_button.set_menu_model(Some(&menu));
    header.pack_end(&menu_button);

    // -- settings ---------------------------------------------------------

    let icon_size = adw::SpinRow::with_range(8.0, 48.0, 1.0);
    icon_size.set_title("Icon size");
    icon_size.set_subtitle("Height of a tray icon on the panel, in pixels");

    let sort = adw::ComboRow::new();
    sort.set_title("Order");
    sort.set_subtitle("How tray icons are arranged in the panel");
    sort.set_model(Some(&gtk::StringList::new(&SORT_LABELS)));

    let show_when_empty = adw::SwitchRow::new();
    show_when_empty.set_title("Show when empty");
    show_when_empty
        .set_subtitle("Keep the tray in the panel while no item is registered");

    let panel_box = adw::ComboRow::new();
    panel_box.set_title("Panel area");
    panel_box.set_subtitle("Where the tray sits in the top bar");
    panel_box.set_model(Some(&gtk::StringList::new(&PANEL_LABELS)));

    let panel_position = adw::SpinRow::with_range(0.0, 20.0, 1.0);
    panel_position.set_title("Position");
    panel_position.set_subtitle("Slot inside that area, counting from its left edge");

    let behaviour = adw::PreferencesGroup::new();
    behaviour.set_title("Behaviour");
    behaviour.add(&icon_size);
    behaviour.add(&sort);
    behaviour.add(&show_when_empty);

    let placement = adw::PreferencesGroup::new();
    placement.set_title("Placement");
    placement.add(&panel_box);
    placement.add(&panel_position);

    let items_group = adw::PreferencesGroup::new();
    items_group.set_title("Tray items");
    items_group.set_description(Some(
        "Turn an item off to hide it from the panel. The arrow buttons move \
         it within the tray.",
    ));

    let status_rows = status_group();
    let status_group = adw::PreferencesGroup::new();
    status_group.set_title("Status");
    for (row, _) in &status_rows {
        status_group.add(row);
    }

    // -- page -------------------------------------------------------------

    let page = adw::PreferencesPage::new();
    page.add(&behaviour);
    page.add(&placement);
    page.add(&items_group);
    page.add(&status_group);

    let scrolled = gtk::ScrolledWindow::builder()
        .child(&page)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&scrolled));

    let toast = adw::ToastOverlay::new();
    toast.set_child(Some(&toolbar));
    window.set_content(Some(&toast));

    let widgets = Widgets {
        window,
        toast,
        icon_size,
        sort,
        show_when_empty,
        panel_box,
        panel_position,
        items_group,
        status_rows,
        send,
        applying: Rc::new(Cell::new(false)),
        shown: RefCell::new(Vec::new()),
        keys: RefCell::new(Vec::new()),
    };

    connect(&widgets);
    widgets
}

fn status_group() -> Vec<(adw::ActionRow, gtk::Label)> {
    [
        "Daemon",
        "Tray watcher",
        "Registered items",
        "Configuration",
        "Last error",
    ]
    .into_iter()
    .map(|title| {
        let label = gtk::Label::new(None);
        label.add_css_class("dim-label");
        label.set_halign(gtk::Align::End);
        label.set_valign(gtk::Align::Center);
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_max_width_chars(32);

        let row = adw::ActionRow::new();
        row.set_title(title);
        row.add_suffix(&label);
        (row, label)
    })
    .collect()
}

/// Attach handlers that turn widget changes into messages.
fn connect(widgets: &Widgets) {
    let guard = widgets.applying.clone();
    let send = widgets.send.clone();
    widgets.icon_size.connect_value_notify(move |row| {
        if !guard.get() {
            let _ = send.send_blocking(Msg::SetIconSize(row.value() as i32));
        }
    });

    let guard = widgets.applying.clone();
    let send = widgets.send.clone();
    widgets.sort.connect_selected_notify(move |row| {
        if guard.get() {
            return;
        }
        let mode = SORT_MODES
            .get(row.selected() as usize)
            .copied()
            .unwrap_or(SORT_MODES[0]);
        let _ = send.send_blocking(Msg::SetSort(mode.to_owned()));
    });

    let guard = widgets.applying.clone();
    let send = widgets.send.clone();
    widgets.show_when_empty.connect_active_notify(move |row| {
        if !guard.get() {
            let _ = send.send_blocking(Msg::SetShowWhenEmpty(row.is_active()));
        }
    });

    let guard = widgets.applying.clone();
    let send = widgets.send.clone();
    widgets.panel_box.connect_selected_notify(move |row| {
        if guard.get() {
            return;
        }
        let panel_box = PANEL_BOXES
            .get(row.selected() as usize)
            .copied()
            .unwrap_or(PANEL_BOXES[2]);
        let _ = send.send_blocking(Msg::SetPanelBox(panel_box.to_owned()));
    });

    let guard = widgets.applying.clone();
    let send = widgets.send.clone();
    widgets.panel_position.connect_value_notify(move |row| {
        if !guard.get() {
            let _ = send.send_blocking(Msg::SetPanelPosition(row.value() as i32));
        }
    });
}

/// Bring the widgets in line with the state.
pub fn apply(state: &State, widgets: &Widgets) {
    widgets.applying.set(true);
    widgets.icon_size.set_value(state.config.icon_size as f64);
    widgets
        .sort
        .set_selected(index_of(SORT_MODES, &state.config.sort) as u32);
    widgets
        .show_when_empty
        .set_active(state.config.show_when_empty);
    widgets
        .panel_box
        .set_selected(index_of(PANEL_BOXES, &state.config.panel_box) as u32);
    widgets
        .panel_position
        .set_value(state.config.panel_position as f64);
    widgets.applying.set(false);

    apply_items(state, widgets);
    apply_status(state, widgets);
}

fn apply_items(state: &State, widgets: &Widgets) {
    let keys: Vec<String> = state.items.iter().map(|item| item.key.clone()).collect();

    if *widgets.keys.borrow() == keys {
        // Same items: only their state can have moved, so the rows — and the
        // switch the user may be holding — stay where they are.
        widgets.applying.set(true);
        for (item, row) in state.items.iter().zip(widgets.shown.borrow().iter()) {
            row.visible.set_active(!state.is_hidden(&item.key));
            let index = state.position(&item.key).unwrap_or_default();
            row.up.set_sensitive(index > 0);
            row.down.set_sensitive(index + 1 < state.items.len());
        }
        widgets.applying.set(false);
        return;
    }

    for row in widgets.shown.borrow_mut().drain(..) {
        widgets.items_group.remove(&row.widget);
    }

    if state.items.is_empty() {
        widgets.items_group.set_description(Some(
            "No tray items are registered. Start an application with a tray \
             icon — `taskbar-testitem` works — and it will show up here.",
        ));
    } else {
        widgets.items_group.set_description(Some(
            "Turn an item off to hide it from the panel. The arrow buttons \
             move it within the tray.",
        ));
    }

    let mut shown = Vec::with_capacity(state.items.len());
    for (index, item) in state.items.iter().enumerate() {
        let row = build_item_row(item, state, index, widgets);
        widgets.items_group.add(&row.widget);
        shown.push(row);
    }
    *widgets.shown.borrow_mut() = shown;
    *widgets.keys.borrow_mut() = keys;
}

fn build_item_row(item: &ItemView, state: &State, index: usize, widgets: &Widgets) -> ItemRow {
    let row = adw::ActionRow::new();
    row.set_title(if item.title.is_empty() {
        &item.app_id
    } else {
        &item.title
    });
    row.set_subtitle(&subtitle_of(item));
    row.add_prefix(&icon_image(&item.icon, ITEM_ICON_SIZE));

    // Visibility switch: the last control, where a switch belongs.
    let visible = gtk::Switch::new();
    visible.set_valign(gtk::Align::Center);
    visible.set_tooltip_text(Some("Show in the panel"));
    visible.set_active(!state.is_hidden(&item.key));
    row.add_suffix(&visible);
    row.set_activatable_widget(Some(&visible));

    // Reordering.
    let up = gtk::Button::from_icon_name("go-up-symbolic");
    up.add_css_class("flat");
    up.set_valign(gtk::Align::Center);
    up.set_tooltip_text(Some("Move left"));
    up.set_sensitive(index > 0);

    let down = gtk::Button::from_icon_name("go-down-symbolic");
    down.add_css_class("flat");
    down.set_valign(gtk::Align::Center);
    down.set_tooltip_text(Some("Move right"));
    down.set_sensitive(index + 1 < state.items.len());

    row.add_suffix(&up);
    row.add_suffix(&down);

    // Values are set before the handlers are attached, so nothing echoes.
    let key = item.key.clone();
    up.connect_clicked({
        let send = widgets.send.clone();
        let key = key.clone();
        move |_| {
            let _ = send.send_blocking(Msg::MoveItem(key.clone(), -1));
        }
    });
    down.connect_clicked({
        let send = widgets.send.clone();
        let key = key.clone();
        move |_| {
            let _ = send.send_blocking(Msg::MoveItem(key.clone(), 1));
        }
    });
    visible.connect_active_notify({
        let send = widgets.send.clone();
        let guard = widgets.applying.clone();
        let key = key.clone();
        move |button| {
            if !guard.get() {
                let _ =
                    send.send_blocking(Msg::SetItemHidden(key.clone(), !button.is_active()));
            }
        }
    });

    ItemRow {
        widget: row,
        up,
        down,
        visible,
    }
}

fn subtitle_of(item: &ItemView) -> String {
    let mut parts = vec![];
    if !item.app_id.is_empty() {
        parts.push(item.app_id.clone());
    }
    match item.status.as_str() {
        "NeedsAttention" => parts.push("needs attention".into()),
        "Passive" => parts.push("hidden by the application".into()),
        _ => {}
    }
    parts.join("  ·  ")
}

fn apply_status(state: &State, widgets: &Widgets) {
    let status = state.status.as_ref();
    let values = [
        status
            .map(|status| format!("{}  ·  {} item(s)", status.version, status.item_count))
            .unwrap_or_else(|| "not reachable".to_owned()),
        status
            .map(|status| {
                if status.watcher_owned {
                    "ours".to_owned()
                } else if status.watcher_owner.is_empty() {
                    "nobody owns it".to_owned()
                } else {
                    format!("owned by {}", status.watcher_owner)
                }
            })
            .unwrap_or_else(|| "unknown".to_owned()),
        state.items.len().to_string(),
        status
            .map(|status| status.config_path.clone())
            .unwrap_or_default(),
        state.error.clone().unwrap_or_default(),
    ];

    for ((_, label), value) in widgets.status_rows.iter().zip(values) {
        label.set_text(value.trim());
    }
}

fn index_of(options: [&str; 3], value: &str) -> usize {
    options
        .iter()
        .position(|option| *option == value)
        .unwrap_or_default()
}

/// A small image for one icon of the wire format.
///
/// The icon travels as a PNG, which GDK decodes for us: no pixel format or
/// stride to agree on, and a broken image degrades to the fallback icon
/// instead of anything worse. Theme names are the fallback for applications
/// that name their icon instead of drawing it.
pub fn icon_image(icon: &IconView, pixel_size: i32) -> gtk::Image {
    let image = gtk::Image::new();
    image.set_pixel_size(pixel_size);

    if !icon.data.is_empty() {
        let bytes = glib::Bytes::from_owned(icon.data.clone());
        match gdk::Texture::from_bytes(&bytes) {
            Ok(texture) => image.set_paintable(Some(&texture)),
            Err(error) => {
                tracing::warn!(%error, "icon is not a readable image");
                image.set_icon_name(Some("image-missing"));
            }
        }
    } else if let Some(name) = non_empty(&icon.name) {
        image.set_paintable(Some(&themed_paintable(
            name,
            &icon.theme_path,
            pixel_size,
        )));
    } else {
        image.set_icon_name(Some("image-missing"));
    }
    image
}

fn themed_paintable(name: &str, theme_path: &str, pixel_size: i32) -> gdk::Paintable {
    let theme = gtk::IconTheme::for_display(&gdk::Display::default().expect("display"));
    if let Some(path) = non_empty(theme_path) {
        if std::path::Path::new(path).is_absolute() {
            theme.add_search_path(path);
        }
    }
    theme
        .lookup_icon(
            name,
            &[],
            pixel_size,
            1,
            gtk::TextDirection::None,
            gtk::IconLookupFlags::empty(),
        )
        .upcast()
}

fn non_empty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}
