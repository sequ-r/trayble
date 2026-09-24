//! Client proxy for `org.kde.StatusNotifierWatcher`, the registry every tray
//! application announces itself to.

use zbus::proxy;

#[proxy(
    interface = "org.kde.StatusNotifierWatcher",
    default_service = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher",
    gen_async = true,
    gen_blocking = false,
)]
pub trait StatusNotifierWatcher {
    /// Register an item. `service` is a bus name, an object path on the
    /// caller's connection, or both separated by `/`.
    fn register_status_notifier_item(&self, service: &str) -> zbus::Result<()>;

    /// Register a tray host. A watcher will not consider items visible until
    /// a host is registered.
    fn register_status_notifier_host(&self, service: &str) -> zbus::Result<()>;

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> zbus::Result<Vec<String>>;

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn protocol_version(&self) -> zbus::Result<i32>;

    #[zbus(signal)]
    fn status_notifier_item_registered(&self, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    fn status_notifier_item_unregistered(&self, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    fn status_notifier_host_registered(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn status_notifier_host_unregistered(&self) -> zbus::Result<()>;
}
