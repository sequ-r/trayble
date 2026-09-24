//! Domain model of a StatusNotifierItem, the tray protocol defined by
//! freedesktop.org/KDE and implemented by libappindicator, libayatana and Qt.
//!
//! Only the shape of the data and pure derivations live here. Talking to the
//! application that owns the item is the daemon's job.

use crate::icon::{pick_best, Pixmap};
use crate::props::{PropMap, PropValue};

/// Well-known bus name every tray application looks for.
pub const WATCHER_BUS_NAME: &str = "org.kde.StatusNotifierWatcher";
pub const WATCHER_OBJECT_PATH: &str = "/StatusNotifierWatcher";
pub const WATCHER_INTERFACE: &str = "org.kde.StatusNotifierWatcher";
pub const HOST_INTERFACE: &str = "org.kde.StatusNotifierHost";

/// Interface a tray item exposes.
pub const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";
/// Object path used when the registration did not name one.
pub const DEFAULT_ITEM_PATH: &str = "/StatusNotifierItem";

/// `Status` of an item: controls visibility and which icon is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// The item asks not to be displayed at all.
    Passive,
    #[default]
    Active,
    /// The item wants attention: its attention icon takes over.
    NeedsAttention,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Passive => "Passive",
            Status::Active => "Active",
            Status::NeedsAttention => "NeedsAttention",
        }
    }

    /// Unknown values map to `Active`, the least surprising default.
    pub fn parse(value: &str) -> Self {
        match value {
            "Passive" => Status::Passive,
            "NeedsAttention" => Status::NeedsAttention,
            _ => Status::Active,
        }
    }
}

/// Coarse classification of what the item represents.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Category {
    #[default]
    ApplicationStatus,
    Communications,
    SystemServices,
    Hardware,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::ApplicationStatus => "ApplicationStatus",
            Category::Communications => "Communications",
            Category::SystemServices => "SystemServices",
            Category::Hardware => "Hardware",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "Communications" => Category::Communications,
            "SystemServices" => Category::SystemServices,
            "Hardware" => Category::Hardware,
            _ => Category::ApplicationStatus,
        }
    }
}

/// An icon as the application supplies it: a theme name *and* a list of raw
/// pixmaps, either of which may be empty.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Icon {
    pub name: String,
    pub theme_path: String,
    pub pixmaps: Vec<Pixmap>,
}

impl Icon {
    pub fn is_empty(&self) -> bool {
        self.name.is_empty() && !self.pixmaps.iter().any(Pixmap::is_valid)
    }

    /// The pixmap closest to `target` pixels, if any.
    pub fn best_pixmap(&self, target: u32) -> Option<&Pixmap> {
        pick_best(&self.pixmaps, target)
    }
}

/// The hover tooltip of an item.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolTip {
    pub icon_name: String,
    pub icon_pixmaps: Vec<Pixmap>,
    pub title: String,
    pub description: String,
}

/// All properties of a StatusNotifierItem, decoded and defaulted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemProps {
    pub category: Category,
    pub id: String,
    pub title: String,
    pub status: Status,
    pub window_id: u32,
    pub icon_theme_path: String,
    pub icon: Icon,
    pub overlay: Icon,
    pub attention: Icon,
    pub attention_movie: String,
    pub tooltip: ToolTip,
    /// Object path of the item's `com.canonical.dbusmenu`, or empty.
    pub menu_path: String,
    /// When true a left click should open the menu instead of activating.
    pub item_is_menu: bool,
    pub icon_accessible_desc: String,
    pub attention_accessible_desc: String,
    /// Ayatana extension: short text shown next to the icon.
    pub label: String,
    pub label_guide: String,
}

impl ItemProps {
    pub fn from_map(map: PropMap) -> Self {
        Self::default().with_properties(&map)
    }

    /// Fold a whole property dictionary into a copy of `self`.
    pub fn with_properties(self, properties: &PropMap) -> Self {
        properties
            .iter()
            .fold(self, |props, (name, value)| {
                props.with_property(name, value.clone())
            })
    }

    /// Fold one assignment into a copy of `self`. Unknown properties are
    /// ignored so that spec additions do not break older trays.
    pub fn with_property(mut self, name: &str, value: PropValue) -> Self {
        let text = || value.as_str().unwrap_or_default().to_owned();
        let pixmaps = || value.as_pixmaps().unwrap_or_default().to_vec();

        match name {
            "Category" => self.category = Category::parse(&text()),
            "Id" => self.id = text(),
            "Title" => self.title = text(),
            "Status" => self.status = Status::parse(&text()),
            "WindowId" => self.window_id = value.as_int().unwrap_or_default() as u32,
            "IconThemePath" => {
                self.icon_theme_path = text();
                let path = self.icon_theme_path.clone();
                self.icon.theme_path = path.clone();
                self.overlay.theme_path = path.clone();
                self.attention.theme_path = path;
            }
            "IconName" => self.icon.name = text(),
            "IconPixmap" => self.icon.pixmaps = pixmaps(),
            "OverlayIconName" => self.overlay.name = text(),
            "OverlayIconPixmap" => self.overlay.pixmaps = pixmaps(),
            "AttentionIconName" => self.attention.name = text(),
            "AttentionIconPixmap" => self.attention.pixmaps = pixmaps(),
            "AttentionMovieName" => self.attention_movie = text(),
            "IconAccessibleDesc" => self.icon_accessible_desc = text(),
            "AttentionAccessibleDesc" => self.attention_accessible_desc = text(),
            "ItemIsMenu" => self.item_is_menu = value.as_bool().unwrap_or_default(),
            "Menu" => self.menu_path = text(),
            "XAyatanaLabel" => self.label = text(),
            "XAyatanaLabelGuide" => self.label_guide = text(),
            "ToolTip" => {
                if let PropValue::ToolTip(tooltip) = value {
                    self.tooltip = *tooltip;
                }
            }
            _ => {}
        }
        self
    }

    /// The icon to display right now: items that need attention show their
    /// attention icon when they have one.
    pub fn effective_icon(&self) -> &Icon {
        if self.status == Status::NeedsAttention && !self.attention.is_empty() {
            &self.attention
        } else {
            &self.icon
        }
    }

    /// Per the spec, `Passive` items must not be shown.
    pub fn is_visible(&self) -> bool {
        self.status != Status::Passive
    }

    /// Text used by assistive technologies and as a fallback title.
    pub fn accessible_name(&self) -> &str {
        let name = match self.status {
            Status::NeedsAttention if !self.attention_accessible_desc.is_empty() => {
                &self.attention_accessible_desc
            }
            _ if !self.icon_accessible_desc.is_empty() => &self.icon_accessible_desc,
            _ if !self.label.is_empty() => &self.label,
            _ => &self.title,
        };
        if name.is_empty() {
            &self.id
        } else {
            name
        }
    }

    /// True when the item exposes a menu we can show.
    pub fn has_menu(&self) -> bool {
        !self.menu_path.is_empty() && self.menu_path != "/NO_DBUSMENU"
    }
}

/// Where a tray item lives on the bus.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ServiceRef {
    pub bus_name: String,
    pub object_path: String,
}

impl ServiceRef {
    /// Resolve the `service` argument of `RegisterStatusNotifierItem`.
    ///
    /// The spec allows three spellings, depending on how much the client
    /// bothers to say about itself:
    ///
    /// * `"/Some/Path"` — path on the caller's own connection,
    /// * `"bus.name/Some/Path"` — explicit bus name and path,
    /// * `"bus.name"` — explicit bus name, default item path.
    pub fn resolve(service: &str, caller: &str) -> Self {
        let service = service.trim();
        if service.is_empty() {
            return Self {
                bus_name: caller.to_owned(),
                object_path: DEFAULT_ITEM_PATH.to_owned(),
            };
        }
        if let Some((bus_name, object_path)) = service.split_once('/') {
            if bus_name.is_empty() {
                // "/Some/Path": the caller owns the item.
                Self {
                    bus_name: caller.to_owned(),
                    object_path: normalize_path(object_path),
                }
            } else {
                Self {
                    bus_name: bus_name.to_owned(),
                    object_path: normalize_path(object_path),
                }
            }
        } else {
            Self {
                bus_name: service.to_owned(),
                object_path: DEFAULT_ITEM_PATH.to_owned(),
            }
        }
    }

    /// The spelling other clients see: `bus.name/Object/Path`. Used by
    /// `RegisteredStatusNotifierItems` and by the registration signals of the
    /// watcher, and accepted again by [`ServiceRef::resolve`].
    pub fn registration_id(&self) -> String {
        format!("{}{}", self.bus_name, self.object_path)
    }
}

fn normalize_path(object_path: &str) -> String {
    if object_path.is_empty() {
        DEFAULT_ITEM_PATH.to_owned()
    } else {
        let trimmed = object_path.trim_start_matches('/');
        format!("/{trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::props::PropValue;

    fn props_with(name: &str, value: PropValue) -> ItemProps {
        ItemProps::default().with_property(name, value)
    }

    #[test]
    fn registration_resolves_all_three_spellings() {
        let explicit = ServiceRef::resolve("org.example.Tray/StatusNotifierItem", ":1.7");
        assert_eq!(explicit.bus_name, "org.example.Tray");
        assert_eq!(explicit.object_path, "/StatusNotifierItem");

        let path_only = ServiceRef::resolve("/org/example/Item", ":1.7");
        assert_eq!(path_only.bus_name, ":1.7");
        assert_eq!(path_only.object_path, "/org/example/Item");

        let name_only = ServiceRef::resolve("org.example.Tray", ":1.7");
        assert_eq!(name_only.bus_name, "org.example.Tray");
        assert_eq!(name_only.object_path, "/StatusNotifierItem");
    }

    #[test]
    fn registration_ids_round_trip() {
        let service = ServiceRef::resolve("org.example.Tray/StatusNotifierItem", ":1.7");
        assert_eq!(service.registration_id(), "org.example.Tray/StatusNotifierItem");

        // The plain path spelling resolves against the caller and comes back
        // out with the caller's name, ready to be registered again.
        let service = ServiceRef::resolve("/StatusNotifierItem", ":1.7");
        assert_eq!(service.registration_id(), ":1.7/StatusNotifierItem");
        assert_eq!(
            ServiceRef::resolve(&service.registration_id(), ":1.7"),
            service
        );
    }

    #[test]
    fn status_parsing_is_lenient() {
        assert_eq!(Status::parse("NeedsAttention"), Status::NeedsAttention);
        assert_eq!(Status::parse("Passive"), Status::Passive);
        assert_eq!(Status::parse("SomethingElse"), Status::Active);
    }

    #[test]
    fn attention_icon_takes_over_when_needed() {
        let props = ItemProps::default()
            .with_property("IconName", "app-icon".into())
            .with_property("AttentionIconName", "app-alert".into())
            .with_property("Status", "Active".into());

        assert_eq!(props.effective_icon().name, "app-icon");

        let props = props.with_property("Status", "NeedsAttention".into());
        assert_eq!(props.effective_icon().name, "app-alert");
    }

    #[test]
    fn icon_theme_path_reaches_every_icon() {
        let props = props_with("IconThemePath", "/opt/app/icons".into());
        assert_eq!(props.icon.theme_path, "/opt/app/icons");
        assert_eq!(props.overlay.theme_path, "/opt/app/icons");
        assert_eq!(props.attention.theme_path, "/opt/app/icons");
    }

    #[test]
    fn passive_items_are_hidden() {
        assert!(ItemProps::default().is_visible());
        assert!(!props_with("Status", "Passive".into()).is_visible());
    }

    #[test]
    fn accessible_name_falls_back_through_the_spec() {
        let props = props_with("Id", "demo".into());
        assert_eq!(props.accessible_name(), "demo");

        let props = props
            .with_property("Title", "Demo App".into())
            .with_property("IconAccessibleDesc", "Demo is running".into());
        assert_eq!(props.accessible_name(), "Demo is running");
    }

    #[test]
    fn no_dbusmenu_marker_disables_the_menu() {
        assert!(!props_with("Menu", "/NO_DBUSMENU".into()).has_menu());
        assert!(props_with("Menu", "/MenuBar".into()).has_menu());
    }

    #[test]
    fn unknown_properties_are_ignored() {
        let props = props_with("Whatever", PropValue::Unsupported("v".into()));
        assert_eq!(props, ItemProps::default());
    }
}
