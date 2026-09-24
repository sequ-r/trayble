//! The state machine of the tray: what is registered, what is configured and
//! what should happen next.
//!
//! [`update`] is the only place state changes. It is a pure fold over
//! [`Event`]s that returns the next [`State`] and a list of [`Effect`]s —
//! side effects like emitting D-Bus signals or saving the config file are
//! carried out by the caller. That keeps every decision in the tray testable
//! without a session bus.

use crate::config::Config;
use crate::menu::MenuTree;
use crate::sni::{ItemProps, ServiceRef};
use crate::view::ItemView;

/// Stable identifier of a tray item, used in the panel, in the settings app
/// and in the config file. Derived from the application's `Id` so that it
/// survives the application restarting; uniquified when two items collide.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key(String);

impl Key {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for Key {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Key {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// A registered tray item: where it lives, what it says about itself and its
/// menu, if it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub key: Key,
    pub service: ServiceRef,
    pub props: ItemProps,
    pub menu: Option<MenuTree>,
}

/// The whole tray: configuration plus the registered items in registration
/// order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub config: Config,
    pub items: Vec<Item>,
    /// Bumped whenever the visible content changes; used by clients to skip
    /// stale updates.
    pub revision: u64,
}

impl State {
    pub fn item(&self, key: &str) -> Option<&Item> {
        self.items.iter().find(|item| item.key.as_str() == key)
    }

    pub fn item_mut(&mut self, key: &str) -> Option<&mut Item> {
        self.items.iter_mut().find(|item| item.key.as_str() == key)
    }

    /// A key derived from `preferred` that no current item uses.
    pub fn allocate_key(&self, preferred: &str) -> Key {
        let base = sanitize_key(preferred);
        if self.item(&base).is_none() {
            return Key::new(base);
        }
        (2..)
            .map(|suffix| format!("{base}#{suffix}"))
            .find(|candidate| self.item(candidate).is_none())
            .map(Key::new)
            .unwrap_or_else(|| Key::new(base))
    }
}

/// Something that happened to the tray.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// An application registered a tray item.
    Registered { service: ServiceRef, props: ItemProps },
    /// Its bus name went away or it stopped answering.
    Vanished(Key),
    /// It published new properties (a new icon, a new status, ...).
    PropsUpdated { key: Key, props: ItemProps },
    /// Its menu changed.
    MenuUpdated { key: Key, menu: Option<MenuTree> },
    /// The user changed the configuration.
    ConfigUpdated(Config),
}

/// A side effect for the caller to carry out.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// The list of visible items changed; clients should call `ListItems`.
    ItemsChanged { revision: u64 },
    /// One item's menu changed.
    MenuChanged { key: Key },
    /// The configuration changed; clients should call `GetConfig`.
    ConfigChanged { revision: u64 },
    /// The configuration should be written to disk.
    PersistConfig(Config),
}

/// Fold one event into the state. Pure: no I/O, no signalling.
pub fn update(state: State, event: Event) -> (State, Vec<Effect>) {
    match event {
        Event::Registered { service, props } => register(state, service, props),
        Event::Vanished(key) => vanish(state, key),
        Event::PropsUpdated { key, props } => update_props(state, key, props),
        Event::MenuUpdated { key, menu } => update_menu(state, key, menu),
        Event::ConfigUpdated(config) => update_config(state, config),
    }
}

/// Fold a sequence of events, e.g. everything that happened while the
/// settings app was closed.
pub fn update_all(state: State, events: impl IntoIterator<Item = Event>) -> (State, Vec<Effect>) {
    events
        .into_iter()
        .fold((state, Vec::new()), |(state, mut effects), event| {
            let (state, new_effects) = update(state, event);
            effects.extend(new_effects);
            (state, effects)
        })
}

fn register(mut state: State, service: ServiceRef, props: ItemProps) -> (State, Vec<Effect>) {
    // Re-registration is how applications update their identity after a
    // restart; drop the previous incarnation first so it cannot linger.
    state.items.retain(|item| item.service != service);

    let preferred = if props.id.is_empty() {
        props.title.as_str()
    } else {
        props.id.as_str()
    };
    let key = state.allocate_key(preferred);
    state.items.push(Item {
        key,
        service,
        props,
        menu: None,
    });
    announce_items(state)
}

fn vanish(mut state: State, key: Key) -> (State, Vec<Effect>) {
    let before = state.items.len();
    state.items.retain(|item| item.key != key);
    if state.items.len() == before {
        return (state, Vec::new());
    }
    announce_items(state)
}

fn update_props(mut state: State, key: Key, props: ItemProps) -> (State, Vec<Effect>) {
    let Some(item) = state.item_mut(key.as_str()) else {
        return (state, Vec::new());
    };
    if item.props == props {
        return (state, Vec::new());
    }
    item.props = props;
    announce_items(state)
}

fn update_menu(mut state: State, key: Key, menu: Option<MenuTree>) -> (State, Vec<Effect>) {
    let Some(item) = state.item_mut(key.as_str()) else {
        return (state, Vec::new());
    };
    if item.menu == menu {
        return (state, Vec::new());
    }
    item.menu = menu;
    // The icon strip is unchanged, so only the menu needs announcing.
    (state, vec![Effect::MenuChanged { key }])
}

fn update_config(state: State, config: Config) -> (State, Vec<Effect>) {
    let config = config.normalized();
    let config_changed = state.config != config;
    let (mut state, mut effects) = announce_items(state);
    state.config = config;

    if config_changed {
        effects.push(Effect::ConfigChanged {
            revision: state.revision,
        });
        effects.push(Effect::PersistConfig(state.config.clone()));
    }
    (state, effects)
}

/// Bump the revision and announce that the visible items may have changed.
fn announce_items(mut state: State) -> (State, Vec<Effect>) {
    state.revision += 1;
    let effects = vec![Effect::ItemsChanged {
        revision: state.revision,
    }];
    (state, effects)
}

/// The items to show, in order, rendered at `icon_pixel_size` device pixels.
/// Hidden and passive items are left out entirely.
pub fn snapshot(state: &State, icon_pixel_size: u32) -> Vec<ItemView> {
    let order = crate::rules::ordered_keys(&state.items, &state.config);
    order
        .into_iter()
        .filter_map(|key| {
            let item = state.item(key.as_str())?;
            crate::rules::is_visible(item, &state.config).then(|| ItemView::of(item, icon_pixel_size))
        })
        .collect()
}

/// Every registered item, in display order, including the hidden and passive
/// ones: this is the picture the settings app needs to manage the tray.
pub fn snapshot_all(state: &State, icon_pixel_size: u32) -> Vec<ItemView> {
    crate::rules::ordered_keys(&state.items, &state.config)
        .into_iter()
        .filter_map(|key| state.item(key.as_str()))
        .map(|item| ItemView::of(item, icon_pixel_size))
        .collect()
}

/// Item keys a user could hide or reorder, including the hidden ones.
pub fn all_keys(state: &State) -> Vec<Key> {
    crate::rules::ordered_keys(&state.items, &state.config)
}

/// Turn a user facing id into a config-safe key.
fn sanitize_key(preferred: &str) -> String {
    let trimmed: String = preferred
        .trim()
        .chars()
        .map(|ch| if ch.is_control() { '_' } else { ch })
        .collect();
    if trimmed.is_empty() {
        "item".to_owned()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sni::ItemProps;

    fn service(name: &str) -> ServiceRef {
        ServiceRef {
            bus_name: name.to_owned(),
            object_path: "/StatusNotifierItem".to_owned(),
        }
    }

    fn props(id: &str) -> ItemProps {
        ItemProps::default().with_property("Id", id.into())
    }

    fn registered(state: State, name: &str, id: &str) -> (State, Vec<Effect>) {
        update(
            state,
            Event::Registered {
                service: service(name),
                props: props(id),
            },
        )
    }

    #[test]
    fn registering_bumps_the_revision_and_emits() {
        let (state, effects) = registered(State::default(), ":1.1", "steam");
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].key.as_str(), "steam");
        assert_eq!(state.revision, 1);
        assert_eq!(effects, vec![Effect::ItemsChanged { revision: 1 }]);
    }

    #[test]
    fn colliding_ids_get_unique_keys() {
        let (state, _) = registered(State::default(), ":1.1", "app");
        let (state, _) = registered(state, ":1.2", "app");
        let keys: Vec<_> = state.items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, vec!["app", "app#2"]);
    }

    #[test]
    fn empty_ids_fall_back_to_the_title_or_a_placeholder() {
        let (state, _) = registered(State::default(), ":1.1", "");
        assert_eq!(state.items[0].key.as_str(), "item");
    }

    #[test]
    fn vanishing_items_are_removed_once() {
        let (state, _) = registered(State::default(), ":1.1", "steam");
        let key = state.items[0].key.clone();

        let (state, effects) = update(state, Event::Vanished(key.clone()));
        assert!(state.items.is_empty());
        assert_eq!(effects, vec![Effect::ItemsChanged { revision: 2 }]);

        let (state, effects) = update(state, Event::Vanished(key));
        assert!(state.items.is_empty(), "idempotent");
        assert!(effects.is_empty(), "no news is no effect");
    }

    #[test]
    fn unchanged_properties_are_not_announced() {
        let (state, _) = registered(State::default(), ":1.1", "steam");
        let key = state.items[0].key.clone();

        let (state, effects) = update(
            state,
            Event::PropsUpdated {
                key: key.clone(),
                props: props("steam"),
            },
        );
        assert_eq!(state.revision, 1);
        assert!(effects.is_empty());

        let (state, effects) = update(
            state,
            Event::PropsUpdated {
                key,
                props: props("steam").with_property("Title", "Steam".into()),
            },
        );
        assert_eq!(state.revision, 2);
        assert_eq!(effects, vec![Effect::ItemsChanged { revision: 2 }]);
    }

    #[test]
    fn menu_updates_only_announce_the_menu() {
        let (state, _) = registered(State::default(), ":1.1", "steam");
        let key = state.items[0].key.clone();

        let (state, effects) = update(
            state,
            Event::MenuUpdated {
                key: key.clone(),
                menu: Some(MenuTree {
                    revision: 1,
                    ..MenuTree::default()
                }),
            },
        );
        assert_eq!(effects, vec![Effect::MenuChanged { key }]);
        assert_eq!(state.revision, 1, "the icon strip did not change");
    }

    #[test]
    fn re_registration_replaces_the_old_incarnation() {
        let (state, _) = registered(State::default(), ":1.1", "steam");
        let (state, _) = registered(state, ":1.1", "steam");
        assert_eq!(state.items.len(), 1);
    }

    #[test]
    fn config_changes_are_normalised_persisted_and_announced() {
        let (state, effects) = update(
            State::default(),
            Event::ConfigUpdated(Config {
                icon_size: 999,
                hidden: vec!["steam".into()],
                ..Config::default()
            }),
        );

        assert_eq!(state.config.icon_size, 128);
        assert!(state.config.is_hidden("steam"));
        assert!(
            effects.contains(&Effect::PersistConfig(state.config.clone())),
            "the config is written back: {effects:?}"
        );
        assert!(
            effects.iter().any(|e| matches!(e, Effect::ConfigChanged { .. })),
            "clients are told: {effects:?}"
        );
    }

    #[test]
    fn snapshot_skips_hidden_and_passive_items() {
        let (state, _) = registered(State::default(), ":1.1", "steam");
        let (state, _) = registered(state, ":1.2", "discord");
        let (mut state, _) = registered(state, ":1.3", "hidden-app");

        state.item_mut("hidden-app").unwrap().props.status = crate::sni::Status::Passive;
        state.config = state.config.with_hidden("discord", true);

        let view = snapshot(&state, 22);
        let keys: Vec<_> = view.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, vec!["steam"]);
    }

    #[test]
    fn update_all_folds_a_batch() {
        let (state, effects) = update_all(
            State::default(),
            [
                Event::Registered {
                    service: service(":1.1"),
                    props: props("steam"),
                },
                Event::Registered {
                    service: service(":1.2"),
                    props: props("discord"),
                },
            ],
        );
        assert_eq!(state.items.len(), 2);
        assert_eq!(effects.len(), 2);
    }
}
