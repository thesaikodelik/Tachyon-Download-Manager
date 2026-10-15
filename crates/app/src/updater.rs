//! Tachyon's own updates: a check once a day, a quiet bar in the main window when a
//! new version is out, and the update downloaded, checked and installed on request.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::{AnyElement, App, FontWeight, div, prelude::*, px};
use tachyon_engine::Update;

use crate::app::{self, AppState};
use crate::assets::Icon;
use crate::theme::Theme;
use crate::widgets::{Button, icon, progress_bar};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const FIRST_CHECK: Duration = Duration::from_secs(20);
const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);

#[derive(Clone, Default)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Update),
    Downloading(Update, Arc<AtomicU64>),
    Failed(String),
}

fn set(state: UpdateState, cx: &mut App) {
    cx.global_mut::<AppState>().update = state;
    cx.refresh_windows();
}

fn state(cx: &App) -> UpdateState {
    cx.global::<AppState>().update.clone()
}

/// Checks a little after start and then daily, while the setting allows. Development
/// profiles don't, unless pointed at a test server.
pub fn init(cx: &mut App) {
    // The installer of an update already applied (it may still be closing; then the
    // next start removes it).
    let updates = app::engine(cx).data_dir().join("updates");
    for entry in std::fs::read_dir(&updates).into_iter().flatten().flatten() {
        let _ = std::fs::remove_file(entry.path());
    }
    let development = std::env::var_os("TACHYON_DATA_DIR").is_some() && std::env::var_os("TACHYON_UPDATE_URL").is_none();
    if development {
        return;
    }
    cx.spawn(async move |cx| {
        let mut wait = FIRST_CHECK;
        loop {
            gpui::Timer::after(wait).await;
            wait = CHECK_EVERY;
            let wanted = cx.update(|cx| app::engine(cx).settings().check_for_updates).unwrap_or(false);
            if wanted && cx.update(|cx| check(false, cx)).is_err() {
                break;
            }
        }
    })
    .detach();
}

/// Looks for a new version; `manual` shows "Checking…" and "up to date" too.
pub fn check(manual: bool, cx: &mut App) {
    if matches!(state(cx), UpdateState::Checking | UpdateState::Downloading(..)) {
        return;
    }
    if manual {
        set(UpdateState::Checking, cx);
        cx.global_mut::<AppState>().update_dismissed = false;
    }
    let lookup = app::engine(cx).check_for_update(VERSION);
    cx.spawn(async move |cx| {
        let result = lookup.await;
        cx.update(|cx| {
            let next = match result {
                Ok(Some(update)) => UpdateState::Available(update),
                Ok(None) => UpdateState::UpToDate,
                Err(e) if manual => UpdateState::Failed(format!("Couldn't check for updates ({e})")),
                Err(e) => {
                    log::info!("update check failed: {e}");
                    return;
                }
            };
            set(next, cx);
        })
        .ok();
    })
    .detach();
}

/// Downloads the update, then hands over to its installer, which closes Tachyon,
/// installs and starts the new version.
pub fn install(cx: &mut App) {
    let UpdateState::Available(update) = state(cx) else { return };
    let progress = Arc::new(AtomicU64::new(0));
    set(UpdateState::Downloading(update.clone(), progress.clone()), cx);
    let download = app::engine(cx).download_update(update.clone(), progress);
    cx.spawn(async move |cx| {
        let result = download.await;
        cx.update(|cx| match result {
            Ok(installer) => match std::process::Command::new(&installer).args(["--setup", crate::setup::SILENT_FLAG]).spawn() {
                // Tachyon saves its downloads as it quits; the installer waits for that.
                Ok(_) => cx.quit(),
                Err(e) => set(UpdateState::Failed(format!("Couldn't start the update ({e})")), cx),
            },
            Err(e) => set(UpdateState::Failed(format!("The update couldn't be downloaded ({e})")), cx),
        })
        .ok();
    })
    .detach();
}

/// One line for Options: what the last check found.
pub fn status_line(cx: &App) -> String {
    match state(cx) {
        UpdateState::Idle => format!("Version {VERSION}"),
        UpdateState::Checking => "Checking for updates…".into(),
        UpdateState::UpToDate => format!("Tachyon {VERSION} is up to date."),
        UpdateState::Available(u) => format!("Tachyon {} is available.", u.version),
        UpdateState::Downloading(u, _) => format!("Downloading Tachyon {}…", u.version),
        UpdateState::Failed(e) => e,
    }
}

/// The bar above the download list while there's an update to tell about.
pub fn banner(theme: &Theme, cx: &App) -> Option<AnyElement> {
    let theme = *theme;
    let dismissed = cx.global::<AppState>().update_dismissed;
    let bar = |glyph: Icon, colour: gpui::Hsla| {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .py(px(7.))
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.accent_soft)
            .text_size(px(13.))
            .child(icon(glyph, px(16.), colour))
    };
    let later = || {
        Button::new("update-later", "Later").compact().on_click(|_, _, cx| {
            cx.global_mut::<AppState>().update_dismissed = true;
            cx.refresh_windows();
        })
    };
    match state(cx) {
        UpdateState::Available(update) if !dismissed => {
            let page = update.page.clone();
            Some(
                bar(Icon::ArrowDown, theme.icon)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("Tachyon {} is available", update.version))),
                    )
                    .child(Button::new("update-notes", "What's new").compact().on_click(move |_, _, cx| cx.open_url(&page)))
                    .child(Button::new("update-now", "Update").compact().primary().on_click(|_, _, cx| install(cx)))
                    .child(later())
                    .into_any_element(),
            )
        }
        UpdateState::Downloading(update, progress) => {
            let done = progress.load(Ordering::Relaxed);
            let fraction = (update.size > 0).then(|| done as f64 / update.size as f64);
            Some(
                bar(Icon::ArrowDown, theme.icon)
                    .child(div().child(format!("Downloading Tachyon {}…", update.version)))
                    .child(div().flex_1().min_w_0().child(progress_bar(fraction, px(4.), theme.accent, &theme)))
                    .into_any_element(),
            )
        }
        UpdateState::Failed(e) if !dismissed => Some(
            bar(Icon::Alert, theme.warning)
                .child(div().flex_1().min_w_0().child(e))
                .child(Button::new("update-retry", "Try again").compact().on_click(|_, _, cx| check(true, cx)))
                .child(later())
                .into_any_element(),
        ),
        _ => None,
    }
}
