//! Tachyon installs itself: the same tachyon.exe, run as TachyonSetup.exe (or with
//! --setup), shows the installer in Tachyon's own look, copies itself into place
//! with the browser extension it carries, and registers with Windows. Run with
//! --uninstall (from Windows' Installed apps) it removes itself again.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    App, AppContext, Application, Bounds, Context, Entity, FontWeight, PathPromptOptions, SharedString, Window, div,
    prelude::*, px, size,
};
use tachyon_engine::{Engine, ThemeMode, WindowStyle};

use crate::app;
use crate::assets::{Assets, Icon};
use crate::theme::Theme;
use crate::widgets::text_input::TextInput;
use crate::widgets::{Button, Checkbox, fit_window_height, icon, progress_bar, title_bar};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/extension_files.rs"));
}

const PRODUCT: &str = "Tachyon Download Manager";
const UNINSTALL_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\TachyonDownloadManager";
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const SETUP_FLAG: &str = "--setup";
pub const UNINSTALL_FLAG: &str = "--uninstall";
/// Opens Options at the Browser page, for the installer's last step.
pub const BROWSER_SETUP_FLAG: &str = "--browser-setup";

/// Started as the installer: named TachyonSetup….exe, or with --setup.
pub fn is_setup_launch() -> bool {
    let named = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()))
        .is_some_and(|name| name.starts_with("tachyonsetup") || name.starts_with("tachyon-setup"));
    named || std::env::args().any(|a| a == SETUP_FLAG)
}

pub fn is_uninstall_launch() -> bool {
    std::env::args().any(|a| a == UNINSTALL_FLAG)
}

fn default_install_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("C:\\")).join("Programs").join("Tachyon")
}

/// Runs the installer (or the uninstaller) window until it is done.
pub fn run(uninstall: bool) {
    crate::theme::set_style(WindowStyle::LiquidGlass);
    crate::theme::set_mode(ThemeMode::System);
    Application::new().with_assets(Assets).run(move |cx: &mut App| {
        crate::widgets::text_input::bind_keys(cx);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let (title, height) = if uninstall { ("Uninstall Tachyon", 230.) } else { ("Tachyon Setup", 470.) };
        let bounds = Bounds::centered(None, size(px(560.), px(height)), cx);
        let opened = app::open_styled(app::dialog_options(title, bounds), cx, move |window, cx| {
            cx.new(|cx| SetupWindow::new(uninstall, window, cx))
        });
        if opened.is_err() {
            cx.quit();
        }
        cx.activate(true);
    });
}

#[derive(Clone, PartialEq)]
enum Page {
    Choose,
    Working,
    Done,
    Failed(String),
}

/// What the background work reports to the window.
#[derive(Default)]
struct Progress {
    fraction: f32,
    step: String,
    /// Some: finished; the error if it failed.
    finished: Option<Result<Vec<String>, String>>,
}

struct Choices {
    folder: PathBuf,
    start_menu: bool,
    desktop: bool,
    autostart: bool,
    video: bool,
}

pub struct SetupWindow {
    uninstall: bool,
    page: Page,
    folder: Entity<TextInput>,
    start_menu: bool,
    desktop: bool,
    autostart: bool,
    video: bool,
    /// Uninstalling: also delete the download list and settings.
    remove_data: bool,
    /// A version already installed, which this one updates.
    installed: Option<String>,
    progress: Arc<Mutex<Progress>>,
    /// Things that went wrong without stopping the install.
    notes: Vec<String>,
}

impl SetupWindow {
    fn new(uninstall: bool, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let installed = reg::read(UNINSTALL_KEY, "DisplayVersion");
        let folder_path = reg::read(UNINSTALL_KEY, "InstallLocation").map(PathBuf::from).unwrap_or_else(default_install_dir);
        let folder = cx.new(|cx| TextInput::new(cx, "Folder").with_text(crate::format::display_path(&folder_path)));
        Self {
            uninstall,
            page: Page::Choose,
            folder,
            start_menu: true,
            desktop: true,
            autostart: false,
            video: true,
            remove_data: false,
            installed,
            progress: Arc::new(Mutex::new(Progress::default())),
            notes: Vec::new(),
        }
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                this.update(cx, |this, cx| {
                    // A folder of its own inside the one picked, as installers do.
                    let path = if path.ends_with("Tachyon") { path } else { path.join("Tachyon") };
                    this.folder.update(cx, |i, cx| i.set_text(crate::format::display_path(&path), cx));
                })
                .ok();
            }
        })
        .detach();
    }

    /// Starts the work in the background and follows its progress.
    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.page = Page::Working;
        let progress = self.progress.clone();
        if self.uninstall {
            let remove_data = self.remove_data;
            std::thread::spawn(move || {
                let result = uninstall(remove_data, &progress);
                progress.lock().unwrap().finished = Some(result.map(|()| Vec::new()));
            });
        } else {
            let choices = Choices {
                folder: crate::format::expand_home(self.folder.read(cx).text().trim()),
                start_menu: self.start_menu,
                desktop: self.desktop,
                autostart: self.autostart,
                video: self.video,
            };
            std::thread::spawn(move || {
                let result = install(&choices, &progress);
                progress.lock().unwrap().finished = Some(result);
            });
        }
        cx.spawn_in(window, async move |this, cx| {
            loop {
                gpui::Timer::after(Duration::from_millis(80)).await;
                let done = this
                    .update(cx, |this, cx| {
                        let finished = this.progress.lock().unwrap().finished.take();
                        if let Some(result) = finished {
                            match result {
                                Ok(notes) => {
                                    this.notes = notes;
                                    this.page = Page::Done;
                                }
                                Err(e) => this.page = Page::Failed(e),
                            }
                        }
                        cx.notify();
                        this.page != Page::Working
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn installed_exe(&self, cx: &Context<Self>) -> PathBuf {
        crate::format::expand_home(self.folder.read(cx).text().trim()).join("tachyon.exe")
    }

    fn launch(&self, args: &[&str], cx: &mut Context<Self>) {
        if let Err(e) = std::process::Command::new(self.installed_exe(cx)).args(args).spawn() {
            log::warn!("could not start Tachyon: {e}");
        }
        cx.quit();
    }

    fn render_choose(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let box_row = |id: &'static str, label: SharedString, checked: bool, set: fn(&mut Self, bool)| {
            Checkbox::new(id, label, checked).on_toggle(cx.listener(move |this, v: &bool, _, cx| {
                set(this, *v);
                cx.notify();
            }))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .px(px(14.))
            .py(px(14.))
            .rounded(px(theme.radius_large))
            .bg(theme.layer)
            .border_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().w(px(76.)).flex_none().text_color(theme.muted).child("Install to"))
                    .child(div().flex_1().min_w_0().child(self.folder.clone()))
                    .child(Button::new("browse", "Browse…").on_click(cx.listener(|this, _, window, cx| this.browse(window, cx)))),
            )
            .child(
                div()
                    .flex()
                    .gap(px(24.))
                    .pl(px(84.))
                    .child(box_row("start-menu", "Start menu shortcut".into(), self.start_menu, |t, v| t.start_menu = v))
                    .child(box_row("desktop", "Desktop shortcut".into(), self.desktop, |t, v| t.desktop = v)),
            )
            .child(div().pl(px(84.)).child(box_row(
                "autostart",
                "Start with Windows, in the notification area".into(),
                self.autostart,
                |t, v| t.autostart = v,
            )))
            .child(
                div()
                    .pl(px(84.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(box_row("video", "Video downloads (YouTube and more), about 20 MB".into(), self.video, |t, v| {
                        t.video = v
                    }))
                    .child(
                        div()
                            .pl(px(24.))
                            .text_size(px(12.))
                            .text_color(theme.muted)
                            .child("The latest yt-dlp and QuickJS, kept up to date. Otherwise fetched on first use."),
                    ),
            )
            .into_any_element()
    }

    fn render_working(&self, theme: &Theme) -> gpui::AnyElement {
        let (fraction, step) = {
            let p = self.progress.lock().unwrap();
            (p.fraction, p.step.clone())
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .px(px(14.))
            .py(px(18.))
            .rounded(px(theme.radius_large))
            .bg(theme.layer)
            .border_1()
            .border_color(theme.border)
            .child(progress_bar(Some(fraction as f64), px(6.), theme.accent, theme))
            .child(div().text_color(theme.muted).child(step))
            .into_any_element()
    }

    fn render_message(&self, theme: &Theme, glyph: Icon, color: gpui::Hsla, heading: String, body: String) -> gpui::AnyElement {
        div()
            .flex()
            .gap(px(12.))
            .px(px(14.))
            .py(px(16.))
            .rounded(px(theme.radius_large))
            .bg(theme.layer)
            .border_1()
            .border_color(theme.border)
            .child(icon(glyph, px(26.), color))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.))
                    .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(heading))
                    .child(div().text_color(theme.muted).child(body))
                    .children(self.notes.iter().map(|n| div().text_size(px(12.)).text_color(theme.warning).child(n.clone()))),
            )
            .into_any_element()
    }

    fn buttons(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        match (&self.page, self.uninstall) {
            (Page::Choose, false) => vec![
                Button::new("install", if self.installed.is_some() { "Update" } else { "Install" })
                    .primary()
                    .on_click(cx.listener(|this, _, window, cx| this.start(window, cx)))
                    .into_any_element(),
                Button::new("cancel", "Cancel").on_click(|_, _, cx| cx.quit()).into_any_element(),
            ],
            (Page::Choose, true) => vec![
                Button::new("uninstall", "Uninstall")
                    .primary()
                    .on_click(cx.listener(|this, _, window, cx| this.start(window, cx)))
                    .into_any_element(),
                Button::new("cancel", "Cancel").on_click(|_, _, cx| cx.quit()).into_any_element(),
            ],
            (Page::Working, _) => Vec::new(),
            (Page::Done, false) => vec![
                Button::new("open", "Open Tachyon")
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| this.launch(&[], cx)))
                    .into_any_element(),
                Button::new("browser", "Set up the browser extension")
                    .on_click(cx.listener(|this, _, _, cx| this.launch(&[BROWSER_SETUP_FLAG], cx)))
                    .into_any_element(),
            ],
            (Page::Done, true) | (Page::Failed(_), _) => {
                vec![Button::new("close", "Close").primary().on_click(|_, _, cx| cx.quit()).into_any_element()]
            }
        }
    }
}

impl Render for SetupWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        let version = env!("CARGO_PKG_VERSION");
        let subtitle = match (&self.installed, self.uninstall) {
            (_, true) => "Removes Tachyon from this computer.".to_string(),
            (Some(old), false) if old != version => format!("Updates version {old} to {version}."),
            (Some(_), false) => format!("Reinstalls version {version}."),
            (None, false) => format!("Fast, light, open-source download manager \u{b7} version {version}"),
        };
        let body = match &self.page {
            Page::Choose if self.uninstall => div()
                .px(px(14.))
                .py(px(14.))
                .rounded(px(theme.radius_large))
                .bg(theme.layer)
                .border_1()
                .border_color(theme.border)
                .child(Checkbox::new("remove-data", "Also delete my download list and settings", self.remove_data).on_toggle(
                    cx.listener(|this, v: &bool, _, cx| {
                        this.remove_data = *v;
                        cx.notify();
                    }),
                ))
                .into_any_element(),
            Page::Choose => self.render_choose(&theme, cx),
            Page::Working => self.render_working(&theme),
            Page::Done if self.uninstall => self.render_message(
                &theme,
                Icon::Check,
                theme.success,
                "Tachyon is removed".into(),
                "Downloaded files stay where they are.".into(),
            ),
            Page::Done => self.render_message(
                &theme,
                Icon::Check,
                theme.success,
                "Tachyon is ready".into(),
                "Open it, and add the extension to your browser so it can take over downloads.".into(),
            ),
            Page::Failed(e) => self.render_message(&theme, Icon::Alert, theme.danger, "Something went wrong".into(), e.clone()),
        };
        let buttons = self.buttons(cx);
        let title = if self.uninstall { "Uninstall Tachyon" } else { "Tachyon Setup" };
        div()
            .size_full()
            .bg(theme.window)
            .text_color(theme.text)
            .text_size(px(13.))
            .child(fit_window_height(
                div()
                    .flex()
                    .flex_col()
                    .child(title_bar(title, false, window, &theme))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(4.))
                            .pt(px(8.))
                            .pb(px(16.))
                            .child(icon(Icon::Bolt, px(48.), theme.icon))
                            .child(div().text_size(px(19.)).font_weight(FontWeight::SEMIBOLD).child(PRODUCT))
                            .child(div().text_color(theme.muted).child(subtitle)),
                    )
                    .child(div().px(px(16.)).child(body))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(8.))
                            .px(px(16.))
                            .py(px(14.))
                            .when(!self.uninstall && self.page == Page::Choose, |this| {
                                this.child(
                                    div()
                                        .flex_1()
                                        .text_size(px(12.))
                                        .text_color(theme.muted)
                                        .child("Free and open source under the MIT license."),
                                )
                            })
                            .children(buttons),
                    ),
            ))
    }
}

fn report(progress: &Mutex<Progress>, fraction: f32, step: &str) {
    let mut p = progress.lock().unwrap();
    p.fraction = fraction;
    p.step = step.to_string();
}

/// Installs (or updates) Tachyon. Ok with notes about optional parts that failed.
fn install(choices: &Choices, progress: &Mutex<Progress>) -> Result<Vec<String>, String> {
    let mut notes = Vec::new();
    let dir = &choices.folder;
    let exe = dir.join("tachyon.exe");
    let me = std::env::current_exe().map_err(|e| e.to_string())?;

    report(progress, 0.05, "Closing Tachyon if it runs…");
    processes::stop_all(&exe);

    report(progress, 0.15, "Copying Tachyon…");
    std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    if !same_file(&me, &exe) {
        let temp = dir.join("tachyon.exe.new");
        std::fs::copy(&me, &temp).map_err(|e| format!("Couldn't copy Tachyon: {e}"))?;
        std::fs::rename(&temp, &exe).map_err(|e| format!("Couldn't replace Tachyon (is it still running?): {e}"))?;
    }

    report(progress, 0.3, "Writing the browser extension…");
    let extension = dir.join("extension");
    let _ = std::fs::remove_dir_all(&extension);
    for (relative, bytes) in embedded::EXTENSION_FILES {
        let path = extension.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, bytes).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    }

    report(progress, 0.4, "Adding shortcuts…");
    let start_menu = dirs::data_dir().map(|d| d.join("Microsoft\\Windows\\Start Menu\\Programs").join(format!("{PRODUCT}.lnk")));
    let desktop = dirs::desktop_dir().map(|d| d.join(format!("{PRODUCT}.lnk")));
    for (wanted, place) in [(choices.start_menu, start_menu), (choices.desktop, desktop)] {
        let Some(place) = place else { continue };
        if wanted {
            if let Err(e) = shortcut::create(&exe, &place) {
                notes.push(format!("The shortcut {} couldn't be made: {e}", place.display()));
            }
        } else {
            let _ = std::fs::remove_file(&place);
        }
    }

    report(progress, 0.5, "Registering with Windows…");
    let command = format!("\"{}\" {}", exe.display(), crate::platform::START_IN_TRAY_FLAG);
    if choices.autostart {
        reg::write(RUN_KEY, Some(PRODUCT), &command);
    } else {
        reg::delete_value(RUN_KEY, PRODUCT);
    }
    let size_kb = (dir_size(dir) / 1024).to_string();
    let exe_text = exe.display().to_string();
    for (name, value) in [
        ("DisplayName", PRODUCT.to_string()),
        ("DisplayVersion", env!("CARGO_PKG_VERSION").to_string()),
        ("Publisher", "thesaikodelik".to_string()),
        ("DisplayIcon", exe_text.clone()),
        ("InstallLocation", dir.display().to_string()),
        ("UninstallString", format!("\"{exe_text}\" {UNINSTALL_FLAG}")),
        ("URLInfoAbout", "https://github.com/thesaikodelik/Tachyon-Download-Manager".to_string()),
    ] {
        reg::write(UNINSTALL_KEY, Some(name), &value);
    }
    reg::write_dword(UNINSTALL_KEY, "EstimatedSize", size_kb.parse().unwrap_or(0));
    reg::write_dword(UNINSTALL_KEY, "NoModify", 1);
    reg::write_dword(UNINSTALL_KEY, "NoRepair", 1);

    if choices.video {
        report(progress, 0.6, "Getting the latest video tools (yt-dlp, QuickJS)…");
        match Engine::default_data_dir() {
            Some(data_dir) => {
                if let Err(e) = tachyon_engine::prepare_tools(&data_dir) {
                    notes.push(format!("Video tools will be fetched on first use instead ({e})."));
                }
            }
            None => notes.push("Video tools will be fetched on first use instead.".into()),
        }
    }
    report(progress, 1.0, "Done");
    Ok(notes)
}

/// Removes Tachyon; its own folder goes once this program has exited.
fn uninstall(remove_data: bool, progress: &Mutex<Progress>) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = me.parent().map(Path::to_path_buf).ok_or("no folder")?;
    report(progress, 0.1, "Closing Tachyon…");
    processes::stop_all(&dir.join("tachyon.exe"));
    report(progress, 0.3, "Removing shortcuts…");
    for place in [
        dirs::data_dir().map(|d| d.join("Microsoft\\Windows\\Start Menu\\Programs").join(format!("{PRODUCT}.lnk"))),
        dirs::desktop_dir().map(|d| d.join(format!("{PRODUCT}.lnk"))),
    ]
    .into_iter()
    .flatten()
    {
        let _ = std::fs::remove_file(place);
    }
    report(progress, 0.5, "Unregistering…");
    reg::delete_value(RUN_KEY, PRODUCT);
    reg::delete_value(RUN_KEY, "Tachyon Download Manager");
    reg::delete_tree(UNINSTALL_KEY);
    crate::native_host::unregister();
    if remove_data && let Some(data) = Engine::default_data_dir() {
        report(progress, 0.7, "Deleting the download list and settings…");
        let _ = std::fs::remove_dir_all(data);
    }
    report(progress, 0.9, "Removing Tachyon's folder…");
    // A running program can't delete itself: a short-lived helper does it once this
    // window is closed, trying every second for half a minute.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let dir = dir.display();
        let script = format!(
            "for /l %i in (1,1,30) do (ping 127.0.0.1 -n 2 > nul & rmdir /s /q \"{dir}\" 2> nul & if not exist \"{dir}\" exit)"
        );
        // raw_arg: cmd reads the line itself and doesn't understand Rust's \" escaping.
        let _ = std::process::Command::new("cmd").arg("/c").raw_arg(&script).creation_flags(0x0800_0000).spawn();
    }
    report(progress, 1.0, "Done");
    Ok(())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| {
            let path = e.path();
            if path.is_dir() { dir_size(&path) } else { e.metadata().map(|m| m.len()).unwrap_or(0) }
        })
        .sum()
}

/// Values under HKEY_CURRENT_USER.
mod reg {
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_DWORD, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegDeleteTreeW, RegGetValueW,
        RegSetKeyValueW,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn read(key: &str, name: &str) -> Option<String> {
        let (key, name) = (wide(key), wide(name));
        let mut buf = [0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        // SAFETY: the buffer size is passed in bytes.
        let status = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size)
        };
        (status == 0).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
    }

    pub fn write(key: &str, name: Option<&str>, value: &str) {
        let (key, value) = (wide(key), wide(value));
        let name = name.map(wide);
        // SAFETY: NUL-terminated strings; the length is in bytes and includes the NUL.
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            );
        }
    }

    pub fn write_dword(key: &str, name: &str, value: u32) {
        let (key, name) = (wide(key), wide(name));
        // SAFETY: a 4-byte value.
        unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_DWORD, (&raw const value).cast(), 4) };
    }

    pub fn delete_value(key: &str, name: &str) {
        let (key, name) = (wide(key), wide(name));
        // SAFETY: NUL-terminated strings.
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
    }

    pub fn delete_tree(key: &str) {
        let key = wide(key);
        // SAFETY: NUL-terminated string.
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr()) };
    }
}

/// Windows shortcuts (.lnk), made through the shell.
mod shortcut {
    use std::path::Path;

    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{HSTRING, Interface};

    pub fn create(target: &Path, place: &Path) -> Result<(), String> {
        // SAFETY: COM on this (installer worker) thread, with interfaces released on drop.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.message())?;
            link.SetPath(&HSTRING::from(target.as_os_str())).map_err(|e| e.message())?;
            if let Some(dir) = target.parent() {
                link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str())).map_err(|e| e.message())?;
            }
            link.SetDescription(&HSTRING::from("Fast, light, open-source download manager")).map_err(|e| e.message())?;
            let file: IPersistFile = link.cast().map_err(|e| e.message())?;
            if let Some(parent) = place.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            file.Save(&HSTRING::from(place.as_os_str()), true).map_err(|e| e.message())?;
        }
        Ok(())
    }
}

/// Stopping a running copy of Tachyon, which Windows won't let us overwrite.
mod processes {
    use std::path::Path;

    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, QueryFullProcessImageNameW,
        TerminateProcess, WaitForSingleObject,
    };

    /// Ends every process running `exe` (other than this one) and waits for it.
    pub fn stop_all(exe: &Path) {
        let Ok(wanted) = exe.canonicalize() else { return };
        // SAFETY: a process snapshot walked and closed here; handles closed after use.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot.is_null() || snapshot as isize == -1 {
                return;
            }
            let me = GetCurrentProcessId();
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
            let mut more = Process32FirstW(snapshot, &mut entry) != 0;
            while more {
                if entry.th32ProcessID != me {
                    let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | 0x0010_0000, 0, entry.th32ProcessID);
                    if !process.is_null() {
                        let mut name = [0u16; 1024];
                        let mut len = name.len() as u32;
                        if QueryFullProcessImageNameW(process, 0, name.as_mut_ptr(), &mut len) != 0 {
                            let path = std::path::PathBuf::from(String::from_utf16_lossy(&name[..len as usize]));
                            if path.canonicalize().is_ok_and(|p| p == wanted) {
                                TerminateProcess(process, 0);
                                WaitForSingleObject(process, 5000);
                            }
                        }
                        CloseHandle(process);
                    }
                }
                more = Process32NextW(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
        }
    }
}
