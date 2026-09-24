//! The data contract shared with the GNOME Shell extension and the settings
//! app over D-Bus.
//!
//! These types are deliberately dumb records: they carry a fully resolved,
//! self-contained picture of one tray item or one menu so that the shell can
//! render them without knowing anything about StatusNotifierItem. All
//! decisions — which pixmap to use, how overlays blend, what a menu node means
//! — have already been made in [`crate`].

use serde::{Deserialize, Serialize};

use crate::icon::{composite, Pixmap};
use crate::menu::{MenuNode, MenuTree, NodeKind, ToggleState, ToggleType, ROOT_ID};
use crate::model::Item;
use crate::sni::{Icon, ToolTip};

/// One icon, ready to paint.
///
/// `data` holds raw ARGB32 pixels in network byte order (`width * 4` bytes
/// per row), which is what `St.ImageContent` and `gdk::MemoryTexture` both
/// accept directly. `name` is only filled in when the application supplied no
/// pixel data at all and the theme has to be consulted instead.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, zvariant::Type)]
pub struct IconView {
    pub name: String,
    pub theme_path: String,
    pub width: i32,
    pub height: i32,
    pub row_stride: i32,
    pub data: Vec<u8>,
}

impl IconView {
    /// Resolve `base` (with `overlay` drawn on top) at `target` pixels.
    ///
    /// Pixel data wins over theme names: it is what the application drew,
    /// it lets overlays be blended here rather than in two different GUI
    /// toolkits, and it needs no icon theme lookups. Theme names are kept as
    /// a fallback for applications that only name their icon.
    pub fn resolve(base: &Icon, overlay: Option<&Icon>, target: u32) -> Self {
        if let Some(pixmap) = base.best_pixmap(target) {
            let merged = match overlay.and_then(|overlay| overlay.best_pixmap(target)) {
                Some(overlay) => composite(pixmap, overlay),
                None => pixmap.clone(),
            };
            return Self::from_pixmap(&merged);
        }
        Self::named(&base.name, &base.theme_path)
    }

    pub fn from_pixmap(pixmap: &Pixmap) -> Self {
        Self {
            name: String::new(),
            theme_path: String::new(),
            width: pixmap.width,
            height: pixmap.height,
            row_stride: pixmap.row_stride() as i32,
            data: pixmap.data.clone(),
        }
    }

    pub fn named(name: &str, theme_path: &str) -> Self {
        Self {
            name: name.to_owned(),
            theme_path: theme_path.to_owned(),
            ..IconView::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.name.is_empty() && self.data.is_empty()
    }

    /// The pixels, if this icon carries any.
    pub fn to_pixmap(&self) -> Option<Pixmap> {
        (!self.data.is_empty()).then(|| Pixmap::new(self.width, self.height, self.data.clone()))
    }
}

/// One tray item as the panel shows it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, zvariant::Type)]
pub struct ItemView {
    /// Stable key used for configuration (hide, reorder).
    pub key: String,
    /// The application's own id, e.g. `"steam"`.
    pub app_id: String,
    pub title: String,
    pub tooltip: String,
    /// `"Passive"`, `"Active"` or `"NeedsAttention"`.
    pub status: String,
    pub accessible_name: String,
    pub has_menu: bool,
    /// A left click should open the menu instead of activating the item.
    pub item_is_menu: bool,
    pub icon: IconView,
}

impl ItemView {
    pub fn of(item: &Item, icon_pixel_size: u32) -> Self {
        let props = &item.props;
        let overlay = (!props.overlay.is_empty()).then_some(&props.overlay);
        Self {
            key: item.key.to_string(),
            app_id: props.id.clone(),
            title: props.title.clone(),
            tooltip: tooltip_text(&props.tooltip),
            status: props.status.as_str().to_owned(),
            accessible_name: props.accessible_name().to_owned(),
            has_menu: props.has_menu(),
            item_is_menu: props.item_is_menu,
            icon: IconView::resolve(props.effective_icon(), overlay, icon_pixel_size),
        }
    }
}

fn tooltip_text(tooltip: &ToolTip) -> String {
    if tooltip.description.is_empty() {
        tooltip.title.clone()
    } else {
        tooltip.description.clone()
    }
}

/// One menu entry. Nodes are sent as a flat, pre-order list with parent ids
/// rather than as a recursive tree, which keeps the D-Bus signature simple
/// for the JavaScript side.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, zvariant::Type)]
pub struct MenuNodeView {
    pub id: i32,
    pub parent: i32,
    /// `"standard"` or `"separator"`.
    pub kind: String,
    /// Label with the access key marker already stripped.
    pub label: String,
    pub enabled: bool,
    pub visible: bool,
    /// `-1` (not checkable), `0` (off) or `1` (on).
    pub toggle_state: i32,
    /// `"none"`, `"checkmark"` or `"radio"`.
    pub toggle_type: String,
    pub icon_name: String,
    /// Key chords like `[["Control", "Q"]]`.
    pub shortcut: Vec<Vec<String>>,
    pub has_children: bool,
}

impl MenuNodeView {
    pub fn of(node: &MenuNode, parent: i32) -> Self {
        Self {
            id: node.id,
            parent,
            kind: node.kind.as_str().to_owned(),
            label: node.label.clone(),
            enabled: node.enabled,
            visible: node.visible,
            toggle_state: node.toggle_state.as_i32(),
            toggle_type: node.toggle_type.as_str().to_owned(),
            icon_name: node.icon_name.clone(),
            shortcut: node.shortcut.clone(),
            has_children: !node.children.is_empty(),
        }
    }
}

/// A whole menu for one item.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, zvariant::Type)]
pub struct MenuView {
    pub key: String,
    pub revision: u64,
    pub nodes: Vec<MenuNodeView>,
}

/// Flatten a menu tree into the wire representation, pre-order, skipping the
/// invisible root node.
pub fn flatten_menu(key: &str, tree: &MenuTree) -> MenuView {
    let mut nodes = Vec::new();
    flatten_into(&tree.root.children, tree.root.id, &mut nodes);
    MenuView {
        key: key.to_owned(),
        revision: tree.revision,
        nodes,
    }
}

fn flatten_into(siblings: &[MenuNode], parent: i32, out: &mut Vec<MenuNodeView>) {
    for node in siblings {
        out.push(MenuNodeView::of(node, parent));
        flatten_into(&node.children, node.id, out);
    }
}

/// Rebuild a [`MenuTree`] from the flat representation. Useful for tests and
/// for clients that want the same navigation code as the daemon.
pub fn inflate_menu(view: &MenuView) -> MenuTree {
    let mut root = MenuNode::new(ROOT_ID);
    for node in &view.nodes {
        let mut rebuilt = MenuNode::new(node.id);
        rebuilt.kind = match node.kind.as_str() {
            "separator" => NodeKind::Separator,
            _ => NodeKind::Standard,
        };
        rebuilt.label = node.label.clone();
        rebuilt.enabled = node.enabled;
        rebuilt.visible = node.visible;
        rebuilt.toggle_state = ToggleState::parse(node.toggle_state as i64);
        rebuilt.toggle_type = ToggleType::parse(&node.toggle_type);
        rebuilt.icon_name = node.icon_name.clone();
        rebuilt.shortcut = node.shortcut.clone();

        match parent_of(&mut root, node.parent) {
            Some(parent) => parent.children.push(rebuilt),
            None => root.children.push(rebuilt),
        }
    }
    MenuTree {
        revision: view.revision,
        root,
    }
}

fn parent_of(node: &mut MenuNode, id: i32) -> Option<&mut MenuNode> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter_mut().find_map(|child| parent_of(child, id))
}

/// User preferences as sent over the wire.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, zvariant::Type)]
pub struct ConfigView {
    /// Logical pixels.
    pub icon_size: i32,
    /// `"registered"`, `"manual"` or `"title"`.
    pub sort: String,
    pub order: Vec<String>,
    pub hidden: Vec<String>,
    pub show_when_empty: bool,
    /// `"left"`, `"center"` or `"right"`.
    pub panel_box: String,
    pub panel_position: i32,
}

/// What the daemon knows about itself and the bus, for diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, zvariant::Type)]
pub struct StatusView {
    pub version: String,
    pub watcher_name: String,
    /// Whether `org.kde.StatusNotifierWatcher` is ours.
    pub watcher_owned: bool,
    /// Its current owner, empty when nobody owns it.
    pub watcher_owner: String,
    pub host_registered: bool,
    pub item_count: i32,
    pub revision: u64,
    pub config_path: String,
    /// Last error worth showing a user, empty when all is well.
    pub last_error: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icon::Pixmap;
    use crate::menu::{MenuNode, MenuTree};
    use crate::model::{Item, Key};
    use crate::props::PropValue;
    use crate::sni::{ItemProps, ServiceRef, Status};

    fn pixmap(size: i32, pixel: [u8; 4]) -> Pixmap {
        let mut p = Pixmap::new(size, size, vec![0; (size * size) as usize * 4]);
        for y in 0..size {
            for x in 0..size {
                p.set_pixel(x, y, pixel);
            }
        }
        p
    }

    fn item() -> Item {
        Item {
            key: Key::new("demo"),
            service: ServiceRef {
                bus_name: ":1.1".into(),
                object_path: "/StatusNotifierItem".into(),
            },
            props: ItemProps::default()
                .with_property("Id", "demo".into())
                .with_property("Title", "Demo".into())
                .with_property(
                    "IconPixmap",
                    PropValue::Pixmaps(vec![pixmap(8, [255, 1, 2, 3]), pixmap(32, [255, 4, 5, 6])]),
                )
                .with_property("Menu", "/MenuBar".into()),
            menu: None,
        }
    }

    #[test]
    fn item_view_resolves_the_icon_for_the_requested_size() {
        let view = ItemView::of(&item(), 22);
        assert_eq!(view.key, "demo");
        assert_eq!(view.icon.width, 32, "the 32px pixmap fills 22px best");
        assert_eq!(view.icon.row_stride, 32 * 4);
        assert!(view.has_menu);
        assert!(!view.icon.is_empty());
    }

    #[test]
    fn item_view_composites_the_overlay_icon() {
        let mut item = item();
        item.props = item
            .props
            .clone()
            .with_property("IconPixmap", PropValue::Pixmaps(vec![pixmap(16, [255, 0, 0, 0])]))
            .with_property(
                "OverlayIconPixmap",
                PropValue::Pixmaps(vec![pixmap(16, [255, 255, 255, 255])]),
            );

        let view = ItemView::of(&item, 16);
        let merged = view.icon.to_pixmap().unwrap();
        assert_eq!(merged.pixel(8, 8), Some([255, 255, 255, 255]));
    }

    #[test]
    fn item_view_falls_back_to_the_theme_name() {
        let mut item = item();
        item.props = item
            .props
            .clone()
            .with_property("IconPixmap", PropValue::Pixmaps(vec![]))
            .with_property("IconName", "demo-icon".into());

        let view = ItemView::of(&item, 22);
        assert!(view.icon.data.is_empty());
        assert_eq!(view.icon.name, "demo-icon");
    }

    #[test]
    fn attention_status_switches_icon_and_name() {
        let mut item = item();
        item.props = item
            .props
            .clone()
            .with_property("AttentionIconName", "alert".into())
            .with_property("Status", Status::NeedsAttention.as_str().into());

        assert_eq!(item.props.effective_icon().name, "alert");
    }

    #[test]
    fn menus_survive_the_round_trip() {
        let tree = MenuTree {
            revision: 7,
            root: MenuNode {
                id: ROOT_ID,
                children: vec![
                    MenuNode::new(1).with_property("label", "_Quit".into()),
                    MenuNode {
                        id: 2,
                        children: vec![MenuNode::new(3)
                            .with_property("type", "separator".into())],
                        ..MenuNode::default()
                    },
                ],
                ..MenuNode::default()
            },
        };

        let view = flatten_menu("demo", &tree);
        assert_eq!(
            view.nodes.iter().map(|n| (n.id, n.parent)).collect::<Vec<_>>(),
            [(1, 0), (2, 0), (3, 2)]
        );
        assert_eq!(view.nodes[0].label, "Quit");
        assert!(view.nodes[1].has_children);

        let restored = inflate_menu(&view);
        assert_eq!(restored.find(3).unwrap().kind, NodeKind::Separator);
        assert_eq!(restored.find(1).unwrap().label, "Quit");
        assert_eq!(restored.revision, 7);
    }

    /// The D-Bus signatures are the contract with the GNOME Shell extension,
    /// which unpacks items child by child (see `daemon.js`), and with the
    /// settings app. Pinning them here means an accidental field reorder
    /// cannot silently break those clients.
    #[test]
    fn wire_signatures_are_stable() {
        fn signature<T: zvariant::Type>() -> String {
            T::SIGNATURE.to_string()
        }

        assert_eq!(signature::<ItemView>(), "(ssssssbb(ssiiiay))");
        assert_eq!(signature::<IconView>(), "(ssiiiay)");
        assert_eq!(signature::<MenuView>(), "(sta(iissbbissaasb))");
        assert_eq!(signature::<ConfigView>(), "(isasasbsi)");
        assert_eq!(signature::<StatusView>(), "(ssbsbitss)");
    }
}
