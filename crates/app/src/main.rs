//! Tachyon Download Manager: a native download accelerator for macOS and Windows
//! inspired by Internet Download Manager, written in Rust with GPUI.

// No console window behind the app in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod add_dialog;
mod antivirus;
mod app;
mod assets;
mod brand;
mod complete_dialog;
mod file_icons;
mod format;
mod links_window;
mod main_window;
mod message_dialog;
mod native_host;
mod options_window;
mod platform;
mod post_action;
mod progress_window;
mod scheduler_window;
mod setup;
#[cfg(feature = "dev-snapshots")]
mod snapshot;
mod theme;
mod tray;
mod widgets;

use std::sync::Arc;

use gpui::{App, Application, KeyBinding, Menu, MenuItem, SystemMenuType, actions};
use tachyon_engine::Engine;

use crate::add_dialog::AddPrefill;

actions!(
    tachyon,
    [
        Quit,
        About,
        AddUrl,
        ShowOptions,
        ShowScheduler,
        ShowMainWindow,
        StartQueue,
        StopQueue,
        StopAll,
        DeleteCompleted,
        CloseWindow,
    ]
);

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Started by a browser so its extension can open Tachyon: do that and leave.
    if native_host::is_host_launch() {
        native_host::serve();
        return;
    }
    // Tachyon is its own installer and uninstaller.
    if setup::is_uninstall_launch() {
        setup::run(true);
        return;
    }
    if setup::is_setup_launch() {
        setup::run(false);
        return;
    }

    // TACHYON_DATA_DIR keeps development runs away from the real download list.
    let data_dir = std::env::var_os("TACHYON_DATA_DIR").map(std::path::PathBuf::from).or_else(Engine::default_data_dir);
    // A second launch shows the running Tachyon instead of starting another engine
    // on the same download list.
    if let Some(dir) = &data_dir
        && !platform::claim_single_instance(dir)
    {
        return;
    }
    let start_in_tray = std::env::args().any(|arg| arg == platform::START_IN_TRAY_FLAG);
    let opened = match data_dir {
        Some(dir) => Engine::open(dir),
        None => Engine::open_default(),
    };
    let engine = match opened {
        Ok(engine) => Arc::new(engine),
        Err(e) => {
            eprintln!("Tachyon could not start its download engine: {e:#}");
            std::process::exit(1);
        }
    };
    // Development runs leave the browsers pointing at the installed Tachyon.
    if std::env::var_os("TACHYON_DATA_DIR").is_none() {
        native_host::register(engine.data_dir());
    }

    let app = Application::new().with_assets(assets::Assets);
    app.on_reopen(app::open_main_window);
    app.run(move |cx: &mut App| {
        app::init(engine.clone(), cx);
        widgets::text_input::bind_keys(cx);
        main_window::bind_keys(cx);
        register_actions(cx);
        cx.set_menus(menus());

        let quit_engine = engine.clone();
        cx.on_app_quit(move |_| {
            quit_engine.shutdown();
            async {}
        })
        .detach();

        #[cfg(feature = "dev-snapshots")]
        snapshot::init(cx);

        tray::init(cx);
        let settings = engine.settings();
        if settings.start_with_system {
            // Keeps the sign-in entry pointing at this copy of Tachyon if it moved.
            if let Err(e) = platform::set_start_with_system(true) {
                log::warn!("could not update starting with Windows: {e}");
            }
        }
        app::open_main_window_with(!(start_in_tray && tray::is_active()), cx);
        // From the installer's last step: how to add the browser extension.
        if std::env::args().any(|a| a == setup::BROWSER_SETUP_FLAG) {
            app::open_options_at(7, cx);
        }
    });
}

fn register_actions(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &About, cx| {
        app::open_main_window(cx);
        brand::show_about(cx);
    });
    cx.on_action(|_: &AddUrl, cx| app::open_add_dialog(AddPrefill::FromClipboard, cx));
    cx.on_action(|_: &ShowOptions, cx| app::open_options(cx));
    cx.on_action(|_: &ShowScheduler, cx| app::open_scheduler(cx));
    cx.on_action(|_: &ShowMainWindow, cx| app::open_main_window(cx));
    cx.on_action(|_: &StartQueue, cx| app::engine(cx).start_queue());
    cx.on_action(|_: &StopQueue, cx| app::engine(cx).stop_queue());
    cx.on_action(|_: &StopAll, cx| app::engine(cx).pause_all());
    cx.on_action(|_: &DeleteCompleted, cx| {
        app::engine(cx).remove_completed();
    });
    cx.on_action(|_: &CloseWindow, cx| {
        if let Some(window) = cx.active_window() {
            window.update(cx, |_, window, _| window.remove_window()).ok();
        }
    });

    cx.bind_keys([
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("secondary-n", AddUrl, None),
        KeyBinding::new("secondary-,", ShowOptions, None),
        KeyBinding::new("secondary-shift-s", ShowScheduler, None),
        KeyBinding::new("secondary-0", ShowMainWindow, None),
        KeyBinding::new("secondary-w", CloseWindow, None),
    ]);
}

fn menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "Tachyon".into(),
            items: vec![
                MenuItem::action("About Tachyon", About),
                MenuItem::separator(),
                MenuItem::action("Options…", ShowOptions),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Quit Tachyon", Quit),
            ],
        },
        Menu {
            name: "Downloads".into(),
            items: vec![
                MenuItem::action("Add URL…", AddUrl),
                MenuItem::separator(),
                MenuItem::action("Stop All", StopAll),
                MenuItem::action("Delete All Completed", DeleteCompleted),
                MenuItem::separator(),
                MenuItem::action("Start Queue", StartQueue),
                MenuItem::action("Stop Queue", StopQueue),
                MenuItem::action("Scheduler…", ShowScheduler),
            ],
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Downloads", ShowMainWindow),
                MenuItem::action("Close Window", CloseWindow),
            ],
        },
    ]
}
