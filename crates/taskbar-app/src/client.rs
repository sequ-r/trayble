//! The D-Bus side of the settings app.
//!
//! One worker thread owns the connection: it answers [`Call`]s with [`Msg`]s
//! and turns the daemon's change signals into [`Msg::Refresh`]. Messages are
//! delivered on the GTK main loop, so the UI thread never blocks on the bus
//! and never sees a D-Bus type.

use async_channel::Sender;
use futures_util::StreamExt;
use taskbar_api::daemon::DaemonApiProxy;
use tokio::sync::mpsc;

use crate::model::{Call, Msg, Snapshot};

/// Icons in the list are fetched at this size in device pixels: 16 logical
/// pixels on a normal screen, 32 on a HiDPI one.
const ICON_PIXEL_SIZE: i32 = 32;

/// A handle to the worker.
pub struct Client {
    calls: mpsc::UnboundedSender<Call>,
}

impl Client {
    /// Start the worker. Replies and change signals arrive on `messages`.
    pub fn start(messages: Sender<Msg>) -> Self {
        let (calls, incoming) = mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name("taskbar-dbus".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                runtime.block_on(run(incoming, messages));
            })
            .expect("start the D-Bus worker");
        Self { calls }
    }

    /// Ask the worker to do something. Fire and forget: the answer comes back
    /// as a [`Msg`].
    pub fn call(&self, call: Call) {
        if self.calls.send(call).is_err() {
            tracing::warn!("the D-Bus worker is gone");
        }
    }
}

async fn run(incoming: mpsc::UnboundedReceiver<Call>, messages: Sender<Msg>) {
    let mut incoming = incoming;
    let Ok(connection) = zbus::Connection::session().await else {
        deliver(&messages, Msg::Failed("no session bus".into()));
        return;
    };
    let daemon = match DaemonApiProxy::new(&connection).await {
        Ok(daemon) => daemon,
        Err(error) => {
            deliver(&messages, Msg::Failed(error.to_string()));
            return;
        }
    };

    // Change signals only say "ask again", so they all look the same here.
    let mut items_changed = daemon.receive_items_changed().await.ok();
    let mut config_changed = daemon.receive_config_changed().await.ok();
    let mut status_changed = daemon.receive_status_changed().await.ok();

    deliver(&messages, Msg::Refresh);
    loop {
        tokio::select! {
            call = incoming.recv() => {
                let Some(call) = call else { break };
                let msg = execute(&daemon, call).await;
                if !deliver(&messages, msg) {
                    break;
                }
            }
            _ = next(&mut items_changed) => {
                if !deliver(&messages, Msg::Refresh) {
                    break;
                }
            }
            _ = next(&mut config_changed) => {
                if !deliver(&messages, Msg::Refresh) {
                    break;
                }
            }
            _ = next(&mut status_changed) => {
                if !deliver(&messages, Msg::Refresh) {
                    break;
                }
            }
        }
    }
}

/// The next signal of an optional stream; a missing stream never fires.
async fn next<S: StreamExt + Unpin>(stream: &mut Option<S>) -> Option<S::Item> {
    match stream.as_mut() {
        Some(stream) => stream.next().await,
        None => std::future::pending().await,
    }
}

/// Hand a message to the GTK main loop. False means the window is gone.
fn deliver(messages: &Sender<Msg>, msg: Msg) -> bool {
    messages.send_blocking(msg).is_ok()
}

async fn execute(daemon: &DaemonApiProxy<'_>, call: Call) -> Msg {
    let result = match call {
        Call::Load => return load(daemon).await,
        Call::SetConfig(config) => daemon.set_config(config).await,
        Call::MoveItem(key, delta) => daemon.move_item(&key, delta).await,
    };
    match result {
        // Mutating calls are followed by a reload, so what the user sees
        // always matches what the daemon kept.
        Ok(()) => load(daemon).await,
        Err(error) => Msg::Failed(error.to_string()),
    }
}

/// Fetch the whole picture in one go: status, configuration and items.
async fn load(daemon: &DaemonApiProxy<'_>) -> Msg {
    let (status, config, items) = tokio::join!(
        daemon.get_status(),
        daemon.get_config(),
        daemon.list_all_items(ICON_PIXEL_SIZE),
    );

    match (status, config, items) {
        (Ok(status), Ok(config), Ok(items)) => {
            Msg::Loaded(Box::new(Snapshot {
                status,
                config,
                items,
            }))
        }
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
            Msg::Failed(error.to_string())
        }
    }
}
