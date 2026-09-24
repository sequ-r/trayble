//! Domain model of a `com.canonical.dbusmenu`, the menu protocol used by
//! StatusNotifierItem clients (Qt, libdbusmenu, libayatana).
//!
//! Menus arrive as deeply nested property bags. This module turns them into a
//! [`MenuTree`] of typed [`MenuNode`]s and keeps it up to date as the
//! application patches its menu.

use crate::props::{PropMap, PropValue};

pub const DBUSMENU_INTERFACE: &str = "com.canonical.dbusmenu";
/// Marker path meaning "this item has no menu".
pub const NO_DBUSMENU_PATH: &str = "/NO_DBUSMENU";
/// The id of the invisible root node.
pub const ROOT_ID: i32 = 0;

/// What a node is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NodeKind {
    #[default]
    Standard,
    Separator,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Standard => "standard",
            NodeKind::Separator => "separator",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "separator" => NodeKind::Separator,
            _ => NodeKind::Standard,
        }
    }
}

/// State of a checkable item: `-1` when the item is not checkable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToggleState {
    #[default]
    Unset,
    Off,
    On,
}

impl ToggleState {
    pub fn as_i32(self) -> i32 {
        match self {
            ToggleState::Unset => -1,
            ToggleState::Off => 0,
            ToggleState::On => 1,
        }
    }

    pub fn parse(value: i64) -> Self {
        match value {
            0 => ToggleState::Off,
            1 => ToggleState::On,
            _ => ToggleState::Unset,
        }
    }
}

/// How a checkable item behaves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToggleType {
    #[default]
    None,
    Checkmark,
    Radio,
}

impl ToggleType {
    pub fn as_str(self) -> &'static str {
        match self {
            ToggleType::None => "none",
            ToggleType::Checkmark => "checkmark",
            ToggleType::Radio => "radio",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "checkmark" => ToggleType::Checkmark,
            "radio" => ToggleType::Radio,
            _ => ToggleType::None,
        }
    }
}

/// One entry of a menu, with its children.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuNode {
    pub id: i32,
    pub kind: NodeKind,
    pub label: String,
    pub enabled: bool,
    pub visible: bool,
    pub toggle_state: ToggleState,
    pub toggle_type: ToggleType,
    pub icon_name: String,
    /// Accelerator as a list of key chords, e.g. `[["Control", "Q"]]`.
    pub shortcut: Vec<Vec<String>>,
    pub children: Vec<MenuNode>,
}

impl Default for MenuNode {
    fn default() -> Self {
        Self {
            id: ROOT_ID,
            kind: NodeKind::Standard,
            label: String::new(),
            enabled: true,
            visible: true,
            toggle_state: ToggleState::Unset,
            toggle_type: ToggleType::None,
            icon_name: String::new(),
            shortcut: Vec::new(),
            children: Vec::new(),
        }
    }
}

impl MenuNode {
    pub fn new(id: i32) -> Self {
        Self {
            id,
            ..MenuNode::default()
        }
    }

    /// Fold a property dictionary into a copy of the node.
    pub fn with_properties(self, properties: &PropMap) -> Self {
        properties.iter().fold(self, |node, (name, value)| {
            node.with_property(name, value.clone())
        })
    }

    pub fn with_property(mut self, name: &str, value: PropValue) -> Self {
        match name {
            "type" => self.kind = NodeKind::parse(value.as_str().unwrap_or_default()),
            // The label carries the access key marker; see [`strip_mnemonic`].
            "label" => self.label = strip_mnemonic(value.as_str().unwrap_or_default()),
            "enabled" => self.enabled = value.as_bool().unwrap_or(true),
            "visible" => self.visible = value.as_bool().unwrap_or(true),
            "toggle-state" => {
                self.toggle_state = ToggleState::parse(value.as_int().unwrap_or(-1))
            }
            "toggle-type" => {
                self.toggle_type = ToggleType::parse(value.as_str().unwrap_or_default())
            }
            "icon-name" => self.icon_name = value.as_str().unwrap_or_default().to_owned(),
            "shortcut" => {
                if let Some(chords) = value.as_string_lists() {
                    self.shortcut = chords.to_vec();
                }
            }
            _ => {}
        }
        self
    }

    /// A copy of the node with a property reset to its spec default.
    pub fn without_property(mut self, name: &str) -> Self {
        let default = MenuNode::new(self.id);
        match name {
            "type" => self.kind = default.kind,
            "label" => self.label = default.label,
            "enabled" => self.enabled = default.enabled,
            "visible" => self.visible = default.visible,
            "toggle-state" => self.toggle_state = default.toggle_state,
            "toggle-type" => self.toggle_type = default.toggle_type,
            "icon-name" => self.icon_name = default.icon_name,
            "shortcut" => self.shortcut = default.shortcut,
            _ => {}
        }
        self
    }

    pub fn is_separator(&self) -> bool {
        self.kind == NodeKind::Separator
    }

    pub fn find(&self, id: i32) -> Option<&MenuNode> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(id))
    }

    /// Number of nodes in this subtree, including `self`.
    pub fn len(&self) -> usize {
        1 + self.children.iter().map(MenuNode::len).sum::<usize>()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 1
    }
}

/// A menu: the root node (whose children are the visible entries) plus the
/// revision reported by the application.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuTree {
    pub revision: u64,
    pub root: MenuNode,
}

impl MenuTree {
    /// A copy with the `ItemsPropertiesUpdated` patch applied: properties of
    /// existing nodes replaced or reset, children untouched.
    pub fn updated(&self, updates: &[(i32, PropMap)], removals: &[(i32, Vec<String>)]) -> Self {
        Self {
            revision: self.revision,
            root: patch_node(self.root.clone(), updates, removals),
        }
    }

    pub fn find(&self, id: i32) -> Option<&MenuNode> {
        self.root.find(id)
    }
}

fn patch_node(
    node: MenuNode,
    updates: &[(i32, PropMap)],
    removals: &[(i32, Vec<String>)],
) -> MenuNode {
    let id = node.id;
    let children = node
        .children
        .into_iter()
        .map(|child| patch_node(child, updates, removals))
        .collect();

    let node = MenuNode { children, ..node };
    let node = updates
        .iter()
        .filter(|(updated, _)| *updated == id)
        .fold(node, |node, (_, props)| node.with_properties(props));

    removals
        .iter()
        .filter(|(removed, _)| *removed == id)
        .fold(node, |node, (_, names)| {
            names.iter().fold(node, |node, name| node.without_property(name))
        })
}

/// Render a dbusmenu label as text.
///
/// The protocol encodes access keys in the label: `"__"` is a literal
/// underscore, and the first remaining `"_"` marks the access key (the
/// character after it) and is not displayed. So `"_Quit"` becomes `"Quit"`
/// with the access key `Q`, and `"A__B"` becomes `"A_B"`.
pub fn strip_mnemonic(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '_' {
            out.push(ch);
            continue;
        }
        // "__" is an escaped underscore; a lone one marks the access key and
        // disappears.
        if let Some('_') = chars.peek() {
            chars.next();
            out.push('_');
        }
    }
    out
}

/// The access key marked in a label, if any.
pub fn mnemonic_of(label: &str) -> Option<char> {
    let mut chars = label.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '_' {
            continue;
        }
        match chars.peek() {
            Some('_') => {
                chars.next();
            }
            Some(&key) => return Some(key),
            None => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: i32) -> MenuNode {
        MenuNode::new(id)
    }

    fn tree() -> MenuTree {
        MenuTree {
            revision: 3,
            root: MenuNode {
                id: ROOT_ID,
                children: vec![
                    node(1).with_property("label", "_Quit".into()),
                    node(2).with_property("type", "separator".into()),
                    MenuNode {
                        id: 3,
                        label: "Sub".into(),
                        children: vec![node(4)],
                        ..MenuNode::default()
                    },
                ],
                ..MenuNode::default()
            },
        }
    }

    #[test]
    fn mnemonics_follow_the_protocol() {
        assert_eq!(strip_mnemonic("_Quit"), "Quit");
        assert_eq!(mnemonic_of("_Quit"), Some('Q'));
        assert_eq!(strip_mnemonic("A__B"), "A_B");
        assert_eq!(mnemonic_of("A__B"), None);
        assert_eq!(strip_mnemonic("Plain"), "Plain");
        assert_eq!(strip_mnemonic("Trailing_"), "Trailing");
    }

    #[test]
    fn nodes_default_like_the_spec_says() {
        let node = node(7)
            .with_property("label", "_Save".into())
            .with_property("enabled", false.into())
            .with_property("toggle-state", 1.into())
            .with_property("toggle-type", "checkmark".into());

        assert_eq!(node.label, "Save");
        assert!(!node.enabled);
        assert_eq!(node.toggle_state, ToggleState::On);
        assert_eq!(node.toggle_type, ToggleType::Checkmark);
        // Unset properties keep their documented defaults.
        assert!(node.visible);
        assert_eq!(node.kind, NodeKind::Standard);
    }

    #[test]
    fn removals_reset_to_defaults() {
        let node = node(7)
            .with_property("label", "_Save".into())
            .with_property("enabled", false.into())
            .without_property("enabled");

        assert_eq!(node.label, "Save", "untouched properties survive");
        assert!(node.enabled, "removed property is back to its default");
    }

    #[test]
    fn updates_patch_nested_nodes() {
        let patched = tree().updated(
            &[(4, PropMap::from([("label".to_owned(), "Inner".into())]))],
            &[(2, vec!["type".to_owned()])],
        );

        assert_eq!(patched.find(4).unwrap().label, "Inner");
        assert_eq!(patched.find(2).unwrap().kind, NodeKind::Standard);
        assert_eq!(patched.find(1).unwrap().label, "Quit");
        assert_eq!(patched.find(3).unwrap().children.len(), 1);
    }

    #[test]
    fn find_reaches_into_submenus() {
        let tree = tree();
        assert_eq!(tree.find(4).map(|n| n.id), Some(4));
        assert_eq!(tree.find(99), None);
        assert_eq!(tree.root.len(), 5);
    }
}
