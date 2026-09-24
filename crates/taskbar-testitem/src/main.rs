//! `taskbar-testitem`: a fake tray application.
//!
//! Registers a StatusNotifierItem complete with pixmaps, a tooltip and a
//! `com.canonical.dbusmenu` menu, so the whole tray pipeline — daemon, panel
//! indicator, settings app — can be exercised without hunting for a real
//! application that happens to have a tray icon.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use taskbar_api::menu::{MenuLayout, PropertyMap};
use taskbar_core::sni::{ITEM_INTERFACE, WATCHER_BUS_NAME, WATCHER_OBJECT_PATH};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedValue, Structure, Value};
use zbus::Connection;

const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";

const NODE_NOTIFICATION: i32 = 1;
const NODE_CHECKBOX: i32 = 2;
const NODE_SEPARATOR: i32 = 3;
const NODE_QUIT: i32 = 4;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let options = Options::parse();
    tracing::info!(id = %options.id, "starting fake tray item");

    let conn = Connection::session().await?;
    let frames = Arc::new(icon_frames());
    let checked = Arc::new(AtomicBool::new(true));
    let frame = Arc::new(AtomicUsize::new(0));
    let status = Arc::new(AtomicUsize::new(0));

    let item = Item {
        id: options.id.clone(),
        title: options.title.clone(),
        icon_name: options.icon_name.clone(),
        frames: frames.clone(),
        frame: frame.clone(),
        status: status.clone(),
    };
    let menu = Menu {
        checked: checked.clone(),
        revision: AtomicUsize::new(1),
    };

    conn.object_server().at(ITEM_PATH, item).await?;
    conn.object_server().at(MENU_PATH, menu).await?;

    // Announce ourselves to the watcher. The bare path spelling is what most
    // applications send: the item lives on our own connection.
    watcher(&conn)
        .await?
        .register_status_notifier_item(ITEM_PATH)
        .await?;
    tracing::info!("registered with {WATCHER_BUS_NAME}");

    let mut jobs = tokio::task::JoinSet::new();
    if let Some(seconds) = options.animate {
        let conn = conn.clone();
        let frames = frames.clone();
        let frame = frame.clone();
        jobs.spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs_f64(seconds));
            loop {
                ticker.tick().await;
                let next = (frame.load(Ordering::Relaxed) + 1) % frames.len();
                frame.store(next, Ordering::Relaxed);
                tracing::info!(frame = next, "changing icon");
                emit(&conn, ITEM_PATH, ITEM_INTERFACE, "NewIcon", &()).await;
            }
        });
    }
    if let Some(seconds) = options.flip_status {
        let conn = conn.clone();
        let status = status.clone();
        jobs.spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs_f64(seconds));
            loop {
                ticker.tick().await;
                let next = (status.load(Ordering::Relaxed) + 1) % 3;
                status.store(next, Ordering::Relaxed);
                let name = ["Active", "NeedsAttention", "Passive"][next];
                tracing::info!(status = name, "changing status");
                emit(&conn, ITEM_PATH, ITEM_INTERFACE, "NewStatus", &(name,)).await;
            }
        });
    }

    tracing::info!("running; middle click the tray icon to quit");
    tokio::signal::ctrl_c().await?;
    jobs.abort_all();
    Ok(())
}

struct Options {
    id: String,
    title: String,
    icon_name: String,
    animate: Option<f64>,
    flip_status: Option<f64>,
}

impl Options {
    fn parse() -> Self {
        let mut options = Self {
            id: "taskbar-testitem".to_owned(),
            title: "Test Item".to_owned(),
            icon_name: String::new(),
            animate: None,
            flip_status: None,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut value = |name: &str| {
                args.next()
                    .unwrap_or_else(|| die(&format!("{name} needs a value")))
            };
            match arg.as_str() {
                "--id" => options.id = value("--id"),
                "--title" => options.title = value("--title"),
                "--icon-name" => options.icon_name = value("--icon-name"),
                "--animate" => options.animate = Some(parse_number(&value("--animate"), "--animate")),
                "--flip-status" => {
                    options.flip_status = Some(parse_number(&value("--flip-status"), "--flip-status"))
                }
                "--help" | "-h" => {
                    println!(
                        "taskbar-testitem {}\n\n\
                         A fake StatusNotifierItem for testing the tray.\n\n\
                         Usage: taskbar-testitem [--id ID] [--title TEXT] [--icon-name NAME]\n\
                                [--animate SECONDS] [--flip-status SECONDS]",
                        env!("CARGO_PKG_VERSION")
                    );
                    std::process::exit(0);
                }
                other => die(&format!("unknown argument '{other}' (try --help)")),
            }
        }
        options
    }
}

fn parse_number(text: &str, name: &str) -> f64 {
    text.parse()
        .unwrap_or_else(|_| die(&format!("{name} wants a number, got '{text}'")))
}

fn die(message: &str) -> ! {
    eprintln!("taskbar-testitem: {message}");
    std::process::exit(2)
}

/// The tray item itself.
struct Item {
    id: String,
    title: String,
    icon_name: String,
    frames: Arc<Vec<IconFrame>>,
    frame: Arc<AtomicUsize>,
    status: Arc<AtomicUsize>,
}

type IconFrame = Vec<(i32, i32, Vec<u8>)>;

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        &self.id
    }

    #[zbus(property)]
    fn title(&self) -> &str {
        &self.title
    }

    #[zbus(property)]
    fn status(&self) -> &'static str {
        ["Active", "NeedsAttention", "Passive"][self.status.load(Ordering::Relaxed)]
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
        &self.icon_name
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> IconFrame {
        self.frames[self.frame.load(Ordering::Relaxed)].clone()
    }

    #[zbus(property)]
    fn attention_icon_pixmap(&self) -> IconFrame {
        self.frames[(self.frame.load(Ordering::Relaxed) + 1) % self.frames.len()].clone()
    }

    #[zbus(property)]
    fn overlay_icon_pixmap(&self) -> IconFrame {
        self.frames[(self.frame.load(Ordering::Relaxed) + 2) % self.frames.len()].clone()
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> &str {
        ""
    }

    #[zbus(property)]
    fn menu(&self) -> &str {
        MENU_PATH
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn tool_tip(&self) -> (String, IconFrame, String, String) {
        (
            String::new(),
            Vec::new(),
            self.title.clone(),
            format!("A fake tray item ({}).", self.id),
        )
    }

    #[zbus(property)]
    fn icon_accessible_desc(&self) -> &str {
        &self.title
    }

    async fn activate(&self, x: i32, y: i32) {
        tracing::info!(x, y, "Activate");
    }

    async fn secondary_activate(&self, x: i32, y: i32) {
        tracing::info!(x, y, "SecondaryActivate");
    }

    async fn context_menu(&self, x: i32, y: i32) {
        tracing::info!(x, y, "ContextMenu");
    }

    async fn scroll(&self, delta: i32, orientation: &str) {
        tracing::info!(delta, orientation, "Scroll");
    }

    #[zbus(signal)]
    async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_status(emitter: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;
}

/// Its menu, exposed over `com.canonical.dbusmenu`.
struct Menu {
    checked: Arc<AtomicBool>,
    revision: AtomicUsize,
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    fn get_layout(
        &self,
        parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, MenuLayout) {
        let children = vec![
            leaf(NODE_NOTIFICATION, &[("label", Value::from("Show _notification"))]),
            leaf(
                NODE_CHECKBOX,
                &[
                    ("label", Value::from("_Checked option")),
                    ("toggle-type", Value::from("checkmark")),
                    (
                        "toggle-state",
                        Value::from(if self.checked.load(Ordering::Relaxed) {
                            1i32
                        } else {
                            0i32
                        }),
                    ),
                ],
            ),
            leaf(NODE_SEPARATOR, &[("type", Value::from("separator"))]),
            leaf(NODE_QUIT, &[("label", Value::from("_Quit")), ("shortcut", shortcuts())]),
        ];
        (
            self.revision.load(Ordering::Relaxed) as u32,
            (parent_id.max(0), props(&[]), children),
        )
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, PropertyMap)> {
        let (_, (_, _, children)) = self.get_layout(0, -1, Vec::new());
        children
            .iter()
            .filter_map(node_from_value)
            .filter(|(id, _)| ids.is_empty() || ids.contains(id))
            .collect()
    }

    fn event(&self, id: i32, event_id: &str, _data: Value<'_>, timestamp: u32) {
        tracing::info!(id, event_id, timestamp, "menu event");
        match (id, event_id) {
            (NODE_CHECKBOX, "clicked") => {
                let now = !self.checked.load(Ordering::Relaxed);
                self.checked.store(now, Ordering::Relaxed);
            }
            (NODE_QUIT, "clicked") => {
                tracing::info!("quitting on request of the menu");
                std::process::exit(0);
            }
            _ => {}
        }
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    #[zbus(signal)]
    async fn layout_updated(emitter: &SignalEmitter<'_>, revision: u32, parent: i32)
        -> zbus::Result<()>;

    #[zbus(signal)]
    async fn items_properties_updated(
        emitter: &SignalEmitter<'_>,
        updated_props: Vec<(i32, PropertyMap)>,
        removed_props: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;
}

/// Build one menu node as the wire value a `children` array carries.
fn leaf(id: i32, entries: &[(&str, Value<'_>)]) -> OwnedValue {
    let properties: HashMap<String, Value<'_>> = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.try_clone().expect("clone value")))
        .collect();
    let structure =
        Structure::from((id, properties, Vec::<OwnedValue>::new()));
    OwnedValue::try_from(Value::Structure(structure)).expect("owned node")
}

fn props(entries: &[(&str, Value<'_>)]) -> PropertyMap {
    entries
        .iter()
        .map(|(name, value)| {
            (
                (*name).to_owned(),
                OwnedValue::try_from(value.try_clone().expect("clone value")).expect("owned value"),
            )
        })
        .collect()
}

fn shortcuts() -> Value<'static> {
    Value::Array(
        zvariant::Array::from(vec![vec!["Control".to_owned(), "Q".to_owned()]]),
    )
}

/// Decode a node we built ourselves, used to answer `GetGroupProperties`.
fn node_from_value(owned: &OwnedValue) -> Option<(i32, PropertyMap)> {
    let value: &Value<'_> = owned;
    let Value::Structure(structure) = value else {
        return None;
    };
    let fields = structure.fields();
    let id = match fields.first()? {
        Value::I32(id) => *id,
        _ => return None,
    };
    let properties = match fields.get(1)? {
        Value::Dict(dict) => dict
            .iter()
            .filter_map(|(key, value)| match key {
                Value::Str(name) => Some((
                    name.as_str().to_owned(),
                    OwnedValue::try_from(value.try_clone().expect("clone value"))
                        .expect("owned value"),
                )),
                _ => None,
            })
            .collect(),
        _ => PropertyMap::new(),
    };
    Some((id, properties))
}

/// A handful of generated icons: a coloured square with a moving stripe, so
/// icon updates are visible at a glance.
fn icon_frames() -> Vec<IconFrame> {
    (0..4)
        .map(|frame| {
            [22, 44]
                .into_iter()
                .map(|size| {
                    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
                    for y in 0..size {
                        for x in 0..size {
                            let stripe = ((x + y) / (size / 8)) % 4 == frame;
                            let (r, g, b) = if stripe { (255, 255, 255) } else { (30, 90, 200) };
                            // ARGB32 in network byte order.
                            pixels.extend_from_slice(&[255, r, g, b]);
                        }
                    }
                    (size, size, pixels)
                })
                .collect()
        })
        .collect()
}

async fn watcher(conn: &Connection) -> anyhow::Result<taskbar_api::StatusNotifierWatcherProxy<'static>> {
    Ok(taskbar_api::StatusNotifierWatcherProxy::builder(conn)
        .destination(WATCHER_BUS_NAME)?
        .path(WATCHER_OBJECT_PATH)?
        .build()
        .await?)
}

async fn emit<B: serde::Serialize + zvariant::Type>(
    conn: &Connection,
    path: &str,
    interface: &str,
    name: &str,
    body: &B,
) {
    if let Err(error) = conn
        .emit_signal(None::<&str>, ObjectPath::try_from(path.to_owned()).expect("path"), interface, name, body)
        .await
    {
        tracing::warn!(%error, "cannot emit {name}");
    }
}
