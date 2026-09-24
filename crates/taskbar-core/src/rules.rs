//! Ordering and visibility of tray items.
//!
//! Pure queries and permutations over the registered items; the panel shows
//! exactly what these functions return.

use crate::config::{Config, SortMode};
use crate::model::{Item, Key};

/// The order in which items are shown: whatever the user arranged when the
/// sort mode is manual, otherwise registration order or alphabetical order.
/// Items without an explicit place keep their registration order and come
/// last.
pub fn ordered_keys(items: &[Item], config: &Config) -> Vec<Key> {
    match config.sort {
        SortMode::Registered => items.iter().map(|item| item.key.clone()).collect(),
        SortMode::Title => {
            let mut sorted: Vec<&Item> = items.iter().collect();
            sorted.sort_by(|left, right| {
                left.props
                    .accessible_name()
                    .to_lowercase()
                    .cmp(&right.props.accessible_name().to_lowercase())
                    .then_with(|| left.key.cmp(&right.key))
            });
            sorted.into_iter().map(|item| item.key.clone()).collect()
        }
        SortMode::Manual => manual_order(items, &config.order),
    }
}

fn manual_order(items: &[Item], order: &[String]) -> Vec<Key> {
    let position = |key: &Key| {
        order
            .iter()
            .position(|entry| entry == key.as_str())
            .unwrap_or(usize::MAX)
    };

    let mut sorted: Vec<&Item> = items.iter().collect();
    sorted.sort_by_key(|item| (position(&item.key),));
    sorted.into_iter().map(|item| item.key.clone()).collect()
}

/// Whether an item is shown at all: it must not be hidden by the user and it
/// must not be asking to be hidden (`Status == "Passive"`).
pub fn is_visible(item: &Item, config: &Config) -> bool {
    item.props.is_visible() && !config.is_hidden(item.key.as_str())
}

/// The order that results from moving `key` by `delta` places in the order the
/// user currently sees (`current`, e.g. from [`ordered_keys`]). Returns the
/// new manual order, or `None` when the move is a no-op (unknown key or moving
/// past either end).
pub fn moved_order(current: &[Key], key: &str, delta: i32) -> Option<Vec<String>> {
    let from = current.iter().position(|entry| entry.as_str() == key)?;
    let to = from as i64 + delta as i64;
    if to < 0 || to >= current.len() as i64 {
        return None;
    }

    let mut order: Vec<String> = current.iter().map(|entry| entry.to_string()).collect();
    let moved = order.remove(from);
    order.insert(to as usize, moved);
    Some(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sni::ItemProps;

    fn item(key: &str, title: &str) -> Item {
        Item {
            key: key.into(),
            service: crate::sni::ServiceRef {
                bus_name: format!(":{key}"),
                object_path: "/StatusNotifierItem".to_owned(),
            },
            props: ItemProps::default().with_property("Title", title.into()),
            menu: None,
        }
    }

    fn keys(items: &[Item], config: &Config) -> Vec<String> {
        ordered_keys(items, config)
            .into_iter()
            .map(|key| key.to_string())
            .collect()
    }

    fn items() -> Vec<Item> {
        vec![item("zeta", "Zeta"), item("alpha", "Alpha"), item("mid", "Mid")]
    }

    #[test]
    fn registration_order_is_the_default() {
        assert_eq!(keys(&items(), &Config::default()), ["zeta", "alpha", "mid"]);
    }

    #[test]
    fn title_order_sorts_by_visible_name() {
        let config = Config {
            sort: SortMode::Title,
            ..Config::default()
        };
        assert_eq!(keys(&items(), &config), ["alpha", "mid", "zeta"]);
    }

    #[test]
    fn manual_order_places_known_items_first() {
        let config = Config::default()
            .with_order(vec!["mid".into(), "zeta".into()]);
        assert_eq!(keys(&items(), &config), ["mid", "zeta", "alpha"]);
    }

    #[test]
    fn moved_order_swaps_with_a_neighbour() {
        let current = ["a", "b", "c"].map(Key::from);
        assert_eq!(moved_order(&current, "a", 1), Some(vec!["b".into(), "a".into(), "c".into()]));
        assert_eq!(moved_order(&current, "c", -1), Some(vec!["a".into(), "c".into(), "b".into()]));
    }

    #[test]
    fn moved_order_refuses_to_leave_the_cliff() {
        let current = ["a", "b"].map(Key::from);
        assert_eq!(moved_order(&current, "a", -1), None);
        assert_eq!(moved_order(&current, "b", 1), None);
        assert_eq!(moved_order(&current, "gone", 1), None);
    }

    #[test]
    fn hidden_items_disappear() {
        let items = items();
        let config = Config::default().with_hidden("alpha", true);
        assert_eq!(keys(&items, &config), ["zeta", "alpha", "mid"]);
        assert!(!is_visible(&items[1], &config));
        assert!(is_visible(&items[0], &config));
    }

    #[test]
    fn passive_items_disappear() {
        let mut items = items();
        items[0].props.status = crate::sni::Status::Passive;
        assert!(!is_visible(&items[0], &Config::default()));
    }
}
