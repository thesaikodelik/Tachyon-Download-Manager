//! App-wide state and the functions that open each window.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::TimeZone;
use gpui::{
    App, AppContext, Bounds, Entity, Global, Pixels, Render, Size, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowKind, WindowOptions, point, px, size,
};
use tachyon_engine::{DownloadId, Engine, EngineEvent, PostAction, looks_downloadable};

use crate::add_dialog::{AddDialog, AddPrefill};
use crate::complete_dialog::CompleteDialog;
use crate::main_window::MainWindow;
use crate::options_window::OptionsWindow;
use crate::post_action::PostActionDialog;
use crate::progress_window::ProgressWindow;
use crate::scheduler_window::SchedulerWindow;

pub struct AppState {
    pub engine: Arc<Engine>,
    main_window: Option<WindowHandle<MainWindow>>,
    /// The main window's native handle, for placing dialogs over it.
    main_hwnd: Option<usize>,
    progress_windows: HashMap<DownloadId, WindowHandle<ProgressWindow>>,
    options_window: Option<WindowHandle<OptionsWindow>>,
    scheduler_window: Option<WindowHandle<SchedulerWindow>>,
    last_clipboard: Option<String>,
    /// What the update check found.
    pub update: crate::updater::UpdateState,
    /// The update bar was closed with Later.
    pub update_dismissed: bool,
}

impl Global for AppState {}

pub fn engine(cx: &App) -> Arc<Engine> {
    cx.global::<AppState>().engine.clone()
}

pub fn init(engine: Arc<Engine>, cx: &mut App) {
    crate::theme::set_style(engine.settings().window_style);
    crate::theme::set_mode(engine.settings().theme_mode);
    let last_clipboard = cx.read_from_clipboard().and_then(|c| c.text());
    cx.set_global(AppState {
        engine,
        main_window: None,
        main_hwnd: None,
        progress_windows: HashMap::new(),
        options_window: None,
        scheduler_window: None,
        last_clipboard,
        update: Default::default(),
        update_dismissed: false,
    });
    crate::updater::init(cx);
    cx.spawn(async move |cx| {
        loop {
            gpui::Timer::after(Duration::from_millis(400)).await;
            if cx.update(tick).is_err() {
                break;
            }
        }
    })
    .detach();
}

/// Applies the settings that live outside the engine: window style and sign-in start.
pub fn apply_system_settings(settings: &tachyon_engine::Settings, cx: &mut App) {
    crate::theme::set_style(settings.window_style);
    crate::theme::set_mode(settings.theme_mode);
    cx.refresh_windows();
    if let Err(e) = crate::platform::set_start_with_system(settings.start_with_system) {
        log::warn!("could not change starting with Windows: {e}");
    }
}

/// Reacts to engine events and watches the clipboard.
fn tick(cx: &mut App) {
    if crate::platform::take_show_request() {
        // Tachyon was launched again: show this instance instead.
        open_main_window(cx);
    }
    // Straight to the file's details, as for a link from the browser.
    for link in crate::platform::take_dropped_links() {
        let request = tachyon_engine::AddRequest { request: tachyon_engine::RequestSpec::new(link), ..Default::default() };
        open_add_dialog(AddPrefill::Captured(Box::new(request)), cx);
    }
    let engine = engine(cx);
    for event in engine.drain_events() {
        match event {
            EngineEvent::Completed(id) => on_completed(id, cx),
            EngineEvent::Failed { .. } => {
                if engine.settings().play_sounds {
                    crate::platform::play_sound(false);
                }
            }
            EngineEvent::QuotaReached { resumes_at } => {
                let when = chrono::Local
                    .timestamp_opt(resumes_at, 0)
                    .single()
                    .map(|t| t.format("%H:%M").to_string())
                    .unwrap_or_else(|| "the end of the period".into());
                crate::message_dialog::show(
                    crate::message_dialog::Kind::Info,
                    "Download Limit",
                    "Download limit reached",
                    format!(
                        "Downloads were stopped because the limit set in Options → Connection was reached. \
                         They can start again at {when}."
                    ),
                    cx,
                );
            }
            EngineEvent::LinksCaptured(batch) => open_links_window(*batch, cx),
            EngineEvent::Captured(request) => {
                open_add_dialog(AddPrefill::Captured(request), cx);
            }
            EngineEvent::QueueFinished { post_action } => run_post_action(post_action, cx),
            EngineEvent::PostAction(action) => run_post_action(action, cx),
        }
    }

    for finding in crate::antivirus::take_findings() {
        let name = finding.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        crate::message_dialog::show(
            crate::message_dialog::Kind::Warning,
            "Virus Scan",
            format!("Virus scan: {name}"),
            finding.message,
            cx,
        );
    }

    if engine.settings().monitor_clipboard {
        let text = cx.read_from_clipboard().and_then(|c| c.text());
        let state = cx.global_mut::<AppState>();
        if text != state.last_clipboard {
            state.last_clipboard = text.clone();
            if let Some(url) = text.map(|t| t.trim().to_string())
                && looks_downloadable(&url, &engine.settings().categories)
            {
                open_add_dialog(AddPrefill::Url(url), cx);
            }
        }
    }
}

fn on_completed(id: DownloadId, cx: &mut App) {
    let engine = engine(cx);
    let Some(download) = engine.get(id) else { return };
    if let Some(handle) = cx.global_mut::<AppState>().progress_windows.remove(&id) {
        handle.update(cx, |_, window, _| window.remove_window()).ok();
    }
    let settings = engine.settings();
    if settings.play_sounds {
        crate::platform::play_sound(true);
    }
    if settings.virus_scan {
        crate::antivirus::scan(download.path(), settings.virus_scanner.clone());
    }
    if download.completion.open_file {
        cx.open_with_system(&download.path());
    }
    if download.completion.show_dialog && engine.settings().show_complete_dialog {
        open_complete_dialog(id, cx);
    }
}

pub fn run_post_action(action: PostAction, cx: &mut App) {
    if action == PostAction::Nothing {
        return;
    }
    let bounds = centered(size(px(380.), px(170.)), cx);
    open_styled(dialog_options("Downloads Finished", bounds), cx, |window, cx| {
        cx.new(|cx| PostActionDialog::new(action, window, cx))
    })
    .ok();
    cx.activate(true);
}

/// Opens a window with its glass in place before anything is drawn, and keeps it
/// off screen (cloaked) until its first frame is ready. Otherwise a new window
/// briefly shows what is behind it, unblurred, and then pops into place.
pub fn open_styled<V: Render + 'static>(
    options: WindowOptions,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> anyhow::Result<WindowHandle<V>> {
    let visible = options.show;
    cx.open_window(options, move |window, cx| {
        let theme = crate::theme::Theme::for_appearance(window.appearance());
        crate::platform::sync_chrome(window, crate::theme::backdrop(), theme.dark);
        crate::platform::refuse_file_drops(window);
        if visible {
            crate::platform::cloak(window, true);
            // Two frames: dialogs that fit their content take their final size in the second.
            window.on_next_frame(|window, _| {
                window.on_next_frame(|window, _| crate::platform::cloak(window, false));
            });
            // Never leave a window invisible, even if no frame comes.
            window
                .spawn(cx, async move |cx| {
                    gpui::Timer::after(Duration::from_millis(500)).await;
                    cx.update(|window, _| crate::platform::cloak(window, false)).ok();
                })
                .detach();
        }
        build(window, cx)
    })
}

/// Where a window of `size` opens: centred over the main window while it is on
/// screen, as Windows dialogs are, otherwise in the middle of the screen.
///
/// The main window's position comes from Windows rather than GPUI, which can't
/// read a window that is busy, e.g. handling the click that opens the dialog.
pub fn centered(size: Size<Pixels>, cx: &mut App) -> Bounds<Pixels> {
    let main = cx.try_global::<AppState>().and_then(|state| state.main_hwnd);
    match main.and_then(crate::platform::shown_window_rect) {
        Some(main) => Bounds::new(
            point(
                main.origin.x + (main.size.width - size.width) / 2.,
                main.origin.y + (main.size.height - size.height) / 2.,
            ),
            size,
        ),
        None => Bounds::centered(None, size, cx),
    }
}

/// Windows 11 styles draw their own title bar on the window's glass.
pub fn custom_title_bar() -> bool {
    cfg!(windows) && crate::theme::style() != tachyon_engine::WindowStyle::Classic
}

pub fn dialog_options(title: &str, bounds: Bounds<Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_string().into()),
            appears_transparent: custom_title_bar(),
            traffic_light_position: None,
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_resizable: false,
        is_minimizable: true,
        // Dialogs resize themselves between steps, so only guard against absurd sizes.
        window_min_size: Some(size(px(240.), px(120.))),
        ..Default::default()
    }
}

/// Brings an existing window forward; returns false if it was closed.
fn activate<T: 'static>(handle: Option<WindowHandle<T>>, cx: &mut App) -> bool {
    handle.is_some_and(|h| h.update(cx, |_, window, _| window.activate_window()).is_ok())
}

pub fn open_main_window(cx: &mut App) {
    open_main_window_with(true, cx);
}

/// Opens the main window, or with `show` false creates it hidden (starting in the
/// notification area). An existing window is shown, even if hidden in the tray.
pub fn open_main_window_with(show: bool, cx: &mut App) {
    if let Some(handle) = cx.global::<AppState>().main_window {
        let shown = handle
            .update(cx, |_, window, _| {
                if show {
                    crate::platform::show_window(window);
                    window.activate_window();
                }
            })
            .is_ok();
        if shown {
            return;
        }
    }
    let bounds = centered(size(px(1120.), px(680.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("Tachyon Download Manager".into()),
            appears_transparent: custom_title_bar(),
            traffic_light_position: Some(point(px(12.), px(12.))),
        }),
        window_min_size: Some(size(px(760.), px(420.))),
        focus: show,
        show,
        ..Default::default()
    };
    let opened = open_styled(options, cx, |window, cx| {
        let hwnd = crate::platform::native_handle(window);
        cx.global_mut::<AppState>().main_hwnd = hwnd;
        // Links dropped on the main window are added, as in IDM.
        crate::platform::accept_dropped_links(window);
        // With a notification-area icon, closing the window only hides it, as in IDM.
        window.on_window_should_close(cx, |window, cx| {
            let keep_running = crate::tray::is_active() && engine(cx).settings().close_to_tray;
            if keep_running {
                crate::platform::hide_window(window);
            }
            !keep_running
        });
        cx.new(|cx| MainWindow::new(window, cx))
    });
    if let Ok(handle) = opened {
        cx.global_mut::<AppState>().main_window = Some(handle);
    }
    if show {
        cx.activate(true);
    }
}

pub fn open_add_dialog(prefill: AddPrefill, cx: &mut App) {
    // Opens at the size of the first step so it stays centred; later steps resize it.
    let fluent = crate::theme::style() != tachyon_engine::WindowStyle::Classic;
    let bounds = centered(if fluent { size(px(580.), px(136.)) } else { size(px(560.), px(400.)) }, cx);
    // Sent from the browser: Tachyon is in the background and must come forward.
    let from_browser = matches!(prefill, AddPrefill::Captured(_));
    let handle = open_styled(dialog_options("Add Download", bounds), cx, |window, cx| {
        cx.new(|cx| AddDialog::new(prefill, window, cx))
    });
    cx.activate(true);
    if from_browser && let Ok(handle) = handle {
        cx.spawn(async move |cx| {
            // Once now, and again after the window has drawn and uncloaked.
            for delay in [0, 600] {
                cx.background_executor().timer(std::time::Duration::from_millis(delay)).await;
                handle.update(cx, |_, window, _| crate::platform::bring_to_front(window)).ok();
            }
        })
        .detach();
    }
}

pub const PROGRESS_SIZE: (f32, f32) = (540., 360.);
pub const PROGRESS_SIZE_EXPANDED: (f32, f32) = (540., 600.);
pub const PROGRESS_SIZE_FLUENT: (f32, f32) = (520., 362.);
pub const PROGRESS_SIZE_FLUENT_EXPANDED: (f32, f32) = (520., 584.);

/// Size of the progress window for the current style.
pub fn progress_size(expanded: bool) -> (f32, f32) {
    let fluent = crate::theme::style() != tachyon_engine::WindowStyle::Classic;
    match (fluent, expanded) {
        (true, false) => PROGRESS_SIZE_FLUENT,
        (true, true) => PROGRESS_SIZE_FLUENT_EXPANDED,
        (false, false) => PROGRESS_SIZE,
        (false, true) => PROGRESS_SIZE_EXPANDED,
    }
}

pub fn open_progress_window(id: DownloadId, cx: &mut App) {
    let existing = cx.global::<AppState>().progress_windows.get(&id).copied();
    if activate(existing, cx) {
        return;
    }
    let (w, h) = progress_size(false);
    let bounds = centered(size(px(w), px(h)), cx);
    let mut options = dialog_options("Download", bounds);
    options.focus = false;
    if let Ok(handle) = open_styled(options, cx, |window, cx| cx.new(|cx| ProgressWindow::new(id, window, cx))) {
        cx.global_mut::<AppState>().progress_windows.insert(id, handle);
    }
}

pub fn forget_progress_window(id: DownloadId, cx: &mut App) {
    cx.global_mut::<AppState>().progress_windows.remove(&id);
}

pub const COMPLETE_SIZE_FLUENT: (f32, f32) = (540., 240.);

pub fn open_complete_dialog(id: DownloadId, cx: &mut App) {
    let fluent = crate::theme::style() != tachyon_engine::WindowStyle::Classic;
    let (w, h) = if fluent { COMPLETE_SIZE_FLUENT } else { (520., 270.) };
    let bounds = centered(size(px(w), px(h)), cx);
    open_styled(dialog_options("Download Complete", bounds), cx, |window, cx| {
        cx.new(|cx| CompleteDialog::new(id, window, cx))
    })
    .ok();
    cx.activate(true);
}

pub fn open_options(cx: &mut App) {
    if activate(cx.global::<AppState>().options_window, cx) {
        return;
    }
    let bounds = centered(size(px(800.), px(600.)), cx);
    if let Ok(handle) = open_styled(dialog_options("Options", bounds), cx, |window, cx| {
        cx.new(|cx| OptionsWindow::new(window, cx))
    }) {
        cx.global_mut::<AppState>().options_window = Some(handle);
    }
    cx.activate(true);
}

/// The links of a page the browser sent, to pick from. In front of the browser.
pub fn open_links_window(batch: tachyon_engine::LinkBatch, cx: &mut App) {
    let bounds = centered(size(px(860.), px(560.)), cx);
    let mut options = dialog_options("Download Links", bounds);
    options.is_resizable = true;
    let handle = open_styled(options, cx, |window, cx| {
        cx.new(|cx| crate::links_window::LinksWindow::new(batch, window, cx))
    });
    cx.activate(true);
    if let Ok(handle) = handle {
        cx.spawn(async move |cx| {
            for delay in [0, 600] {
                cx.background_executor().timer(std::time::Duration::from_millis(delay)).await;
                handle.update(cx, |_, window, _| crate::platform::bring_to_front(window)).ok();
            }
        })
        .detach();
    }
}

/// Options at the page numbered `tab` (Browser is the last).
pub fn open_options_at(tab: usize, cx: &mut App) {
    open_options(cx);
    if let Some(handle) = cx.global::<AppState>().options_window {
        handle
            .update(cx, |options, _, cx| {
                options.tab = tab;
                cx.notify();
            })
            .ok();
    }
}

pub fn open_scheduler(cx: &mut App) {
    if activate(cx.global::<AppState>().scheduler_window, cx) {
        return;
    }
    let bounds = centered(size(px(620.), px(620.)), cx);
    if let Ok(handle) = open_styled(dialog_options("Scheduler", bounds), cx, |window, cx| {
        cx.new(|cx| SchedulerWindow::new(window, cx))
    }) {
        cx.global_mut::<AppState>().scheduler_window = Some(handle);
    }
    cx.activate(true);
}

/// The extension as Firefox loads it: a copy with Firefox's own manifest, which
/// Chrome would warn about. Rebuilt in Tachyon's data folder on every call.
pub fn firefox_extension_dir(data_dir: &std::path::Path) -> std::io::Result<PathBuf> {
    let source = extension_dir().ok_or_else(|| std::io::Error::other("extension folder not found"))?;
    let target = data_dir.join("firefox-extension");
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    copy_dir(&source, &target)?;
    std::fs::remove_file(target.join("firefox.json"))?;
    std::fs::copy(source.join("firefox.json"), target.join("manifest.json"))?;
    Ok(target)
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

/// The bundled browser extension, for the Options window's "Show extension" button.
pub fn extension_dir() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(PathBuf::from));
    // Inside the macOS app bundle, or next to Tachyon.exe on Windows.
    let bundled = exe_dir.as_ref().map(|p| p.join("../Resources/extension"));
    let beside = exe_dir.map(|p| p.join("extension"));
    let source = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../extension"));
    [bundled, beside, Some(source)]
        .into_iter()
        .flatten()
        .find(|p| p.join("manifest.json").exists())
        .and_then(|p| p.canonicalize().ok())
}
