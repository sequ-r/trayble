//! Pure domain logic for the Taskbar system tray.
//!
//! Everything in this crate is a plain function over plain data: no I/O, no
//! D-Bus, no GTK, no async, no interior mutability. The daemon and the
//! settings app are thin imperative shells around this module: they turn
//! messages into [`Event`]s, fold them through [`update`] and then carry out
//! the returned [`Effect`]s. That keeps all decisions — what a tray icon is,
//! which pixmap to use, how items are ordered, what a menu node means — in
//! one place that can be unit tested without a session bus or a display.
//!
//! The [`view`] module holds the data contract shared with the GNOME Shell
//! extension and the settings app over D-Bus.

pub mod config;
pub mod icon;
pub mod menu;
pub mod model;
pub mod props;
pub mod rules;
pub mod sni;
pub mod view;

pub use config::{Config, PanelBox, SortMode};
pub use icon::Pixmap;
pub use model::{all_keys, snapshot, snapshot_all, update, update_all, Effect, Event, Item, Key, State};
pub use sni::ServiceRef;
pub use view::{ConfigView, IconView, ItemView, MenuNodeView, MenuView, StatusView};
