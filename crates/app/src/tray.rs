//! Notification-area icon (Windows): click to show Tachyon, right-click for a menu,
//! and a tooltip with what is downloading. With it, closing the main window keeps
//! Tachyon running, as IDM does.

use gpui::App;

/// Whether the icon exists, so closing the main window can hide it instead.
pub fn is_active() -> bool {
    imp::is_active()
}

/// Creates the icon and starts handling its events.
pub fn init(cx: &mut App) {
    imp::init(cx)
}

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::time::{Duration, Instant};

    use gpui::App;
    use tachyon_engine::DownloadStatus;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use crate::add_dialog::AddPrefill;
    use crate::{app, format};

    const TITLE: &str = "Tachyon Download Manager";

    struct Tray {
        icon: TrayIcon,
        open: MenuItem,
        add: MenuItem,
        stop_all: MenuItem,
        quit: MenuItem,
        tooltip: String,
        tooltip_at: Instant,
    }

    thread_local! {
        static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
    }

    pub fn is_active() -> bool {
        TRAY.with_borrow(|tray| tray.is_some())
    }

    fn load_icon() -> Option<Icon> {
        let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!("../assets/windows/tray.png")));
        let mut reader = decoder.read_info().ok()?;
        let mut rgba = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut rgba).ok()?;
        rgba.truncate(info.buffer_size());
        Icon::from_rgba(rgba, info.width, info.height).ok()
    }

    pub fn init(cx: &mut App) {
        let open = MenuItem::new("Open Tachyon", true, None);
        let add = MenuItem::new("Add URL…", true, None);
        let stop_all = MenuItem::new("Stop All", true, None);
        let quit = MenuItem::new("Exit", true, None);
        let menu = Menu::new();
        let built = menu
            .append_items(&[&open, &add, &PredefinedMenuItem::separator(), &stop_all, &PredefinedMenuItem::separator(), &quit])
            .map_err(|e| e.to_string())
            .and_then(|()| {
                let mut builder = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip(TITLE);
                if let Some(icon) = load_icon() {
                    builder = builder.with_icon(icon);
                }
                builder.build().map_err(|e| e.to_string())
            });
        let icon = match built {
            Ok(icon) => icon,
            Err(e) => {
                log::warn!("no notification-area icon: {e}");
                return;
            }
        };
        TRAY.with_borrow_mut(|tray| {
            *tray = Some(Tray {
                icon,
                open,
                add,
                stop_all,
                quit,
                tooltip: TITLE.into(),
                tooltip_at: Instant::now(),
            })
        });
        // The icon's events arrive on channels; poll them often enough to feel instant.
        cx.spawn(async move |cx| {
            loop {
                gpui::Timer::after(Duration::from_millis(100)).await;
                if cx.update(poll).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    enum Action {
        Show,
        Add,
        StopAll,
        Quit,
    }

    fn poll(cx: &mut App) {
        let mut actions = Vec::new();
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            match event {
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => actions.push(Action::Show),
                _ => {}
            }
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            TRAY.with_borrow(|tray| {
                let Some(tray) = tray else { return };
                if event.id == *tray.open.id() {
                    actions.push(Action::Show);
                } else if event.id == *tray.add.id() {
                    actions.push(Action::Add);
                } else if event.id == *tray.stop_all.id() {
                    actions.push(Action::StopAll);
                } else if event.id == *tray.quit.id() {
                    actions.push(Action::Quit);
                }
            });
        }
        for action in actions {
            match action {
                Action::Show => app::open_main_window(cx),
                Action::Add => app::open_add_dialog(AddPrefill::FromClipboard, cx),
                Action::StopAll => app::engine(cx).pause_all(),
                Action::Quit => {
                    // Remove the icon first so it doesn't linger until the mouse passes over it.
                    TRAY.with_borrow_mut(|tray| tray.take());
                    cx.quit();
                    return;
                }
            }
        }
        update_tooltip(cx);
    }

    /// "Tachyon Download Manager — 2 downloading · 11.5 MB/s", refreshed every second.
    fn update_tooltip(cx: &mut App) {
        let due = TRAY.with_borrow(|tray| tray.as_ref().is_some_and(|t| t.tooltip_at.elapsed() >= Duration::from_secs(1)));
        if !due {
            return;
        }
        let downloads = app::engine(cx).snapshot();
        let active: Vec<_> = downloads.iter().filter(|d| d.status == DownloadStatus::Downloading).collect();
        let text = if active.is_empty() {
            TITLE.to_string()
        } else {
            let speed: u64 = active.iter().map(|d| d.speed).sum();
            format!("{TITLE}\n{} downloading · {}", active.len(), format::speed(speed))
        };
        TRAY.with_borrow_mut(|tray| {
            let Some(tray) = tray else { return };
            tray.tooltip_at = Instant::now();
            if tray.tooltip != text {
                let _ = tray.icon.set_tooltip(Some(&text));
                tray.tooltip = text;
            }
        });
    }
}

#[cfg(not(windows))]
mod imp {
    use gpui::App;

    pub fn is_active() -> bool {
        false
    }

    pub fn init(_: &mut App) {}
}
