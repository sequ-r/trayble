//! Client proxy for `org.kde.StatusNotifierItem`, the interface every tray
//! application exposes.

use zbus::proxy;

/// `org.kde.StatusNotifierItem`.
///
/// Properties are read in one go with `GetAll` and decoded by
/// [`crate::value`], because applications omit properties freely and a
/// failing getter must not take the whole item down.
#[proxy(
    interface = "org.kde.StatusNotifierItem",
    assume_defaults = false,
    gen_async = true,
    gen_blocking = false,
)]
pub trait StatusNotifierItem {
    /// Ask the item to act as if it were left-clicked.
    fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;

    /// Middle click.
    fn secondary_activate(&self, x: i32, y: i32) -> zbus::Result<()>;

    /// Ask the item to show its own context menu at (`x`, `y`).
    fn context_menu(&self, x: i32, y: i32) -> zbus::Result<()>;

    /// `"horizontal"` or `"vertical"`.
    fn scroll(&self, delta: i32, orientation: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    fn new_title(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn new_icon(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn new_attention_icon(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn new_overlay_icon(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn new_tool_tip(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn new_status(&self, status: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    fn new_icon_theme_path(&self, icon_theme_path: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    fn new_menu(&self) -> zbus::Result<()>;

    /// Ayatana extension, sent when `XAyatanaLabel` changes.
    #[zbus(signal, name = "NewLabel")]
    fn new_label(&self, label: &str, guide: &str) -> zbus::Result<()>;
}
