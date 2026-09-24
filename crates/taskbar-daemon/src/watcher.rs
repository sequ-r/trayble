//! `org.kde.StatusNotifierWatcher`, the registry every tray application looks
//! for when it starts.
//!
//! Owning this name is the whole point of the daemon: without it applications
//! such as Steam or Telegram silently fall back to showing a window instead of
//! a tray icon. When another desktop component already owns the name (Plasma,
//! for instance) the daemon keeps running but reports the conflict instead of
//! fighting over it.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use taskbar_core::sni::{HOST_INTERFACE, WATCHER_INTERFACE, WATCHER_OBJECT_PATH};
use taskbar_core::ServiceRef;
use zbus::fdo;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::Connection;

use crate::item::Registry;
use crate::store::Store;

/// The tray hosts registered with the watcher. We register ourselves so that
/// applications know their icons are visible.
pub struct Hosts(Mutex<BTreeSet<String>>);

impl Hosts {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(BTreeSet::new())))
    }

    /// Add a host, returning true when it was not known yet.
    pub fn add(&self, name: &str) -> bool {
        self.0.lock().expect("host set").insert(name.to_owned())
    }

    pub fn is_empty(&self) -> bool {
        self.0.lock().expect("host set").is_empty()
    }
}

pub struct Watcher {
    conn: Connection,
    store: Arc<Store>,
    registry: Arc<Registry>,
    hosts: Arc<Hosts>,
}

impl Watcher {
    pub fn new(
        conn: Connection,
        store: Arc<Store>,
        registry: Arc<Registry>,
        hosts: Arc<Hosts>,
    ) -> Self {
        Self {
            conn,
            store,
            registry,
            hosts,
        }
    }

    async fn emit<B: serde::Serialize + zvariant::Type>(&self, name: &str, body: &B) {
        emit(&self.conn, WATCHER_INTERFACE, name, body).await;
    }
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    /// Register a tray item. `service` is a bus name, an object path on the
    /// caller's connection, or both separated by `/`.
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        let caller = header
            .sender()
            .map(|sender| sender.to_string())
            .unwrap_or_default();
        let service = ServiceRef::resolve(service, &caller);
        tracing::debug!(?service, caller = %caller, "item registration");
        self.registry.register(service).await;
        Ok(())
    }

    /// Register a tray host. Items consider themselves visible once a host is
    /// registered; we are both host and watcher.
    async fn register_status_notifier_host(&self, service: &str) -> fdo::Result<()> {
        if self.hosts.add(service) {
            self.emit("StatusNotifierHostRegistered", &()).await;
        }
        Ok(())
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.store.with_state(|state| {
            state
                .items
                .iter()
                .map(|item| item.service.registration_id())
                .collect()
        })
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        !self.hosts.is_empty()
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_unregistered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// Emit a signal on the watcher interface, e.g. when an item registers
/// through the method above and other clients want to know.
pub async fn emit<B: serde::Serialize + zvariant::Type>(
    conn: &Connection,
    interface: &str,
    name: &str,
    body: &B,
) {
    if let Err(error) = conn
        .emit_signal(None::<&str>, WATCHER_OBJECT_PATH, interface, name, body)
        .await
    {
        tracing::debug!(%error, "cannot emit {name}");
    }
}

/// Announce a new item to everyone watching the registry.
pub async fn emit_item_registered(conn: &Connection, service: &ServiceRef) {
    announce(conn, "StatusNotifierItemRegistered", service).await;
}

/// Announce that an item is gone.
pub async fn emit_item_unregistered(conn: &Connection, service: &ServiceRef) {
    announce(conn, "StatusNotifierItemUnregistered", service).await;
}

async fn announce(conn: &Connection, signal: &str, service: &ServiceRef) {
    let name = service.registration_id();
    emit(conn, WATCHER_INTERFACE, signal, &(name,)).await;
}

/// Tell applications that a host is here by registering ourselves.
pub async fn register_self_as_host(watcher: &Watcher) {
    if let Err(error) = watcher.register_status_notifier_host(HOST_INTERFACE).await {
        tracing::warn!(%error, "cannot register as host");
    }
}

/// Watch `NameOwnerChanged` and drop the items of applications that leave the
/// bus or lose the name they registered with.
pub fn spawn_name_watch(registry: Arc<Registry>, conn: Connection) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let Ok(proxy) = fdo::DBusProxy::new(&conn).await else {
            return;
        };
        let Ok(mut changes) = proxy.receive_name_owner_changed().await else {
            return;
        };
        while let Some(signal) = changes.next().await {
            let Ok(args) = signal.args() else {
                continue;
            };
            // Both owners are optional: an empty owner means "gone".
            let owner = |value: &Option<zbus::names::UniqueName<'_>>| {
                value.as_ref().map(ToString::to_string).unwrap_or_default()
            };
            let (name, old_owner, new_owner) = (
                args.name().to_string(),
                owner(args.old_owner()),
                owner(args.new_owner()),
            );
            if !new_owner.is_empty() && new_owner == old_owner {
                continue;
            }
            tracing::debug!(%name, %old_owner, %new_owner, "bus name changed hands");
            registry.vanish_bus_name(&name).await;
        }
    })
}

/// The reply to a `RequestName` call, mapped to something worth logging.
pub fn describe_name_reply(reply: &fdo::RequestNameReply) -> &'static str {
    match reply {
        fdo::RequestNameReply::PrimaryOwner => "primary owner",
        fdo::RequestNameReply::InQueue => "queued behind another owner",
        fdo::RequestNameReply::Exists => "already owned by someone else",
        fdo::RequestNameReply::AlreadyOwner => "already ours",
    }
}
