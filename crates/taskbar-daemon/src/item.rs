//! Watching live tray items.
//!
//! One task per registered item keeps the [`Store`] in sync with the
//! application: it reads the item's properties, follows its menus and notices
//! when the application goes away. It is deliberately the only place that
//! talks `org.kde.StatusNotifierItem` and `com.canonical.dbusmenu`, so the
//! rest of the daemon only ever sees [`Event`]s.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use futures_util::StreamExt;
use taskbar_api::item::StatusNotifierItemProxy;
use taskbar_api::menu::{layout_to_tree, DBusMenuProxy, WANTED_PROPERTIES};
use taskbar_api::value::to_prop_map;
use taskbar_core::menu::{MenuTree, ROOT_ID};
use taskbar_core::model::{Event, Key};
use taskbar_core::view::MenuView;
use taskbar_core::sni::{ItemProps, ITEM_INTERFACE};
use taskbar_core::ServiceRef;
use zbus::fdo::PropertiesProxy;
use zbus::names::{BusName, InterfaceName};
use zvariant::{ObjectPath, Value};
use zbus::Connection;

use crate::store::Store;

/// How long to wait after a signal before reading properties. Applications
/// tend to announce an icon change several times in a row; one read is enough.
const COALESCE: Duration = Duration::from_millis(30);

/// How often to check that an item still answers. Some applications (Electron
/// in particular) export their item and later unexport it without closing
/// their bus name.
const PROBE_INTERVAL: Duration = Duration::from_secs(30);

/// Concurrent probes that must fail before an item is considered gone.
const PROBES_BEFORE_GONE: u8 = 2;

/// Bookkeeping for the items we currently watch.
pub struct Registry {
    conn: Connection,
    store: Arc<Store>,
    tasks: Mutex<HashMap<Key, tokio::task::JoinHandle<()>>>,
}

impl Registry {
    pub fn new(conn: Connection, store: Arc<Store>) -> Arc<Self> {
        Arc::new(Self {
            conn,
            store,
            tasks: Mutex::new(HashMap::new()),
        })
    }

    /// Start watching a newly registered item. Nothing happens when its
    /// properties cannot be read at all: a registration that is dead on
    /// arrival must not show up as a broken icon.
    pub async fn register(self: &Arc<Self>, service: ServiceRef) {
        let Some(props) = fetch_props(&self.conn, &service).await else {
            tracing::warn!(?service, "registered item does not answer, ignoring");
            return;
        };

        if let Err(error) = self
            .store
            .apply(Event::Registered {
                service: service.clone(),
                props,
            })
            .await
        {
            tracing::warn!(%error, "cannot register item");
            return;
        }

        let Some(key) = self.store.key_for_service(&service) else {
            return;
        };
        tracing::info!(%key, ?service, "tray item registered");

        let task = spawn_item_task(
            Arc::downgrade(self),
            self.conn.clone(),
            self.store.clone(),
            key.clone(),
            service.clone(),
        );
        if let Some(previous) = self.tasks.lock().expect("task map").insert(key.clone(), task) {
            previous.abort();
        }
        crate::watcher::emit_item_registered(&self.conn, &service).await;
    }

    /// Stop watching one item and drop it from the tray.
    pub async fn vanish_key(self: &Arc<Self>, key: &Key) {
        if let Some(task) = self.tasks.lock().expect("task map").remove(key) {
            task.abort();
        }
        let Some(service) = self.store.service(key.as_str()) else {
            return;
        };
        if let Err(error) = self.store.apply(Event::Vanished(key.clone())).await {
            tracing::warn!(%error, "cannot remove item");
        }
        tracing::info!(%key, "tray item gone");
        crate::watcher::emit_item_unregistered(&self.conn, &service).await;
    }

    /// Drop every item owned by `bus_name`, e.g. when the application exits.
    pub async fn vanish_bus_name(self: &Arc<Self>, bus_name: &str) {
        let keys: Vec<Key> = self
            .store
            .with_state(|state| {
                state
                    .items
                    .iter()
                    .filter(|item| item.service.bus_name == bus_name)
                    .map(|item| item.key.clone())
                    .collect()
            });
        for key in keys {
            self.vanish_key(&key).await;
        }
    }

    /// Fetch the menu of an item from its application, asking it first
    /// whether anything changed (`AboutToShow`), and cache the result.
    pub async fn refresh_menu(&self, key: &str) -> Option<MenuView> {
        let service = self.store.service(key)?;
        let menu_path = self.store.menu_path(key)?;
        let proxy = menu_proxy(&self.conn, &service, &menu_path).await?;

        if let Err(error) = proxy.about_to_show(ROOT_ID).await {
            tracing::debug!(%key, %error, "AboutToShow failed");
        }

        let tree = fetch_menu(&proxy).await?;
        self.store
            .apply(Event::MenuUpdated {
                key: key.into(),
                menu: Some(tree),
            })
            .await
            .ok()?;
        self.store.menu(key)
    }

    /// Open, close or click part of the menu from the application's point of
    /// view.
    pub async fn menu_event(
        &self,
        key: &str,
        node_id: i32,
        event_id: &str,
        data: &Value<'_>,
        timestamp: u32,
    ) {
        let (Some(service), Some(menu_path)) = (self.store.service(key), self.store.menu_path(key))
        else {
            return;
        };
        let Some(proxy) = menu_proxy(&self.conn, &service, &menu_path).await else {
            return;
        };
        if let Err(error) = proxy.event(node_id, event_id, data, timestamp).await {
            tracing::debug!(%key, %error, "menu event failed");
        }
    }

    /// The item proxy of one entry, rebuilt on demand: clicks are rare, so
    /// caching proxies would only add bookkeeping.
    pub async fn item_proxy(&self, key: &str) -> Option<StatusNotifierItemProxy<'static>> {
        let service = self.store.service(key)?;
        let (destination, path) = names(&service)?;
        StatusNotifierItemProxy::builder(&self.conn)
            .destination(destination)
            .ok()?
            .path(path)
            .ok()?
            .build()
            .await
            .ok()
    }

    /// Drop all tasks, e.g. on shutdown.
    pub fn shutdown(&self) {
        for task in self.tasks.lock().expect("task map").values() {
            task.abort();
        }
    }
}

/// Names are converted to owned form once: proxies outlive the strings they
/// were built from that way.
fn names(service: &ServiceRef) -> Option<(BusName<'static>, ObjectPath<'static>)> {
    Some((
        BusName::try_from(service.bus_name.clone()).ok()?,
        ObjectPath::try_from(service.object_path.clone()).ok()?,
    ))
}

async fn properties_proxy(conn: &Connection, service: &ServiceRef) -> Option<PropertiesProxy<'static>> {
    let (destination, path) = names(service)?;
    PropertiesProxy::builder(conn)
        .destination(destination)
        .ok()?
        .path(path)
        .ok()?
        .build()
        .await
        .ok()
}

async fn menu_proxy(
    conn: &Connection,
    service: &ServiceRef,
    menu_path: &str,
) -> Option<DBusMenuProxy<'static>> {
    let (destination, _) = names(service)?;
    let path = ObjectPath::try_from(menu_path.to_owned()).ok()?;
    DBusMenuProxy::builder(conn)
        .destination(destination)
        .ok()?
        .path(path)
        .ok()?
        .build()
        .await
        .ok()
}

/// Read all `org.kde.StatusNotifierItem` properties of one item.
async fn fetch_props(conn: &Connection, service: &ServiceRef) -> Option<ItemProps> {
    let proxy = properties_proxy(conn, service).await?;
    let interface = InterfaceName::from_static_str(ITEM_INTERFACE).expect("static interface name");
    match proxy.get_all(interface).await {
        Ok(properties) => Some(ItemProps::from_map(to_prop_map(&properties))),
        Err(error) => {
            tracing::debug!(?service, %error, "cannot read item properties");
            None
        }
    }
}

async fn fetch_menu(proxy: &DBusMenuProxy<'_>) -> Option<MenuTree> {
    match proxy.get_layout(ROOT_ID, -1, WANTED_PROPERTIES).await {
        Ok((revision, layout)) => Some(layout_to_tree(revision, &layout)),
        Err(error) => {
            tracing::debug!(%error, "cannot read menu layout");
            None
        }
    }
}

/// The life of one watched item: follow its properties and menu until it goes
/// away, then drop it from the tray.
fn spawn_item_task(
    registry: Weak<Registry>,
    conn: Connection,
    store: Arc<Store>,
    key: Key,
    service: ServiceRef,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let Some(registry) = registry.upgrade() else {
            return;
        };

        let Some(item) = registry.item_proxy(key.as_str()).await else {
            tracing::warn!(%key, ?service, "cannot reach item");
            registry.vanish_key(&key).await;
            return;
        };
        let Some(properties) = properties_proxy(&conn, &service).await else {
            tracing::warn!(%key, ?service, "cannot reach item properties");
            registry.vanish_key(&key).await;
            return;
        };

        let mut item_signals = match item.inner().receive_all_signals().await {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!(%key, %error, "cannot watch item signals");
                registry.vanish_key(&key).await;
                return;
            }
        };
        let mut property_signals = match properties.receive_properties_changed().await {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!(%key, %error, "cannot watch property signals");
                registry.vanish_key(&key).await;
                return;
            }
        };

        // The menu, if any, is kept fresh by a task of its own so that this
        // loop does not have to swap streams around when the menu path
        // changes.
        let mut menu_task = spawn_menu_task(
            &registry,
            conn.clone(),
            store.clone(),
            key.clone(),
            service.clone(),
            store.menu_path(key.as_str()),
        );
        let mut menu_path = store.menu_path(key.as_str());

        let mut probe = tokio::time::interval(PROBE_INTERVAL);
        probe.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut failed_probes = 0u8;

        loop {
            tokio::select! {
                _ = item_signals.next() => {
                    tokio::time::sleep(COALESCE).await;
                    if !refresh_props(&registry, &conn, &store, &key, &service, &mut menu_task, &mut menu_path).await {
                        break;
                    }
                    failed_probes = 0;
                }
                _ = property_signals.next() => {
                    tokio::time::sleep(COALESCE).await;
                    if !refresh_props(&registry, &conn, &store, &key, &service, &mut menu_task, &mut menu_path).await {
                        break;
                    }
                    failed_probes = 0;
                }
                _ = probe.tick() => {
                    match fetch_props(&conn, &service).await {
                        Some(props) => {
                            failed_probes = 0;
                            if let Err(error) = store.apply(Event::PropsUpdated { key: key.clone(), props }).await {
                                tracing::warn!(%key, %error, "cannot update item");
                            }
                        }
                        None => {
                            failed_probes += 1;
                            if failed_probes >= PROBES_BEFORE_GONE {
                                tracing::info!(%key, "item stopped answering");
                                break;
                            }
                        }
                    }
                }
            }
        }

        menu_task.abort();
        registry.vanish_key(&key).await;
    })
}

/// Re-read the properties of an item and follow menu changes. Returns false
/// when the item is gone.
async fn refresh_props(
    registry: &Arc<Registry>,
    conn: &Connection,
    store: &Arc<Store>,
    key: &Key,
    service: &ServiceRef,
    menu_task: &mut tokio::task::JoinHandle<()>,
    menu_path: &mut Option<String>,
) -> bool {
    let Some(props) = fetch_props(conn, service).await else {
        return false;
    };
    if let Err(error) = store
        .apply(Event::PropsUpdated {
            key: key.clone(),
            props,
        })
        .await
    {
        tracing::warn!(%key, %error, "cannot update item");
    }

    // The application may have swapped its menu for another one.
    let new_path = store.menu_path(key.as_str());
    if new_path != *menu_path {
        *menu_path = new_path.clone();
        menu_task.abort();
        *menu_task = spawn_menu_task(registry, conn.clone(), store.clone(), key.clone(), service.clone(), new_path);
    }
    true
}

/// Keep the cached menu of one item up to date.
fn spawn_menu_task(
    registry: &Arc<Registry>,
    conn: Connection,
    store: Arc<Store>,
    key: Key,
    service: ServiceRef,
    menu_path: Option<String>,
) -> tokio::task::JoinHandle<()> {
    let registry = Arc::downgrade(registry);
    tokio::spawn(async move {
        let Some(registry) = registry.upgrade() else {
            return;
        };
        let Some(menu_path) = menu_path else {
            return;
        };
        let Some(proxy) = menu_proxy(&conn, &service, &menu_path).await else {
            return;
        };
        let Ok(mut signals) = proxy.inner().receive_all_signals().await else {
            return;
        };

        while signals.next().await.is_some() {
            tokio::time::sleep(COALESCE).await;
            let Some(tree) = fetch_menu(&proxy).await else {
                continue;
            };
            if let Err(error) = store
                .apply(Event::MenuUpdated {
                    key: key.clone(),
                    menu: Some(tree),
                })
                .await
            {
                tracing::debug!(%key, %error, "cannot store menu");
                break;
            }
        }
        drop(registry);
    })
}
