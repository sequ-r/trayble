//! The state machine of the settings app.
//!
//! Pure by construction: [`update`] turns a message into the next [`State`]
//! plus the [`Call`]s that should be made, and nothing else happens here. The
//! GTK side only collects messages and carries out calls.

use taskbar_core::view::{ConfigView, ItemView, StatusView};

/// Everything the window shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// The daemon answered at least once.
    pub loaded: bool,
    pub status: Option<StatusView>,
    pub config: ConfigView,
    pub items: Vec<ItemView>,
    /// Last error worth showing, cleared by the next success.
    pub error: Option<String>,
}

impl State {
    pub fn is_hidden(&self, key: &str) -> bool {
        self.config.hidden.iter().any(|hidden| hidden == key)
    }

    /// The place of `key` in the displayed order.
    pub fn position(&self, key: &str) -> Option<usize> {
        self.items.iter().position(|item| item.key == key)
    }
}

/// A complete picture of the tray, as `ListAllItems` and friends report it:
/// this is the only thing that can replace what the window shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub status: StatusView,
    pub config: ConfigView,
    pub items: Vec<ItemView>,
}

/// Something happened: a reply arrived or the user asked for a change.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    /// A full picture arrived from the daemon.
    Loaded(Box<Snapshot>),
    Failed(String),
    /// The daemon announced a change; ask for a new picture.
    Refresh,

    // -- user intents ------------------------------------------------------

    SetIconSize(i32),
    SetSort(String),
    SetPanelBox(String),
    SetPanelPosition(i32),
    SetShowWhenEmpty(bool),
    SetItemHidden(String, bool),
    MoveItem(String, i32),
}

/// Something the app should do — all of it outside this module.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    Load,
    /// Hand a new configuration to the daemon, which persists it.
    SetConfig(ConfigView),
    /// Let the daemon move an item in the displayed order.
    MoveItem(String, i32),
}

/// Fold one message into the next state. Pure: the calls are returned, never
/// performed.
pub fn update(state: &State, msg: Msg) -> (State, Vec<Call>) {
    match msg {
        Msg::Loaded(snapshot) => (
            State {
                loaded: true,
                status: Some(snapshot.status),
                config: snapshot.config,
                items: snapshot.items,
                error: None,
            },
            Vec::new(),
        ),
        Msg::Failed(error) => (
            State {
                error: Some(error),
                ..state.clone()
            },
            Vec::new(),
        ),
        Msg::Refresh => (state.clone(), vec![Call::Load]),

        Msg::SetIconSize(icon_size) => {
            with_config(state, |config| config.icon_size = icon_size.clamp(8, 128))
        }
        Msg::SetSort(sort) => with_config(state, |config| config.sort = sort),
        Msg::SetPanelBox(panel_box) => with_config(state, |config| config.panel_box = panel_box),
        Msg::SetPanelPosition(position) => {
            with_config(state, |config| config.panel_position = position.clamp(0, 64))
        }
        Msg::SetShowWhenEmpty(show) => {
            with_config(state, |config| config.show_when_empty = show)
        }

        Msg::SetItemHidden(key, hidden) => with_config(state, move |config| {
            config.hidden.retain(|entry| entry != &key);
            if hidden {
                config.hidden.push(key);
            }
        }),

        // Ordering is the daemon's job (it owns the persisted order), so the
        // request is forwarded and the view is refreshed when it answers.
        Msg::MoveItem(key, delta) => (state.clone(), vec![Call::MoveItem(key, delta)]),
    }
}

/// Change the configuration, show the result at once and ask the daemon to
/// keep it. The daemon owns the config file, so this is the only write path.
fn with_config(state: &State, change: impl FnOnce(&mut ConfigView)) -> (State, Vec<Call>) {
    let mut config = state.config.clone();
    change(&mut config);
    let next = State {
        config: config.clone(),
        error: None,
        ..state.clone()
    };
    (next, vec![Call::SetConfig(config)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ConfigView {
        ConfigView {
            icon_size: 16,
            sort: "registered".into(),
            order: Vec::new(),
            hidden: Vec::new(),
            show_when_empty: false,
            panel_box: "right".into(),
            panel_position: 0,
        }
    }

    fn item(key: &str) -> ItemView {
        ItemView {
            key: key.into(),
            app_id: key.into(),
            title: key.to_owned(),
            ..ItemView::default()
        }
    }

    fn loaded() -> State {
        let (state, calls) = update(
            &State::default(),
            Msg::Loaded(Box::new(Snapshot {
                status: StatusView::default(),
                config: config(),
                items: vec![item("steam"), item("discord")],
            })),
        );
        assert!(calls.is_empty(), "loading needs no further calls");
        assert!(state.loaded);
        state
    }

    #[test]
    fn loading_replaces_the_whole_picture() {
        let state = loaded();
        assert_eq!(state.items.len(), 2);
        assert_eq!(state.position("discord"), Some(1));
    }

    #[test]
    fn changing_a_setting_is_reflected_at_once_and_persisted() {
        let state = loaded();
        let (next, calls) = update(&state, Msg::SetIconSize(24));

        assert_eq!(next.config.icon_size, 24, "the control moves at once");
        assert_eq!(
            calls,
            vec![Call::SetConfig(next.config.clone())],
            "the daemon keeps the config file"
        );
    }

    #[test]
    fn icon_size_is_bounded() {
        let state = loaded();
        assert_eq!(update(&state, Msg::SetIconSize(9999)).0.config.icon_size, 128);
        assert_eq!(update(&state, Msg::SetIconSize(1)).0.config.icon_size, 8);
    }

    #[test]
    fn hiding_an_item_adds_it_to_the_hidden_list() {
        let state = loaded();
        let (next, calls) = update(&state, Msg::SetItemHidden("steam".into(), true));
        assert!(next.is_hidden("steam"));
        assert_eq!(calls, vec![Call::SetConfig(next.config.clone())]);
        assert!(!next.is_hidden("discord"));

        let (next, _) = update(&next, Msg::SetItemHidden("steam".into(), false));
        assert!(!next.is_hidden("steam"), "unhiding removes the key again");
    }

    #[test]
    fn moving_an_item_is_forwarded_to_the_daemon() {
        let state = loaded();
        let (next, calls) = update(&state, Msg::MoveItem("steam".into(), 1));
        assert_eq!(next, state, "the daemon decides the order");
        assert_eq!(calls, vec![Call::MoveItem("steam".into(), 1)]);
    }

    #[test]
    fn refresh_asks_for_a_new_picture() {
        let (next, calls) = update(&loaded(), Msg::Refresh);
        assert_eq!(calls, vec![Call::Load]);
        assert!(next.loaded);
    }

    #[test]
    fn failures_are_kept_until_the_next_success() {
        let (state, _) = update(&loaded(), Msg::Failed("boom".into()));
        assert_eq!(state.error.as_deref(), Some("boom"));

        let (state, _) = update(
            &state,
            Msg::Loaded(Box::new(Snapshot {
                status: StatusView::default(),
                config: config(),
                items: Vec::new(),
            })),
        );
        assert_eq!(state.error, None);
    }
}
