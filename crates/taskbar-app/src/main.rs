//! `taskbar-settings`: the GTK4 and libadwaita settings app.
//!
//! A thin shell around the state machine in [`model`]: user input becomes
//! [`Msg`]s, the reducer answers with the next state and the [`Call`]s to
//! make, and [`view`] paints the result. Everything the tray decides happens
//! in `taskbar-core`, next to the daemon that applies it.

mod client;
mod model;
mod view;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use model::{Call, Msg, State};

const APP_ID: &str = "dev.taskbar.Settings";

fn main() -> gtk::glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("taskbar_app=info")),
        )
        .init();

    let application = adw::Application::builder().application_id(APP_ID).build();
    application.connect_activate(build);
    application.run()
}

/// The running app: state, widgets and the worker that talks to the daemon.
struct App {
    state: RefCell<State>,
    widgets: view::Widgets,
    client: client::Client,
}

fn build(application: &adw::Application) {
    if !application.windows().is_empty() {
        return;
    }

    let (messages, incoming) = async_channel::unbounded::<Msg>();
    let widgets = view::build(application, messages.clone());
    let app = Rc::new(App {
        state: RefCell::new(State::default()),
        widgets,
        client: client::Client::start(messages),
    });

    install_actions(application, &app);

    app.widgets.window.present();
    app.client.call(Call::Load);

    // Deliver replies and signals on the main loop, where widgets live.
    gtk::glib::spawn_future_local({
        let app = Rc::clone(&app);
        async move {
            while let Ok(msg) = incoming.recv().await {
                dispatch(&app, msg);
            }
        }
    });
}

/// Fold one message through the reducer, paint the result and carry out the
/// calls it asked for.
fn dispatch(app: &Rc<App>, msg: Msg) {
    let failed = matches!(msg, Msg::Failed(_));
    let (next, calls) = {
        let state = app.state.borrow();
        model::update(&state, msg)
    };
    *app.state.borrow_mut() = next;

    {
        let state = app.state.borrow();
        view::apply(&state, &app.widgets);
        if failed {
            if let Some(error) = &state.error {
                app.widgets.toast.add_toast(adw::Toast::new(error));
            }
        }
    }

    for call in calls {
        app.client.call(call);
    }
}

fn install_actions(application: &adw::Application, app: &Rc<App>) {
    let refresh = gtk::gio::SimpleAction::new("refresh", None);
    refresh.connect_activate({
        let app = Rc::clone(app);
        move |_, _| dispatch(&app, Msg::Refresh)
    });
    app.widgets.window.add_action(&refresh);

    let about = gtk::gio::SimpleAction::new("about", None);
    about.connect_activate({
        let app = Rc::clone(app);
        move |_, _| show_about(&app.widgets.window)
    });
    app.widgets.window.add_action(&about);

    application.set_accels_for_action("win.refresh", &["<Primary>r"]);
    application.set_accels_for_action("win.about", &["<Primary>question"]);
}

fn show_about(window: &adw::ApplicationWindow) {
    let dialog = adw::AboutDialog::builder()
        .application_name("Taskbar")
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("Taskbar contributors")
        .comments("A system tray (StatusNotifierItem) for the GNOME panel.")
        .license_type(gtk::License::Gpl30)
        .build();
    dialog.present(Some(window));
}
