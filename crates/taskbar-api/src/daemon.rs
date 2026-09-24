//! Client proxy for `dev.taskbar.Daemon`, the API the GNOME Shell extension
//! and the settings app use. The daemon implements the same interface.

use taskbar_core::view::{ConfigView, ItemView, MenuView, StatusView};
use zbus::proxy;

/// Bus name of the daemon. D-Bus activation starts it on first use.
pub const DAEMON_BUS_NAME: &str = "dev.taskbar.Daemon";
/// Object path of the API.
pub const DAEMON_OBJECT_PATH: &str = "/dev/taskbar/Daemon";
/// Interface name of the API.
pub const DAEMON_INTERFACE: &str = "dev.taskbar.Daemon";

#[proxy(
    interface = "dev.taskbar.Daemon",
    default_service = "dev.taskbar.Daemon",
    default_path = "/dev/taskbar/Daemon",
    gen_async = true,
    gen_blocking = false,
)]
pub trait DaemonApi {
    /// Round-trip check.
    fn ping(&self) -> zbus::Result<String>;

    fn version(&self) -> zbus::Result<String>;

    /// Diagnostics for the settings app.
    fn get_status(&self) -> zbus::Result<StatusView>;

    /// The visible items, ordered, with icons resolved at
    /// `icon_pixel_size` device pixels.
    fn list_items(&self, icon_pixel_size: i32) -> zbus::Result<Vec<ItemView>>;

    /// Every registered item, including hidden and passive ones — what the
    /// settings app shows.
    fn list_all_items(&self, icon_pixel_size: i32) -> zbus::Result<Vec<ItemView>>;

    /// The menu of one item, flattened for display. Fetches the menu from the
    /// application first, so it is always current.
    fn get_menu(&self, key: &str) -> zbus::Result<MenuView>;

    /// Forward a menu interaction (`"clicked"`, `"opened"`, `"closed"`, ...).
    ///
    /// The dbusmenu `data` argument is deliberately not on this wire: both
    /// reference implementations send an empty value and packing variants in
    /// the JavaScript client is fragile. The daemon fills it in.
    fn menu_event(
        &self,
        key: &str,
        node_id: i32,
        event_id: &str,
        timestamp: u32,
    ) -> zbus::Result<()>;

    /// Left click: (`x`, `y`) are global pointer coordinates.
    fn activate(&self, key: &str, x: i32, y: i32) -> zbus::Result<()>;

    /// Middle click.
    fn secondary_activate(&self, key: &str, x: i32, y: i32) -> zbus::Result<()>;

    /// Ask the item to show its own context menu.
    fn context_menu(&self, key: &str, x: i32, y: i32) -> zbus::Result<()>;

    /// `"horizontal"` or `"vertical"`.
    fn scroll(&self, key: &str, delta: i32, orientation: &str) -> zbus::Result<()>;

    fn get_config(&self) -> zbus::Result<ConfigView>;

    /// Replace the configuration wholesale.
    fn set_config(&self, config: ConfigView) -> zbus::Result<()>;

    /// Hide or show one item without touching the rest of the config.
    fn set_item_hidden(&self, key: &str, hidden: bool) -> zbus::Result<()>;

    /// Move an item by `delta` places in the visible order.
    fn move_item(&self, key: &str, delta: i32) -> zbus::Result<()>;

    /// The visible items changed; call `ListItems` again.
    #[zbus(signal)]
    fn items_changed(&self, revision: u64) -> zbus::Result<()>;

    /// One item's menu changed; call `GetMenu` again.
    #[zbus(signal)]
    fn menu_changed(&self, key: &str, revision: u64) -> zbus::Result<()>;

    /// The configuration changed; call `GetConfig` again.
    #[zbus(signal)]
    fn config_changed(&self, revision: u64) -> zbus::Result<()>;

    /// Watcher state changed (name conflict resolved, ...).
    #[zbus(signal)]
    fn status_changed(&self) -> zbus::Result<()>;
}
