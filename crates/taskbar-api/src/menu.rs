//! Client proxy for `com.canonical.dbusmenu`, the menu protocol used by
//! StatusNotifierItem clients, plus the decoding of its layouts into
//! [`taskbar_core::menu::MenuTree`].

use std::collections::HashMap;

use taskbar_core::menu::{MenuNode, MenuTree};
use taskbar_core::props::PropMap;
use zbus::proxy;
use zvariant::{OwnedValue, Value};

use crate::value::{dict_to_prop_map, to_prop_map, to_prop_value};

/// A property dictionary as it travels on the wire.
pub type PropertyMap = HashMap<String, OwnedValue>;

/// `(id, properties, children)`, the recursive node of `GetLayout`. Children
/// are wrapped in variants (`av`) which is what makes recursion expressible in
/// a D-Bus type signature.
pub type MenuLayout = (i32, PropertyMap, Vec<OwnedValue>);

/// Properties we ask for; everything else is left at its spec default.
pub const WANTED_PROPERTIES: &[&str] = &[
    "type",
    "label",
    "enabled",
    "visible",
    "toggle-state",
    "toggle-type",
    "icon-name",
    "shortcut",
];

#[proxy(
    interface = "com.canonical.dbusmenu",
    assume_defaults = false,
    gen_async = true,
    gen_blocking = false,
)]
pub trait DBusMenu {
    /// The menu tree under `parent_id` (`0` is the invisible root).
    /// A `recursion_depth` of `-1` means "everything".
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: &[&str],
    ) -> zbus::Result<(u32, MenuLayout)>;

    fn get_group_properties(
        &self,
        ids: &[i32],
        property_names: &[&str],
    ) -> zbus::Result<Vec<(i32, PropertyMap)>>;

    fn get_property(&self, id: i32, name: &str) -> zbus::Result<OwnedValue>;

    /// `event_id` is `"clicked"`, `"hovered"`, `"opened"` or `"closed"`.
    fn event(
        &self,
        id: i32,
        event_id: &str,
        data: &Value<'_>,
        timestamp: u32,
    ) -> zbus::Result<()>;

    /// Returns true when the application updated the menu and it has to be
    /// fetched again.
    fn about_to_show(&self, id: i32) -> zbus::Result<bool>;

    fn about_to_show_group(&self, ids: &[i32]) -> zbus::Result<(Vec<i32>, Vec<i32>)>;

    #[zbus(signal)]
    fn layout_updated(&self, revision: u32, parent: i32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn items_properties_updated(
        &self,
        updated_props: Vec<(i32, PropertyMap)>,
        removed_props: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    fn item_activation_requested(&self, id: i32, timestamp: u32) -> zbus::Result<()>;
}

/// Decode a whole `GetLayout` reply.
pub fn layout_to_tree(revision: u32, layout: &MenuLayout) -> MenuTree {
    let (id, properties, children) = layout;
    let mut root = MenuNode::new(*id).with_properties(&to_prop_map(properties));
    root.children = children.iter().filter_map(child_to_node).collect();
    MenuTree {
        revision: u64::from(revision),
        root,
    }
}

/// Decode a node that arrived inside a variant of the `children` array.
pub fn child_to_node(child: &OwnedValue) -> Option<MenuNode> {
    node_from_value(child)
}

/// Decode one `(id, properties, children)` structure at any depth. Nodes that
/// arrive wrapped in a variant (every child of an `av` array is) are unwrapped
/// first.
pub fn node_from_value(value: &Value<'_>) -> Option<MenuNode> {
    let value = match value {
        Value::Value(inner) => inner.as_ref(),
        other => other,
    };
    let Value::Structure(structure) = value else {
        return None;
    };
    let fields = structure.fields();
    let id = match fields.first()? {
        Value::I32(id) => *id,
        _ => return None,
    };

    let mut node = MenuNode::new(id);
    if let Some(Value::Dict(properties)) = fields.get(1) {
        node = node.with_properties(&dict_to_prop_map(properties));
    }
    if let Some(Value::Array(children)) = fields.get(2) {
        node.children = children.inner().iter().filter_map(node_from_value).collect();
    }
    Some(node)
}

/// A menu patch as `ItemsPropertiesUpdated` sends it: properties to replace
/// and property names to reset, both keyed by node id.
pub type MenuPatch = (Vec<(i32, PropMap)>, Vec<(i32, Vec<String>)>);

/// Convert an `ItemsPropertiesUpdated` payload into the domain patch form.
pub fn updates_to_patch(
    updated: &[(i32, PropertyMap)],
    removed: &[(i32, Vec<String>)],
) -> MenuPatch {
    let updates = updated
        .iter()
        .map(|(id, properties)| (*id, to_prop_map(properties)))
        .collect();
    let removals = removed
        .iter()
        .map(|(id, names)| (*id, names.clone()))
        .collect();
    (updates, removals)
}

/// Decode a single property value, e.g. of `GetProperty`.
pub fn owned_to_prop(value: &OwnedValue) -> taskbar_core::props::PropValue {
    to_prop_value(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use taskbar_core::menu::{NodeKind, ToggleState};
    use zvariant::Structure;

    /// A wire accurate `a{sv}` property dictionary.
    type TestProps = HashMap<String, Value<'static>>;

    fn props(entries: &[(&'static str, Value<'static>)]) -> TestProps {
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.try_clone().expect("clone")))
            .collect()
    }

    /// A wire accurate `(ia{sv}av)` node wrapped in the variant a `children`
    /// array carries.
    fn node(id: i32, entries: &[(&'static str, Value<'static>)]) -> OwnedValue {
        node_with(id, entries, Vec::new())
    }

    fn node_with(
        id: i32,
        entries: &[(&'static str, Value<'static>)],
        children: Vec<OwnedValue>,
    ) -> OwnedValue {
        let structure = Structure::from((id, props(entries), children));
        OwnedValue::try_from(Value::Structure(structure)).expect("owned node")
    }

    #[test]
    fn layouts_decode_at_every_depth() {
        let leaf = node(3, &[("type", Value::from("separator"))]);
        let child = node_with(
            2,
            &[
                ("label", Value::from("_Sub")),
                ("enabled", Value::from(false)),
            ],
            vec![leaf],
        );
        let layout: MenuLayout = (
            0,
            [("label".to_owned(), OwnedValue::try_from(Value::from("_Quit")).unwrap())]
                .into(),
            vec![child],
        );

        let tree = layout_to_tree(9, &layout);
        assert_eq!(tree.revision, 9);

        assert_eq!(tree.root.children.len(), 1);

        let sub = tree.find(2).expect("submenu");
        assert_eq!(sub.label, "Sub");
        assert!(!sub.enabled);
        assert_eq!(sub.children.len(), 1);

        let separator = tree.find(3).expect("separator");
        assert_eq!(separator.kind, NodeKind::Separator);
        assert_eq!(separator.toggle_state, ToggleState::Unset);
    }

    #[test]
    fn labels_are_stripped_of_mnemonics() {
        let layout: MenuLayout = (
            0,
            [("label".to_owned(), OwnedValue::try_from(Value::from("_Quit")).unwrap())]
                .into(),
            vec![node(1, &[("label", Value::from("_Save _As"))])],
        );

        let tree = layout_to_tree(1, &layout);
        assert_eq!(tree.find(1).expect("entry").label, "Save As");
    }
}
