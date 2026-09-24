//! The imperative shell around [`taskbar_core::update`].
//!
//! All state changes go through [`Store::apply`], which folds an [`Event`]
//! through the pure reducer and then carries out the returned [`Effect`]s:
//! emitting D-Bus signals and writing the config file. Nothing else in the
//! daemon mutates tray state.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use taskbar_api::daemon::{DAEMON_INTERFACE, DAEMON_OBJECT_PATH};
use taskbar_core::model::{Effect, Event, Key, State};
use taskbar_core::view::{ConfigView, ItemView, MenuView, StatusView};
use taskbar_core::{snapshot, update, Config, ServiceRef};
use zbus::Connection;

/// Shared tray state plus the two things side effects need: a connection to
/// announce changes on and a config file to write to.
pub struct Store {
    conn: Connection,
    config_path: PathBuf,
    state: Mutex<State>,
    last_error: Mutex<String>,
}

impl Store {
    pub fn new(conn: Connection, config_path: PathBuf, config: Config) -> Arc<Self> {
        Arc::new(Self {
            conn,
            config_path,
            state: Mutex::new(State {
                config,
                ..State::default()
            }),
            last_error: Mutex::new(String::new()),
        })
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Read only access to the state, for queries and diagnostics.
    pub fn with_state<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        let state = self.state.lock().expect("state lock");
        f(&state)
    }

    /// The visible items, ordered and rendered at `icon_pixel_size`.
    pub fn items(&self, icon_pixel_size: u32) -> Vec<ItemView> {
        self.with_state(|state| snapshot(state, icon_pixel_size))
    }

    pub fn config(&self) -> ConfigView {
        self.with_state(|state| state.config.to_view())
    }

    pub fn config_full(&self) -> Config {
        self.with_state(|state| state.config.clone())
    }

    pub fn revision(&self) -> u64 {
        self.with_state(|state| state.revision)
    }

    /// The cached menu of one item, if it has one.
    pub fn menu(&self, key: &str) -> Option<MenuView> {
        self.with_state(|state| {
            let item = state.item(key)?;
            let tree = item.menu.as_ref()?;
            Some(taskbar_core::view::flatten_menu(key, tree))
        })
    }

    /// Where an item lives on the bus, so callers can talk to it.
    pub fn service(&self, key: &str) -> Option<ServiceRef> {
        self.with_state(|state| state.item(key).map(|item| item.service.clone()))
    }

    /// The key an item registered with `service` was given.
    pub fn key_for_service(&self, service: &ServiceRef) -> Option<Key> {
        self.with_state(|state| {
            state
                .items
                .iter()
                .find(|item| item.service == *service)
                .map(|item| item.key.clone())
        })
    }

    /// The menu path of one item, if it has a menu.
    pub fn menu_path(&self, key: &str) -> Option<String> {
        self.with_state(|state| {
            let item = state.item(key)?;
            item.props
                .has_menu()
                .then(|| item.props.menu_path.clone())
        })
    }

    pub fn item_count(&self) -> usize {
        self.with_state(|state| state.items.len())
    }

    pub fn record_error(&self, error: impl Into<String>) {
        let mut last = self.last_error.lock().expect("error lock");
        *last = error.into();
    }

    pub fn take_error(&self) -> String {
        self.last_error.lock().expect("error lock").clone()
    }

    pub fn status(&self, version: &str, watcher: WatcherStatus) -> StatusView {
        StatusView {
            version: version.to_owned(),
            watcher_name: taskbar_core::sni::WATCHER_BUS_NAME.to_owned(),
            watcher_owned: watcher.owned,
            watcher_owner: watcher.owner,
            host_registered: watcher.host_registered,
            item_count: self.item_count() as i32,
            revision: self.revision(),
            config_path: self.config_path.display().to_string(),
            last_error: self.take_error(),
        }
    }

    /// Fold one event through the reducer and carry out its effects.
    pub async fn apply(&self, event: Event) -> zbus::Result<()> {
        let effects = {
            let mut state = self.state.lock().expect("state lock");
            let (next, effects) = update(std::mem::take(&mut *state), event);
            *state = next;
            effects
        };
        tracing::debug!(effects = %describe(&effects), "state updated");
        self.run_effects(effects).await
    }

    async fn run_effects(&self, effects: Vec<Effect>) -> zbus::Result<()> {
        for effect in effects {
            match effect {
                Effect::ItemsChanged { revision } => {
                    self.emit("ItemsChanged", &(revision,)).await?;
                }
                Effect::MenuChanged { key } => {
                    self.emit("MenuChanged", &(key.as_str(), self.revision())).await?;
                }
                Effect::ConfigChanged { revision } => {
                    self.emit("ConfigChanged", &(revision,)).await?;
                }
                Effect::PersistConfig(config) => {
                    let path = self.config_path.clone();
                    if let Err(error) = tokio::task::spawn_blocking(move || {
                        crate::config_store::save(&path, &config)
                    })
                    .await
                    .expect("config task")
                    {
                        let message = format!("cannot write {}: {error}", self.config_path.display());
                        tracing::warn!("{message}");
                        self.record_error(message);
                    }
                }
            }
        }
        Ok(())
    }

    async fn emit<B: serde::Serialize + zvariant::Type>(
        &self,
        name: &str,
        body: &B,
    ) -> zbus::Result<()> {
        self.conn
            .emit_signal(
                None::<&str>,
                DAEMON_OBJECT_PATH,
                DAEMON_INTERFACE,
                name,
                body,
            )
            .await
    }
}

/// What the caller needs to know about the watcher name, filled in by main.
#[derive(Clone, Debug, Default)]
pub struct WatcherStatus {
    pub owned: bool,
    pub owner: String,
    pub host_registered: bool,
}

/// Convenience for logging what the reducer decided.
pub fn describe(effects: &[Effect]) -> String {
    effects
        .iter()
        .map(|effect| match effect {
            Effect::ItemsChanged { revision } => format!("items changed (rev {revision})"),
            Effect::MenuChanged { key } => format!("menu changed ({key})"),
            Effect::ConfigChanged { .. } => "config changed".to_owned(),
            Effect::PersistConfig(_) => "config saved".to_owned(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
