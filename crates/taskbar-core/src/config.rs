//! User preferences.
//!
//! The config is a plain value: the daemon owns the file on disk and every
//! other component reads and writes it through the daemon's D-Bus API, so
//! there is exactly one writer and one source of truth.

use serde::{Deserialize, Serialize};

use crate::view::ConfigView;

/// How tray icons are ordered in the panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SortMode {
    /// Order in which applications registered their icon.
    #[default]
    Registered,
    /// Manual order maintained by the user ([`Config::order`]).
    Manual,
    /// Alphabetically by the item's title.
    Title,
}

impl SortMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SortMode::Registered => "registered",
            SortMode::Manual => "manual",
            SortMode::Title => "title",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "manual" => SortMode::Manual,
            "title" => SortMode::Title,
            _ => SortMode::Registered,
        }
    }
}

/// The panel area the tray indicator is added to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PanelBox {
    Left,
    Center,
    #[default]
    Right,
}

impl PanelBox {
    pub fn as_str(self) -> &'static str {
        match self {
            PanelBox::Left => "left",
            PanelBox::Center => "center",
            PanelBox::Right => "right",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "left" => PanelBox::Left,
            "center" => PanelBox::Center,
            _ => PanelBox::Right,
        }
    }
}

/// Everything the user can configure. `order` and `hidden` refer to item keys
/// (see [`crate::model::Key`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Height of a tray icon in logical pixels.
    pub icon_size: u32,
    pub sort: SortMode,
    /// Manual ordering of item keys; items not listed keep registration order
    /// and come last.
    pub order: Vec<String>,
    /// Item keys the user asked to hide.
    pub hidden: Vec<String>,
    /// Keep the indicator in the panel while no item is registered.
    pub show_when_empty: bool,
    /// Panel area the indicator lives in.
    pub panel_box: PanelBox,
    /// Position inside that area.
    pub panel_position: i32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            icon_size: 16,
            sort: SortMode::Registered,
            order: Vec::new(),
            hidden: Vec::new(),
            show_when_empty: false,
            panel_box: PanelBox::Right,
            panel_position: 0,
        }
    }
}

impl Config {
    /// A copy with invariants enforced: bounded icon size, no empty or
    /// duplicated keys. Everything downstream may assume a normalised config.
    pub fn normalized(self) -> Self {
        fn dedupe(keys: Vec<String>) -> Vec<String> {
            let mut seen = std::collections::BTreeSet::new();
            keys.into_iter()
                .map(|key| key.trim().to_owned())
                .filter(|key| !key.is_empty() && seen.insert(key.clone()))
                .collect()
        }

        Self {
            icon_size: self.icon_size.clamp(8, 128),
            sort: self.sort,
            order: dedupe(self.order),
            hidden: dedupe(self.hidden),
            show_when_empty: self.show_when_empty,
            panel_box: self.panel_box,
            panel_position: self.panel_position.clamp(0, 64),
        }
    }

    pub fn is_hidden(&self, key: &str) -> bool {
        self.hidden.iter().any(|hidden| hidden == key)
    }

    /// A copy with `key` hidden or shown again.
    pub fn with_hidden(mut self, key: &str, hidden: bool) -> Self {
        self.hidden.retain(|entry| entry != key);
        if hidden {
            self.hidden.push(key.to_owned());
        }
        self.normalized()
    }

    /// A copy ordered by `order` (which becomes the manual order).
    pub fn with_order(mut self, order: Vec<String>) -> Self {
        self.sort = SortMode::Manual;
        self.order = order;
        self.normalized()
    }

    pub fn to_view(&self) -> ConfigView {
        ConfigView {
            icon_size: self.icon_size as i32,
            sort: self.sort.as_str().to_owned(),
            order: self.order.clone(),
            hidden: self.hidden.clone(),
            show_when_empty: self.show_when_empty,
            panel_box: self.panel_box.as_str().to_owned(),
            panel_position: self.panel_position,
        }
    }

    pub fn from_view(view: ConfigView) -> Self {
        Self {
            icon_size: view.icon_size as u32,
            sort: SortMode::parse(&view.sort),
            order: view.order,
            hidden: view.hidden,
            show_when_empty: view.show_when_empty,
            panel_box: PanelBox::parse(&view.panel_box),
            panel_position: view.panel_position,
        }
        .normalized()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_sane() {
        let config = Config::default().normalized();
        assert_eq!(config.icon_size, 16);
        assert_eq!(config.sort, SortMode::Registered);
        assert!(!config.show_when_empty);
    }

    #[test]
    fn normalisation_dedupes_and_clamps() {
        let config = Config {
            icon_size: 4,
            order: vec!["a".into(), "a".into(), "  ".into(), "b".into()],
            hidden: vec!["a".into(), "a".into()],
            panel_position: -5,
            ..Config::default()
        }
        .normalized();

        assert_eq!(config.icon_size, 8);
        assert_eq!(config.order, vec!["a", "b"]);
        assert_eq!(config.hidden, vec!["a"]);
        assert_eq!(config.panel_position, 0);
    }

    #[test]
    fn with_hidden_toggles_key() {
        let config = Config::default().with_hidden("steam", true);
        assert!(config.is_hidden("steam"));
        let config = config.with_hidden("steam", false);
        assert!(!config.is_hidden("steam"));
        assert!(config.hidden.is_empty());
    }

    #[test]
    fn config_view_round_trips() {
        let config = Config::default()
            .with_hidden("discord", true)
            .with_order(vec!["b".into(), "a".into()]);
        let restored = Config::from_view(config.to_view());
        assert_eq!(restored, config);
    }
}
