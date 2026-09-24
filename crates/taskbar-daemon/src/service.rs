//! `dev.taskbar.Daemon`: the API the GNOME Shell extension and the settings
//! app talk to.
//!
//! It is intentionally small and stateless: `ListItems` returns a complete,
//! self-contained picture of the tray and the signals merely say "ask again".
//! Clients therefore never have to assemble state from deltas, which is what
//! keeps the JavaScript side of the panel indicator trivial.

use std::sync::Arc;

use taskbar_api::item::StatusNotifierItemProxy;
use taskbar_core::model::Event;
use taskbar_core::rules::{moved_order, ordered_keys};
use taskbar_core::view::{ConfigView, ItemView, MenuView, StatusView};
use taskbar_core::Config;
use zbus::fdo;
use zbus::object_server::SignalEmitter;
use zvariant::Value;

use crate::item::Registry;
use crate::store::{Store, WatcherStatus};
use crate::watcher::Hosts;

pub struct Service {
    store: Arc<Store>,
    registry: Arc<Registry>,
    hosts: Arc<Hosts>,
}

impl Service {
    pub fn new(store: Arc<Store>, registry: Arc<Registry>, hosts: Arc<Hosts>) -> Self {
        Self {
            store,
            registry,
            hosts,
        }
    }

    /// A live proxy for one item, or the error to answer with.
    async fn item(&self, key: &str) -> fdo::Result<StatusNotifierItemProxy<'static>> {
        self.registry
            .item_proxy(key)
            .await
            .ok_or_else(|| no_such_item(key))
    }

    async fn apply(&self, event: Event) -> fdo::Result<()> {
        self.store
            .apply(event)
            .await
            .map_err(|error| fdo::Error::Failed(error.to_string()))
    }

    /// A requested icon size, falling back to the configured one.
    fn icon_size(&self, icon_pixel_size: i32) -> u32 {
        if icon_pixel_size > 0 {
            icon_pixel_size as u32
        } else {
            self.store.config_full().icon_size
        }
    }
}

#[zbus::interface(name = "dev.taskbar.Daemon")]
impl Service {
    /// Round-trip check.
    async fn ping(&self) -> String {
        "pong".to_owned()
    }

    async fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_owned()
    }

    /// Diagnostics for the settings app: what the daemon owns, how many items
    /// are registered and what went wrong last.
    async fn get_status(&self) -> StatusView {
        let conn = self.store.connection();
        let our_name = conn
            .unique_name()
            .map(|name| name.to_string())
            .unwrap_or_default();
        let owner = match zbus::fdo::DBusProxy::new(conn).await {
            Ok(bus) => bus
                .get_name_owner(watcher_bus_name())
                .await
                .map(|name| name.to_string())
                .unwrap_or_default(),
            Err(_) => String::new(),
        };
        self.store.status(
            env!("CARGO_PKG_VERSION"),
            WatcherStatus {
                owned: owner == our_name && !our_name.is_empty(),
                owner,
                host_registered: !self.hosts.is_empty(),
            },
        )
    }

    /// The visible items, in order, with icons resolved at
    /// `icon_pixel_size` device pixels.
    async fn list_items(&self, icon_pixel_size: i32) -> Vec<ItemView> {
        self.store.items(self.icon_size(icon_pixel_size))
    }

    /// Every registered item, hidden and passive ones included.
    async fn list_all_items(&self, icon_pixel_size: i32) -> Vec<ItemView> {
        let size = self.icon_size(icon_pixel_size);
        self.store.with_state(|state| taskbar_core::snapshot_all(state, size))
    }

    /// The menu of one item, flattened for display. The application is asked
    /// first, so what comes back is always current.
    async fn get_menu(&self, key: &str) -> fdo::Result<MenuView> {
        if self.store.service(key).is_none() {
            return Err(no_such_item(key));
        }
        self.registry
            .refresh_menu(key)
            .await
            .or_else(|| self.store.menu(key))
            .ok_or_else(|| fdo::Error::Failed(format!("item '{key}' has no menu")))
    }

    /// Forward a menu interaction: `"clicked"`, `"opened"` or `"closed"`.
    async fn menu_event(
        &self,
        key: &str,
        node_id: i32,
        event_id: &str,
        data: Value<'_>,
        timestamp: u32,
    ) -> fdo::Result<()> {
        if self.store.service(key).is_none() {
            return Err(no_such_item(key));
        }
        self.registry
            .menu_event(key, node_id, event_id, &data, timestamp)
            .await;
        Ok(())
    }

    /// Left click: (`x`, `y`) are global pointer coordinates, which is what
    /// the StatusNotifierItem spec asks for.
    async fn activate(&self, key: &str, x: i32, y: i32) -> fdo::Result<()> {
        self.item(key)
            .await?
            .activate(x, y)
            .await
            .map_err(failed)
    }

    /// Middle click.
    async fn secondary_activate(&self, key: &str, x: i32, y: i32) -> fdo::Result<()> {
        self.item(key)
            .await?
            .secondary_activate(x, y)
            .await
            .map_err(failed)
    }

    /// Ask the item to show its own context menu at (`x`, `y`).
    async fn context_menu(&self, key: &str, x: i32, y: i32) -> fdo::Result<()> {
        self.item(key)
            .await?
            .context_menu(x, y)
            .await
            .map_err(failed)
    }

    /// `"horizontal"` or `"vertical"`.
    async fn scroll(&self, key: &str, delta: i32, orientation: &str) -> fdo::Result<()> {
        self.item(key)
            .await?
            .scroll(delta, orientation)
            .await
            .map_err(failed)
    }

    async fn get_config(&self) -> ConfigView {
        self.store.config()
    }

    /// Replace the configuration wholesale. It is normalised and persisted by
    /// the reducer.
    async fn set_config(&self, config: ConfigView) -> fdo::Result<()> {
        self.apply(Event::ConfigUpdated(Config::from_view(config))).await
    }

    /// Hide or show one item without touching the rest of the config.
    async fn set_item_hidden(&self, key: &str, hidden: bool) -> fdo::Result<()> {
        let config = self.store.config_full().with_hidden(key, hidden);
        self.apply(Event::ConfigUpdated(config)).await
    }

    /// Move an item by `delta` places in the order the user sees.
    async fn move_item(&self, key: &str, delta: i32) -> fdo::Result<()> {
        let (config, current) = self.store.with_state(|state| {
            (
                state.config.clone(),
                ordered_keys(&state.items, &state.config),
            )
        });
        let Some(order) = moved_order(&current, key, delta) else {
            return Ok(());
        };
        self.apply(Event::ConfigUpdated(config.with_order(order))).await
    }

    /// The visible items changed; call `ListItems` again.
    #[zbus(signal)]
    async fn items_changed(emitter: &SignalEmitter<'_>, revision: u64) -> zbus::Result<()>;

    /// One item's menu changed; call `GetMenu` again.
    #[zbus(signal)]
    async fn menu_changed(
        emitter: &SignalEmitter<'_>,
        key: &str,
        revision: u64,
    ) -> zbus::Result<()>;

    /// The configuration changed; call `GetConfig` again.
    #[zbus(signal)]
    async fn config_changed(emitter: &SignalEmitter<'_>, revision: u64) -> zbus::Result<()>;

    /// Watcher state changed, e.g. a name conflict was resolved.
    #[zbus(signal)]
    async fn status_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

fn watcher_bus_name() -> zbus::names::BusName<'static> {
    zbus::names::BusName::from_static_str(taskbar_core::sni::WATCHER_BUS_NAME)
        .expect("static bus name")
}

fn no_such_item(key: &str) -> fdo::Error {
    fdo::Error::Failed(format!("no such tray item: {key}"))
}

fn failed(error: impl std::fmt::Display) -> fdo::Error {
    fdo::Error::Failed(error.to_string())
}
