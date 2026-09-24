//! The D-Bus side of the tray protocols: client proxies and the decoding of
//! the loosely typed property dictionaries both tray protocols are built on.
//!
//! Everything here converts between the wire and [`taskbar_core`]; no state is
//! kept and no decisions are made. The daemon and the settings app both build
//! on these types so the contract cannot drift between them.

pub mod daemon;
pub mod item;
pub mod menu;
pub mod value;
pub mod watcher;

pub use daemon::DaemonApiProxy;
pub use item::StatusNotifierItemProxy;
pub use menu::DBusMenuProxy;
pub use watcher::StatusNotifierWatcherProxy;
