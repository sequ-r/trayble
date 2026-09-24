//! taskbar-daemon: the tray itself.
//!
//! Everything interesting lives in `taskbar-core` (pure decisions) and
//! `taskbar-api` (the D-Bus contract). This binary wires them to a session
//! bus: it owns `org.kde.StatusNotifierWatcher` so applications announce
//! their tray icons, keeps their state in one [`Store`] and publishes the
//! result on `dev.taskbar.Daemon` for the panel indicator and the settings
//! app.

mod config_store;
mod item;
mod service;
mod store;
mod watcher;

use std::path::PathBuf;

use taskbar_api::daemon::{DAEMON_BUS_NAME, DAEMON_OBJECT_PATH};
use taskbar_core::sni::{WATCHER_BUS_NAME, WATCHER_OBJECT_PATH};
use zbus::fdo::RequestNameFlags;
use zbus::Connection;

use item::Registry;
use service::Service;
use store::Store;
use watcher::{Hosts, Watcher};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("taskbar_daemon=info")),
        )
        .init();

    let config_path = parse_args();
    let config = config_store::load(&config_path);
    tracing::info!(path = %config_path.display(), "configuration");

    let conn = Connection::session().await?;
    let store = Store::new(conn.clone(), config_path, config);
    let registry = Registry::new(conn.clone(), store.clone());
    let hosts = Hosts::new();

    // Publish the API first: clients may call it before any icon shows up.
    conn.object_server()
        .at(
            DAEMON_OBJECT_PATH,
            Service::new(store.clone(), registry.clone(), hosts.clone()),
        )
        .await?;

    // Then claim the watcher name, which is what makes applications show
    // their icons at all.
    match conn
        .request_name_with_flags(
            WATCHER_BUS_NAME,
            RequestNameFlags::ReplaceExisting | RequestNameFlags::DoNotQueue,
        )
        .await
    {
        Ok(reply) => {
            tracing::info!(name = WATCHER_BUS_NAME, reply = watcher::describe_name_reply(&reply), "watcher name");
            if reply != zbus::fdo::RequestNameReply::Exists {
                let watcher = Watcher::new(
                    conn.clone(),
                    store.clone(),
                    registry.clone(),
                    hosts.clone(),
                );
                watcher::register_self_as_host(&watcher).await;
                conn.object_server().at(WATCHER_OBJECT_PATH, watcher).await?;
            } else {
                let message = format!(
                    "another tray owns {WATCHER_BUS_NAME}; tray icons stay with \
                     its owner until it is stopped"
                );
                tracing::warn!("{message}");
                store.record_error(message);
            }
        }
        Err(error) => {
            tracing::warn!(%error, "cannot claim the watcher name");
            store.record_error(format!("cannot claim {WATCHER_BUS_NAME}: {error}"));
        }
    }

    conn.request_name(DAEMON_BUS_NAME).await?;
    tracing::info!(name = DAEMON_BUS_NAME, "ready");

    let name_watch = watcher::spawn_name_watch(registry.clone(), conn.clone());

    shutdown_signal().await;
    tracing::info!("shutting down");
    name_watch.abort();
    registry.shutdown();
    Ok(())
}

/// The config file, overridable for tests and sandboxed runs.
fn parse_args() -> PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                if let Some(path) = args.next() {
                    return PathBuf::from(path);
                }
            }
            "--version" => {
                println!("taskbar-daemon {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!(
                    "taskbar-daemon {}\n\n\
                     Usage: taskbar-daemon [--config PATH]\n\n\
                     Owns {WATCHER_BUS_NAME} and publishes {DAEMON_BUS_NAME}.",
                    env!("CARGO_PKG_VERSION")
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("taskbar-daemon: unknown argument '{other}' (try --help)");
                std::process::exit(2);
            }
        }
    }
    config_store::config_path()
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::warn!(%error, "cannot listen for signals, running until killed");
        std::future::pending::<()>().await;
    }
}
