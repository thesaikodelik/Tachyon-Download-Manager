#![cfg_attr(target_os = "macos", allow(unexpected_cfgs))] // emitted by the objc macros

//! Development aid (feature `dev-snapshots`): when `$TACHYON_SNAPSHOT_DIR/trigger`
//! appears, every visible Tachyon window is saved there as a PNG. An app may capture
//! its own windows without the Screen Recording permission on macOS, and on Windows
//! `PrintWindow` captures only Tachyon's windows, never what is behind them, which
//! makes this usable from scripts and CI.

use std::path::PathBuf;
use std::time::Duration;

use gpui::App;

#[cfg(target_os = "macos")]
use mac::capture_all;
#[cfg(windows)]
use win::capture_all;

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{CString, c_void};
    use std::path::Path;

    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};

    type CFTypeRef = *const c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: CFTypeRef);
        fn CFURLCreateFromFileSystemRepresentation(
            allocator: CFTypeRef,
            buffer: *const u8,
            len: isize,
            is_directory: bool,
        ) -> CFTypeRef;
        fn CFStringCreateWithCString(allocator: CFTypeRef, c_str: *const i8, encoding: u32) -> CFTypeRef;
    }

    #[link(name = "ImageIO", kind = "framework")]
    unsafe extern "C" {
        fn CGImageDestinationCreateWithURL(url: CFTypeRef, kind: CFTypeRef, count: usize, options: CFTypeRef) -> CFTypeRef;
        fn CGImageDestinationAddImage(dest: CFTypeRef, image: CFTypeRef, properties: CFTypeRef);
        fn CGImageDestinationFinalize(dest: CFTypeRef) -> bool;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {}

    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const i8) -> *mut c_void;
    }

    type CaptureFn = unsafe extern "C" fn(CGRect, u32, u32, u32) -> CFTypeRef;

    const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
    const INCLUDING_WINDOW: u32 = 1 << 3;
    const BOUNDS_IGNORE_FRAMING: u32 = 1;
    const UTF8: u32 = 0x0800_0100;

    fn capture_window(number: u32, path: &Path) -> bool {
        unsafe {
            // Looked up at runtime: the SDK hides this deprecated symbol.
            let name = CString::new("CGWindowListCreateImage").unwrap();
            let sym = dlsym(RTLD_DEFAULT, name.as_ptr());
            if sym.is_null() {
                return false;
            }
            let capture: CaptureFn = std::mem::transmute(sym);
            let null = CGRect {
                x: f64::INFINITY,
                y: f64::INFINITY,
                w: 0.,
                h: 0.,
            };
            let image = capture(null, INCLUDING_WINDOW, number, BOUNDS_IGNORE_FRAMING);
            if image.is_null() {
                return false;
            }
            let bytes = path.as_os_str().as_encoded_bytes();
            let url = CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize, false);
            let png = CString::new("public.png").unwrap();
            let kind = CFStringCreateWithCString(std::ptr::null(), png.as_ptr(), UTF8);
            let dest = CGImageDestinationCreateWithURL(url, kind, 1, std::ptr::null());
            let ok = !dest.is_null() && {
                CGImageDestinationAddImage(dest, image, std::ptr::null());
                CGImageDestinationFinalize(dest)
            };
            for obj in [dest, kind, url, image] {
                if !obj.is_null() {
                    CFRelease(obj);
                }
            }
            ok
        }
    }

    /// Saves every visible window as `<dir>/<n>-<title>.png`.
    pub fn capture_all(dir: &Path) {
        unsafe {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let windows: *mut Object = msg_send![app, windows];
            let count: usize = msg_send![windows, count];
            for i in 0..count {
                let window: *mut Object = msg_send![windows, objectAtIndex: i];
                let visible: bool = msg_send![window, isVisible];
                if !visible {
                    continue;
                }
                let number: isize = msg_send![window, windowNumber];
                let title: *mut Object = msg_send![window, title];
                let utf8: *const i8 = msg_send![title, UTF8String];
                let title = if utf8.is_null() {
                    String::new()
                } else {
                    std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned()
                };
                let safe: String = title
                    .chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '-' })
                    .collect();
                let path = dir.join(format!("{number}-{safe}.png"));
                if !capture_window(number as u32, &path) {
                    log::warn!("snapshot of window {number} failed");
                }
            }
        }
    }

}

#[cfg(windows)]
mod win {
    use std::path::Path;

    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS,
        DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject,
    };
    use windows_sys::Win32::Storage::Xps::PrintWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
        PW_RENDERFULLCONTENT,
    };

    unsafe extern "system" fn collect(hwnd: HWND, list: LPARAM) -> BOOL {
        let mut pid = 0;
        // SAFETY: `list` is the `Vec` passed to `EnumWindows` below.
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == std::process::id() && IsWindowVisible(hwnd) != 0 {
                (*(list as *mut Vec<HWND>)).push(hwnd);
            }
        }
        1
    }

    fn title(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        // SAFETY: the length passed is the buffer's.
        let len = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        String::from_utf16_lossy(&buf[..len.max(0) as usize])
    }

    /// Renders the window, including its DirectX content, into BGRA pixels.
    fn capture(hwnd: HWND) -> Option<(u32, u32, Vec<u8>)> {
        // SAFETY: plain GDI calls; every object created is released before returning.
        unsafe {
            let mut rect: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rect);
            let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
            if w <= 0 || h <= 0 {
                return None;
            }
            let screen = GetDC(std::ptr::null_mut());
            let dc = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(dc, bitmap);
            let printed = PrintWindow(hwnd, dc, PW_RENDERFULLCONTENT) != 0;
            SelectObject(dc, old);
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut pixels = vec![0u8; (w * h * 4) as usize];
            let rows = GetDIBits(dc, bitmap, 0, h as u32, pixels.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS);
            DeleteObject(bitmap);
            DeleteDC(dc);
            ReleaseDC(std::ptr::null_mut(), screen);
            (printed && rows == h).then_some((w as u32, h as u32, pixels))
        }
    }

    fn save_png(path: &Path, width: u32, height: u32, mut bgra: Vec<u8>) -> std::io::Result<()> {
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut encoder = png::Encoder::new(file, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&bgra)?;
        Ok(())
    }

    /// Saves every visible window of this process as `<dir>/<hwnd>-<title>.png`.
    pub fn capture_all(dir: &Path) {
        let mut windows: Vec<HWND> = Vec::new();
        // SAFETY: `collect` only pushes into `windows`, which outlives the call.
        unsafe { EnumWindows(Some(collect), &raw mut windows as LPARAM) };
        for hwnd in windows {
            let safe: String = title(hwnd)
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '-' })
                .collect();
            let path = dir.join(format!("{}-{safe}.png", hwnd as usize));
            match capture(hwnd) {
                Some((w, h, pixels)) => {
                    if let Err(e) = save_png(&path, w, h, pixels) {
                        log::warn!("could not save {}: {e}", path.display());
                    }
                }
                None => log::warn!("snapshot of window {hwnd:?} failed"),
            }
        }
    }
}

/// Drives the UI from scripts: one command per line in `$TACHYON_SNAPSHOT_DIR/cmd`.
fn run_command(line: &str, cx: &mut App) {
    use crate::add_dialog::{AddDialog, AddPrefill};
    use crate::main_window::{ContextMenu, MainWindow, SelectAllRows};
    use crate::options_window::OptionsWindow;
    use crate::progress_window::ProgressWindow;
    use crate::scheduler_window::SchedulerWindow;

    log::info!("dev command: {line}");
    let (name, arg) = line.split_once(' ').unwrap_or((line, ""));
    let number = arg.trim().parse::<u64>().ok();
    let windows = cx.windows();
    match name {
        "main" => crate::app::open_main_window(cx),
        "options" => crate::app::open_options(cx),
        "scheduler" => crate::app::open_scheduler(cx),
        "about" => crate::brand::show_about(cx),
        "add" => crate::app::open_add_dialog(AddPrefill::Url(arg.trim().to_string()), cx),
        "progress" => crate::app::open_progress_window(number.unwrap_or(1), cx),
        "complete" => crate::app::open_complete_dialog(number.unwrap_or(1), cx),
        "start-queue" => crate::app::engine(cx).start_queue(),
        "pause-all" => crate::app::engine(cx).pause_all(),
        "resume" => {
            if let Err(e) = crate::app::engine(cx).start(number.unwrap_or(1)) {
                log::warn!("resume failed: {e}");
            }
        }
        "quit" => cx.quit(),
        "add-ok" => {
            for w in windows.iter().filter_map(|w| w.downcast::<AddDialog>()) {
                w.update(cx, |d, window, cx| d.check_address(window, cx)).ok();
            }
        }
        "add-start" | "add-later" => {
            for w in windows.iter().filter_map(|w| w.downcast::<AddDialog>()) {
                w.update(cx, |d, window, cx| d.add(name == "add-start", window, cx)).ok();
            }
        }
        "links-start" | "links-queue" | "links-all" => {
            for w in windows.iter().filter_map(|w| w.downcast::<crate::links_window::LinksWindow>()) {
                w.update(cx, |l, window, cx| l.dev_command(name, window, cx)).ok();
            }
        }
        "update-check" => crate::updater::check(true, cx),
        "update-install" => crate::updater::install(cx),
        "quality" => {
            for w in windows.iter().filter_map(|w| w.downcast::<AddDialog>()) {
                w.update(cx, |d, _, cx| d.dev_choose_quality(number.unwrap_or(0) as usize, cx)).ok();
            }
        }
        "add-category-menu" => {
            for w in windows.iter().filter_map(|w| w.downcast::<AddDialog>()) {
                w.update(cx, |d, _, cx| d.dev_toggle_category_menu(cx)).ok();
            }
        }
        "style" => {
            let style = match arg.trim() {
                "liquid" => tachyon_engine::WindowStyle::LiquidGlass,
                "smoke" => tachyon_engine::WindowStyle::LiquidSmoke,
                "solid" => tachyon_engine::WindowStyle::Solid,
                _ => tachyon_engine::WindowStyle::Classic,
            };
            crate::theme::set_style(style);
            cx.refresh_windows();
        }
        "mode" => {
            crate::theme::set_mode(match arg.trim() {
                "dark" => tachyon_engine::ThemeMode::Dark,
                "light" => tachyon_engine::ThemeMode::Light,
                _ => tachyon_engine::ThemeMode::System,
            });
            cx.refresh_windows();
        }
        // Opens About from inside the main window's update, as a click on the logo does.
        "about-from-main" => {
            for w in windows.iter().filter_map(|w| w.downcast::<MainWindow>()) {
                w.update(cx, |_, _, cx| crate::brand::show_about(cx)).ok();
            }
        }
        "toggle-tree" => {
            for w in windows.iter().filter_map(|w| w.downcast::<MainWindow>()) {
                w.update(cx, |m, _, cx| m.toggle_categories(cx)).ok();
            }
        }
        "progress-details" => {
            for w in windows.iter().filter_map(|w| w.downcast::<ProgressWindow>()) {
                w.update(cx, |p, window, cx| p.toggle_details(window, cx)).ok();
            }
        }
        "progress-tab" => {
            for w in windows.iter().filter_map(|w| w.downcast::<ProgressWindow>()) {
                w.update(cx, |p, _, cx| {
                    p.tab = number.unwrap_or(0) as usize;
                    cx.notify();
                })
                .ok();
            }
        }
        "options-tab" => {
            for w in windows.iter().filter_map(|w| w.downcast::<OptionsWindow>()) {
                w.update(cx, |o, _, cx| {
                    o.tab = number.unwrap_or(0) as usize;
                    cx.notify();
                })
                .ok();
            }
        }
        "scheduler-tab" => {
            for w in windows.iter().filter_map(|w| w.downcast::<SchedulerWindow>()) {
                w.update(cx, |s, _, cx| {
                    s.tab = number.unwrap_or(0) as usize;
                    cx.notify();
                })
                .ok();
            }
        }
        "select-all" | "select-first" | "menu" | "filter" => {
            for w in windows.iter().filter_map(|w| w.downcast::<MainWindow>()) {
                w.update(cx, |m, window, cx| match name {
                    "select-all" => m.select_all(&SelectAllRows, window, cx),
                    "select-first" => m.dev_select_first(cx),
                    "filter" => m.dev_filter(arg.trim(), cx),
                    _ => {
                        m.dev_select_first(cx);
                        m.context_menu = Some(ContextMenu {
                            position: gpui::point(gpui::px(420.), gpui::px(150.)),
                        });
                        cx.notify();
                    }
                })
                .ok();
            }
        }
        "close" => {
            for w in windows.iter().filter(|w| w.downcast::<MainWindow>().is_none()) {
                w.update(cx, |_, window, _| window.remove_window()).ok();
            }
        }
        other => log::warn!("unknown dev command {other}"),
    }
}

pub fn init(cx: &mut App) {
    let Some(dir) = std::env::var_os("TACHYON_SNAPSHOT_DIR").map(PathBuf::from) else {
        return;
    };
    log::info!("dev snapshots enabled in {}", dir.display());
    cx.spawn(async move |cx| {
        loop {
            gpui::Timer::after(Duration::from_millis(300)).await;
            let command = dir.join("cmd");
            if let Ok(text) = std::fs::read_to_string(&command) {
                let _ = std::fs::remove_file(&command);
                cx.update(|cx| {
                    for line in text.lines().filter(|l| !l.trim().is_empty()) {
                        run_command(line.trim(), cx);
                    }
                })
                .ok();
            }
            let trigger = dir.join("trigger");
            if trigger.exists() {
                let _ = std::fs::remove_file(&trigger);
                capture_all(&dir);
                let _ = std::fs::write(dir.join("done"), b"");
            }
        }
    })
    .detach();
}
