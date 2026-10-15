//! Native window chrome that GPUI doesn't cover: the light blur behind the glass
//! styles, a title bar that follows dark mode, and hiding the main window to the
//! notification area instead of closing it.
//!
//! Everything here is a no-op on other platforms.

use gpui::Window;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backdrop {
    /// Plain opaque window.
    None,
    /// A light blur that keeps what is behind the window recognisable (Liquid Glass).
    Blur,
}

/// Whether this system can draw [`Backdrop::Blur`] behind Tachyon's own drawing
/// (Windows 10 1809, build 17763, or later).
pub fn supports_backdrops() -> bool {
    imp::supports_backdrops()
}

/// Whether the user wants animations (Windows Settings → Accessibility → Visual
/// effects → Animation effects). Tachyon's motion stops when it is off.
pub fn animations_enabled() -> bool {
    imp::animations_enabled()
}

/// Windows 11 (build 22000 or later), which has the Segoe Fluent Icons font.
pub fn is_windows_11() -> bool {
    imp::is_windows_11()
}

/// Matches the native title bar to the app's light/dark appearance and applies
/// `backdrop` if the system supports it. Only touches the window when `backdrop`
/// or `dark` changed for it, so it is cheap enough to run on every render.
pub fn sync_chrome(window: &mut Window, backdrop: Backdrop, dark: bool) {
    imp::sync_chrome(window, backdrop, dark)
}

/// What happened to a file dragged out of Tachyon.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DragOutcome {
    /// The file now lives where it was dropped.
    Moved,
    /// The drop target made a copy (apps that only accept copies, like uploads).
    Copied,
    Cancelled,
}

/// The native handle of `window`, for calls that have to happen outside GPUI's
/// event handling (see [`drag_file_out`]).
pub fn native_handle(window: &Window) -> Option<usize> {
    imp::native_handle(window)
}

/// Lets the user drag `path` to Explorer, the desktop or another app, as with IDM's
/// "Download complete" dialog. Explorer moves the file (like Cut and Paste).
///
/// Blocks in a modal loop until the drop. Run it from a spawned task, never from an
/// event handler: GPUI keeps handling messages for its windows during the loop.
pub fn drag_file_out(handle: usize, path: &std::path::Path) -> Result<DragOutcome, String> {
    imp::drag_file_out(handle, path)
}

/// `path` as the shell expects it: absolute, with backslashes. Shell APIs reject
/// forward slashes ("The parameter is incorrect"), which users may type in paths.
pub fn shell_path(path: &std::path::Path) -> std::path::PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    if cfg!(windows) {
        std::path::PathBuf::from(absolute.to_string_lossy().replace('/', "\\"))
    } else {
        absolute
    }
}

/// Makes sure only one Tachyon runs per profile (`data_dir`), as two engines on the
/// same download list would overwrite each other's state. Returns false when another
/// instance already runs; it has then been asked to show its window, and this one
/// should exit. Elsewhere this always returns true.
pub fn claim_single_instance(data_dir: &std::path::Path) -> bool {
    imp::claim_single_instance(data_dir)
}

/// Whether another launch asked this instance to show its window since the last call.
pub fn take_show_request() -> bool {
    imp::take_show_request()
}

/// Starts Tachyon in the notification area when the user signs in (or stops doing so).
pub fn set_start_with_system(enabled: bool) -> Result<(), String> {
    imp::set_start_with_system(enabled)
}

/// The web browsers installed for this user, as (name, program) pairs, from the list
/// Windows keeps for its "Default apps" settings.
pub fn installed_browsers() -> Vec<(String, std::path::PathBuf)> {
    imp::installed_browsers()
}

/// Plays the system sound for a finished (or failed) download.
pub fn play_sound(success: bool) {
    imp::play_sound(success)
}

/// Command-line flag used when Windows starts Tachyon at sign-in.
pub const START_IN_TRAY_FLAG: &str = "--tray";

/// Shows Windows' "Open with" chooser for `path`.
pub fn open_with(path: &std::path::Path) {
    imp::open_with(path)
}

/// Whether `window` was opened without Windows' title bar, so Tachyon draws its own.
/// Measured rather than remembered, so it stays right when the style changes while
/// windows are open: Windows' caption is at least 30 pixels tall.
pub fn has_custom_title_bar(window: &Window) -> bool {
    imp::has_custom_title_bar(window)
}

/// Whether `window` is still cloaked by [`cloak`] (created but not yet shown).
pub fn is_cloaked(window: &Window) -> bool {
    imp::is_cloaked(window)
}

/// Moves `window` down by `dy` (up when negative), in logical pixels.
pub fn move_window_by(window: &Window, dy: gpui::Pixels) {
    imp::move_window_by(window, dy)
}

/// The bounds, in logical pixels, of the window with native `handle` if it is on
/// screen (not hidden in the notification area, not minimized).
pub fn shown_window_rect(handle: usize) -> Option<gpui::Bounds<gpui::Pixels>> {
    imp::shown_window_rect(handle)
}

/// Hides a window without destroying it, so the app keeps running in the tray.
pub fn hide_window(window: &Window) {
    imp::hide_window(window)
}

/// Shows a window hidden with [`hide_window`] and brings it to the front.
pub fn show_window(window: &Window) {
    imp::show_window(window)
}

/// Stops a window accepting files dropped on it. Tachyon has no use for them, and
/// GPUI 0.2's drop handler misreads the shell's file lists and crashes the app, as
/// when a finished download is dragged out over a Tachyon window.
pub fn refuse_file_drops(window: &Window) {
    imp::refuse_file_drops(window)
}

/// Lets links be dropped on `window` (from a browser's address bar or a page), as
/// on IDM's main window. Each dropped web address is collected for
/// [`take_dropped_links`].
pub fn accept_dropped_links(window: &Window) {
    imp::accept_dropped_links(window)
}

/// Web addresses dropped on Tachyon since the last call.
pub fn take_dropped_links() -> Vec<String> {
    std::mem::take(&mut *DROPPED_LINKS.lock().unwrap_or_else(|e| e.into_inner()))
}

static DROPPED_LINKS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// The web addresses in dropped text: one per line, http(s) only.
fn links_in(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("http://") || l.starts_with("https://"))
        .filter(|l| url::Url::parse(l).is_ok())
        .map(str::to_string)
        .collect()
}

/// Puts a window in front of every other app and gives it the keyboard, even while
/// another app (such as the browser that sent a download) is in front.
pub fn bring_to_front(window: &Window) {
    imp::bring_to_front(window)
}

/// Keeps a shown window off screen (it still renders) until uncloaked, so it
/// appears complete instead of building up in view.
pub fn cloak(window: &Window, cloaked: bool) {
    imp::cloak(window, cloaked)
}

#[cfg(windows)]
mod imp {
    use std::sync::OnceLock;

    use gpui::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOW, SetForegroundWindow, ShowWindow};

    use super::{Backdrop, DragOutcome};

    pub(super) fn hwnd(window: &Window) -> Option<HWND> {
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as HWND),
            _ => None,
        }
    }

    pub fn supports_backdrops() -> bool {
        static SUPPORTED: OnceLock<bool> = OnceLock::new();
        *SUPPORTED.get_or_init(|| build_number().is_some_and(|build| build >= 17763))
    }

    pub fn animations_enabled() -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW};
        let mut enabled: i32 = 1;
        // SAFETY: the out-parameter is a BOOL, as SPI_GETCLIENTAREAANIMATION expects.
        unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut enabled).cast(), 0) };
        enabled != 0
    }

    pub fn is_windows_11() -> bool {
        static WINDOWS_11: OnceLock<bool> = OnceLock::new();
        *WINDOWS_11.get_or_init(|| build_number().is_some_and(|build| build >= 22000))
    }

    /// The OS build number, read from the registry since `GetVersionEx` lies to
    /// unmanifested apps.
    fn build_number() -> Option<u32> {
        use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
        let key: Vec<u16> = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\0".encode_utf16().collect();
        let value: Vec<u16> = "CurrentBuildNumber\0".encode_utf16().collect();
        let mut buf = [0u16; 32];
        let mut len = std::mem::size_of_val(&buf) as u32;
        // SAFETY: the strings are NUL-terminated and `len` is the buffer size in bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut len,
            )
        };
        if status != 0 {
            return None;
        }
        let chars = (len as usize / 2).saturating_sub(1);
        String::from_utf16_lossy(&buf[..chars]).trim().parse().ok()
    }

    fn set_attribute(hwnd: HWND, attribute: i32, value: i32) {
        // SAFETY: `value` outlives the call and the size matches its type.
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                attribute as u32,
                (&raw const value).cast(),
                std::mem::size_of::<i32>() as u32,
            );
        }
    }

    /// Sets the window's "accent" through the undocumented (but long-stable)
    /// SetWindowCompositionAttribute; 3 is a plain blur behind the window, 0 is off.
    fn set_accent(hwnd: HWND, state: u32) {
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};

        #[repr(C)]
        struct AccentPolicy {
            state: u32,
            flags: u32,
            gradient_color: u32,
            animation_id: u32,
        }
        #[repr(C)]
        struct CompositionData {
            attribute: u32,
            data: *mut std::ffi::c_void,
            size: usize,
        }
        type SetWindowCompositionAttribute = unsafe extern "system" fn(HWND, *mut CompositionData) -> i32;
        const WCA_ACCENT_POLICY: u32 = 19;

        // SAFETY: the function is looked up by name and called with the layout
        // Windows has used for it since Windows 7.
        unsafe {
            let user32 = GetModuleHandleA(c"user32.dll".as_ptr().cast());
            if user32.is_null() {
                return;
            }
            let Some(function) = GetProcAddress(user32, c"SetWindowCompositionAttribute".as_ptr().cast()) else {
                return;
            };
            let function: SetWindowCompositionAttribute = std::mem::transmute(function);
            let mut accent = AccentPolicy { state, flags: 0, gradient_color: 0, animation_id: 0 };
            let mut data = CompositionData {
                attribute: WCA_ACCENT_POLICY,
                data: (&raw mut accent).cast(),
                size: size_of::<AccentPolicy>(),
            };
            function(hwnd, &mut data);
        }
    }

    pub fn style_window(window: &mut Window, backdrop: Backdrop, dark: bool) -> bool {
        let Some(hwnd) = hwnd(window) else { return false };
        set_attribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, i32::from(dark));
        let active = backdrop != Backdrop::None && supports_backdrops();
        // GPUI clears each frame to transparent; the blur accent shows behind it.
        set_accent(hwnd, if active && backdrop == Backdrop::Blur { 3 } else { 0 });
        active
    }

    pub fn sync_chrome(window: &mut Window, backdrop: Backdrop, dark: bool) {
        use std::cell::RefCell;
        use std::collections::HashMap;
        thread_local! {
            static APPLIED: RefCell<HashMap<usize, (Backdrop, bool)>> = RefCell::new(HashMap::new());
        }
        let Some(key) = hwnd(window).map(|h| h as usize) else { return };
        let state = (backdrop, dark);
        if APPLIED.with_borrow_mut(|applied| applied.insert(key, state) != Some(state)) {
            style_window(window, backdrop, dark);
        }
    }

    pub fn native_handle(window: &Window) -> Option<usize> {
        hwnd(window).map(|h| h as usize)
    }

    pub fn has_custom_title_bar(window: &Window) -> bool {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowRect};
        let Some(hwnd) = hwnd(window) else { return false };
        let (mut outer, mut inner): (RECT, RECT) = unsafe { (std::mem::zeroed(), std::mem::zeroed()) };
        // SAFETY: valid window handle and out-pointers.
        let ok = unsafe { GetWindowRect(hwnd, &mut outer) != 0 && GetClientRect(hwnd, &mut inner) != 0 };
        ok && (outer.bottom - outer.top) - (inner.bottom - inner.top) < 24
    }

    pub fn drag_file_out(handle: usize, path: &std::path::Path) -> Result<DragOutcome, String> {
        use std::mem::ManuallyDrop;

        use windows::Win32::Foundation::HWND as WinHwnd;
        use windows::Win32::System::Com::{
            DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
        };
        use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
        use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
        use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_MOVE, IDropSource};
        use windows::Win32::UI::Shell::{BHID_DataObject, IShellItem, SHCreateItemFromParsingName, SHDoDragDrop};
        use windows::core::{HSTRING, w};

        let err = |e: windows::core::Error| e.message();
        // SAFETY: COM calls on the UI thread, where GPUI has initialised OLE; the
        // HGLOBAL is handed to the data object, which frees it.
        unsafe {
            // The shell's own data object, so drops behave exactly as from Explorer.
            let path = &super::shell_path(path);
            let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).map_err(err)?;
            let data: IDataObject = item.BindToHandler(None, &BHID_DataObject).map_err(err)?;

            // Ask the target to move rather than copy, as Cut and Paste does.
            let format = RegisterClipboardFormatW(w!("Preferred DropEffect")) as u16;
            let global = GlobalAlloc(GMEM_MOVEABLE, std::mem::size_of::<u32>()).map_err(err)?;
            *GlobalLock(global).cast::<u32>() = DROPEFFECT_MOVE.0;
            let _ = GlobalUnlock(global);
            let etc = FORMATETC {
                cfFormat: format,
                ptd: std::ptr::null_mut(),
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
            };
            let medium = STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 { hGlobal: global },
                pUnkForRelease: ManuallyDrop::new(None),
            };
            data.SetData(&etc, &medium, true).map_err(err)?;

            let effect = SHDoDragDrop(
                Some(WinHwnd(handle as _)),
                &data,
                None::<&IDropSource>,
                DROPEFFECT_MOVE | DROPEFFECT_COPY,
            )
            .map_err(err)?;

            // Explorer usually moves the file itself ("optimised move"). Otherwise a
            // MOVE result means the target made a copy and expects us to delete ours.
            if !path.exists() {
                return Ok(DragOutcome::Moved);
            }
            if effect.contains(DROPEFFECT_MOVE) {
                std::fs::remove_file(path).map_err(|e| e.to_string())?;
                return Ok(DragOutcome::Moved);
            }
            Ok(if effect.contains(DROPEFFECT_COPY) { DragOutcome::Copied } else { DragOutcome::Cancelled })
        }
    }

    /// Name of a per-profile kernel object, so the dev profile and the real one
    /// never take each other for a second instance.
    fn object_name(kind: &str, data_dir: &std::path::Path) -> Vec<u16> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        data_dir.to_string_lossy().to_lowercase().hash(&mut hasher);
        format!("Local\\TachyonDownloadManager-{kind}-{:016x}\0", hasher.finish()).encode_utf16().collect()
    }

    static SHOW_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    pub fn claim_single_instance(data_dir: &std::path::Path) -> bool {
        use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
        use windows_sys::Win32::System::Threading::{CreateEventW, CreateMutexW, INFINITE, SetEvent, WaitForSingleObject};
        use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

        let mutex_name = object_name("instance", data_dir);
        let event_name = object_name("show", data_dir);
        // SAFETY: the names are NUL-terminated; the handles live as long as the process.
        unsafe {
            let mutex = CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr());
            let first = !mutex.is_null() && GetLastError() != ERROR_ALREADY_EXISTS;
            let event = CreateEventW(std::ptr::null(), 0, 0, event_name.as_ptr());
            if event.is_null() {
                return first;
            }
            if !first {
                // Let the running instance come to the front, then wake it.
                AllowSetForegroundWindow(ASFW_ANY);
                SetEvent(event);
                return false;
            }
            let event = event as usize;
            std::thread::Builder::new()
                .name("show-requests".into())
                .spawn(move || {
                    loop {
                        WaitForSingleObject(event as _, INFINITE);
                        SHOW_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                })
                .ok();
            true
        }
    }

    pub fn take_show_request() -> bool {
        SHOW_REQUESTED.swap(false, std::sync::atomic::Ordering::Relaxed)
    }

    pub fn play_sound(success: bool) {
        use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONASTERISK, MB_ICONHAND};
        // The sounds the user picked for "Asterisk" and "Critical Stop" in Windows.
        // SAFETY: plain Win32 call.
        unsafe { MessageBeep(if success { MB_ICONASTERISK } else { MB_ICONHAND }) };
    }

    pub fn installed_browsers() -> Vec<(String, std::path::PathBuf)> {
        use windows_sys::Win32::System::Registry::{
            HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW,
            RegGetValueW, RegOpenKeyExW,
        };

        fn wide(s: &str) -> Vec<u16> {
            s.encode_utf16().chain(Some(0)).collect()
        }
        /// The default value of `key\sub`.
        fn default_value(key: HKEY, sub: &str) -> Option<String> {
            let sub = wide(sub);
            let mut buf = [0u16; 1024];
            let mut size = (buf.len() * 2) as u32;
            // SAFETY: the buffer size is passed in bytes.
            let status = unsafe {
                RegGetValueW(key, sub.as_ptr(), std::ptr::null(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size)
            };
            (status == 0).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
        }
        /// The program in a command line such as `"C:\x\chrome.exe" --flag`.
        fn program_of(command: &str) -> Option<std::path::PathBuf> {
            let command = command.trim();
            let path = match command.strip_prefix('"') {
                Some(rest) => rest.split('"').next()?,
                None => {
                    let end = command.to_ascii_lowercase().find(".exe")? + 4;
                    &command[..end]
                }
            };
            Some(std::path::PathBuf::from(path))
        }

        let mut found: Vec<(String, std::path::PathBuf)> = Vec::new();
        let places = [
            (HKEY_CURRENT_USER, "Software\\Clients\\StartMenuInternet"),
            (HKEY_LOCAL_MACHINE, "Software\\Clients\\StartMenuInternet"),
            (HKEY_LOCAL_MACHINE, "Software\\WOW6432Node\\Clients\\StartMenuInternet"),
        ];
        for (root, path) in places {
            let mut key: HKEY = std::ptr::null_mut();
            // SAFETY: NUL-terminated path; the key is closed below.
            if unsafe { RegOpenKeyExW(root, wide(path).as_ptr(), 0, KEY_READ, &mut key) } != 0 {
                continue;
            }
            for index in 0.. {
                let mut name = [0u16; 256];
                let mut len = name.len() as u32;
                // SAFETY: `len` holds the buffer size in characters.
                let status = unsafe {
                    RegEnumKeyExW(
                        key,
                        index,
                        name.as_mut_ptr(),
                        &mut len,
                        std::ptr::null(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    )
                };
                if status != 0 {
                    break;
                }
                let sub = String::from_utf16_lossy(&name[..len as usize]);
                let Some(program) = default_value(key, &format!("{sub}\\shell\\open\\command")).as_deref().and_then(program_of)
                else {
                    continue;
                };
                let title = default_value(key, &sub).filter(|t| !t.is_empty()).unwrap_or(sub);
                let same = |p: &std::path::PathBuf| p.file_name().map(|n| n.to_ascii_lowercase()) == program.file_name().map(|n| n.to_ascii_lowercase());
                if program.is_file() && !found.iter().any(|(_, p)| same(p)) {
                    found.push((title, program));
                }
            }
            // SAFETY: opened above.
            unsafe { RegCloseKey(key) };
        }
        found
    }

    pub fn set_start_with_system(enabled: bool) -> Result<(), String> {
        use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW};

        let key: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run\0".encode_utf16().collect();
        let name: Vec<u16> = "Tachyon Download Manager\0".encode_utf16().collect();
        // SAFETY: NUL-terminated strings; the data length is in bytes and includes the NUL.
        let status = unsafe {
            if enabled {
                let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                let command: Vec<u16> = format!("\"{}\" {}\0", exe.display(), super::START_IN_TRAY_FLAG)
                    .encode_utf16()
                    .collect();
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    name.as_ptr(),
                    REG_SZ,
                    command.as_ptr().cast(),
                    (command.len() * 2) as u32,
                )
            } else {
                match RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) {
                    2 => 0, // ERROR_FILE_NOT_FOUND: already off
                    status => status,
                }
            }
        };
        if status == 0 { Ok(()) } else { Err(std::io::Error::from_raw_os_error(status as i32).to_string()) }
    }

    pub fn open_with(path: &std::path::Path) {
        // The documented way to show the "Open with" chooser for one file.
        let result = std::process::Command::new("rundll32.exe")
            .arg("shell32.dll,OpenAs_RunDLL")
            .arg(super::shell_path(path))
            .spawn();
        if let Err(e) = result {
            log::warn!("could not show Open with: {e}");
        }
    }

    pub fn is_cloaked(window: &Window) -> bool {
        use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
        let Some(hwnd) = hwnd(window) else { return false };
        let mut cloaked: u32 = 0;
        // SAFETY: the out-parameter is a DWORD, as DWMWA_CLOAKED expects.
        let status = unsafe {
            DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as u32, (&raw mut cloaked).cast(), size_of::<u32>() as u32)
        };
        status == 0 && cloaked != 0
    }

    pub fn move_window_by(window: &Window, dy: gpui::Pixels) {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowRect, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
        };
        let Some(hwnd) = hwnd(window) else { return };
        let dy = (f32::from(dy) * window.scale_factor()).round() as i32;
        // SAFETY: plain calls on a live window handle.
        unsafe {
            let mut rect: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut rect) != 0 {
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    rect.left,
                    rect.top + dy,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }

    pub fn shown_window_rect(handle: usize) -> Option<gpui::Bounds<gpui::Pixels>> {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsIconic, IsWindowVisible};
        let hwnd = handle as HWND;
        // SAFETY: plain queries; a stale handle just fails them.
        unsafe {
            if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
                return None;
            }
            let mut rect: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut rect) == 0 {
                return None;
            }
            let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.;
            let px = |v: i32| gpui::px(v as f32 / scale);
            Some(gpui::Bounds::new(
                gpui::point(px(rect.left), px(rect.top)),
                gpui::size(px(rect.right - rect.left), px(rect.bottom - rect.top)),
            ))
        }
    }

    pub fn hide_window(window: &Window) {
        if let Some(hwnd) = hwnd(window) {
            // SAFETY: `hwnd` belongs to a live GPUI window.
            unsafe { ShowWindow(hwnd, SW_HIDE) };
        }
    }

    /// Tachyon's own drop target: takes links, as text or as an Internet shortcut,
    /// and reads the shell's data properly (GPUI's handler crashes on it).
    #[windows::core::implement(windows::Win32::System::Ole::IDropTarget)]
    struct LinkDropTarget;

    /// The links in `data`, from the formats browsers and Explorer use for them.
    fn links_from(data: &windows::Win32::System::Com::IDataObject) -> Vec<String> {
        use windows::Win32::Foundation::HGLOBAL;
        use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
        use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
        use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
        use windows::Win32::System::Ole::{CF_UNICODETEXT, ReleaseStgMedium};
        use windows::core::w;

        // SAFETY: the medium's memory is locked while read and released afterwards.
        unsafe {
            let url_format = RegisterClipboardFormatW(w!("UniformResourceLocatorW")) as u16;
            for format in [url_format, CF_UNICODETEXT.0] {
                let etc = FORMATETC {
                    cfFormat: format,
                    ptd: std::ptr::null_mut(),
                    dwAspect: DVASPECT_CONTENT.0,
                    lindex: -1,
                    tymed: TYMED_HGLOBAL.0 as u32,
                };
                let Ok(mut medium) = data.GetData(&etc) else { continue };
                let global: HGLOBAL = medium.u.hGlobal;
                let pointer = GlobalLock(global).cast::<u16>();
                let mut text = String::new();
                if !pointer.is_null() {
                    let units = GlobalSize(global) / 2;
                    let slice = std::slice::from_raw_parts(pointer, units);
                    let end = slice.iter().position(|&u| u == 0).unwrap_or(units);
                    text = String::from_utf16_lossy(&slice[..end]);
                    let _ = GlobalUnlock(global);
                }
                ReleaseStgMedium(&mut medium);
                let links = super::links_in(&text);
                if !links.is_empty() {
                    return links;
                }
            }
        }
        Vec::new()
    }

    impl windows::Win32::System::Ole::IDropTarget_Impl for LinkDropTarget_Impl {
        fn DragEnter(
            &self,
            data: windows::core::Ref<windows::Win32::System::Com::IDataObject>,
            _keys: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
            _point: &windows::Win32::Foundation::POINTL,
            effect: *mut windows::Win32::System::Ole::DROPEFFECT,
        ) -> windows::core::Result<()> {
            use windows::Win32::System::Ole::{DROPEFFECT_LINK, DROPEFFECT_NONE};
            let wanted = data.ok().is_ok_and(|data| !links_from(data).is_empty());
            // SAFETY: OLE passes a valid out-pointer.
            unsafe { *effect = if wanted { DROPEFFECT_LINK } else { DROPEFFECT_NONE } };
            DRAG_HAS_LINKS.store(wanted, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }

        fn DragOver(
            &self,
            _keys: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
            _point: &windows::Win32::Foundation::POINTL,
            effect: *mut windows::Win32::System::Ole::DROPEFFECT,
        ) -> windows::core::Result<()> {
            use windows::Win32::System::Ole::{DROPEFFECT_LINK, DROPEFFECT_NONE};
            let wanted = DRAG_HAS_LINKS.load(std::sync::atomic::Ordering::Relaxed);
            // SAFETY: OLE passes a valid out-pointer.
            unsafe { *effect = if wanted { DROPEFFECT_LINK } else { DROPEFFECT_NONE } };
            Ok(())
        }

        fn DragLeave(&self) -> windows::core::Result<()> {
            Ok(())
        }

        fn Drop(
            &self,
            data: windows::core::Ref<windows::Win32::System::Com::IDataObject>,
            _keys: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
            _point: &windows::Win32::Foundation::POINTL,
            effect: *mut windows::Win32::System::Ole::DROPEFFECT,
        ) -> windows::core::Result<()> {
            use windows::Win32::System::Ole::{DROPEFFECT_LINK, DROPEFFECT_NONE};
            let links = data.ok().map(links_from).unwrap_or_default();
            // SAFETY: OLE passes a valid out-pointer.
            unsafe { *effect = if links.is_empty() { DROPEFFECT_NONE } else { DROPEFFECT_LINK } };
            super::DROPPED_LINKS.lock().unwrap_or_else(|e| e.into_inner()).extend(links);
            Ok(())
        }
    }

    static DRAG_HAS_LINKS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    pub fn accept_dropped_links(window: &Window) {
        use windows::Win32::System::Ole::{IDropTarget, RegisterDragDrop, RevokeDragDrop};
        let Some(hwnd) = hwnd(window) else { return };
        let hwnd = windows::Win32::Foundation::HWND(hwnd as _);
        let target: IDropTarget = LinkDropTarget.into();
        // SAFETY: OLE is initialised on this (GPUI's UI) thread; OLE keeps the target
        // alive until the window is destroyed or the target revoked.
        unsafe {
            let _ = RevokeDragDrop(hwnd);
            if let Err(e) = RegisterDragDrop(hwnd, &target) {
                log::warn!("could not accept dropped links: {e}");
            }
        }
    }

    pub fn refuse_file_drops(window: &Window) {
        if let Some(hwnd) = hwnd(window) {
            // SAFETY: undoes GPUI's RegisterDragDrop on a live window.
            let _ = unsafe { windows::Win32::System::Ole::RevokeDragDrop(windows::Win32::Foundation::HWND(hwnd as _)) };
        }
    }

    pub fn bring_to_front(window: &Window) {
        use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, HWND_NOTOPMOST, HWND_TOPMOST,
            SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos,
        };
        let Some(hwnd) = hwnd(window) else { return };
        // SAFETY: plain Win32 calls on a live GPUI window; the thread input is
        // detached again before returning.
        unsafe {
            // Above everything for a moment, so it shows even if focus is refused.
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW;
            SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, flags);
            SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, flags);
            // Windows lets only the app in front hand over focus: share its input
            // state for a moment, as if it had.
            let front = GetForegroundWindow();
            let front_thread = if front.is_null() { 0 } else { GetWindowThreadProcessId(front, std::ptr::null_mut()) };
            let ours = GetCurrentThreadId();
            let attached = front_thread != 0 && front_thread != ours && AttachThreadInput(ours, front_thread, 1) != 0;
            BringWindowToTop(hwnd);
            SetForegroundWindow(hwnd);
            if attached {
                AttachThreadInput(ours, front_thread, 0);
            }
        }
    }

    pub fn cloak(window: &Window, cloaked: bool) {
        use windows_sys::Win32::Graphics::Dwm::DWMWA_CLOAK;
        if let Some(hwnd) = hwnd(window) {
            set_attribute(hwnd, DWMWA_CLOAK, i32::from(cloaked));
        }
    }

    pub fn show_window(window: &Window) {
        if let Some(hwnd) = hwnd(window) {
            // SAFETY: `hwnd` belongs to a live GPUI window.
            unsafe {
                ShowWindow(hwnd, SW_SHOW);
                SetForegroundWindow(hwnd);
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use gpui::Window;

    use super::{Backdrop, DragOutcome};

    pub fn supports_backdrops() -> bool {
        false
    }

    pub fn is_windows_11() -> bool {
        false
    }

    pub fn animations_enabled() -> bool {
        true
    }

    pub fn style_window(_: &mut Window, _: Backdrop, _: bool) -> bool {
        false
    }

    pub fn sync_chrome(_: &mut Window, _: Backdrop, _: bool) {}

    pub fn native_handle(_: &Window) -> Option<usize> {
        None
    }

    pub fn has_custom_title_bar(_: &Window) -> bool {
        false
    }

    pub fn drag_file_out(_: usize, _: &std::path::Path) -> Result<DragOutcome, String> {
        Err("dragging files out is only supported on Windows".into())
    }

    pub fn open_with(_: &std::path::Path) {}

    pub fn claim_single_instance(_: &std::path::Path) -> bool {
        true
    }

    pub fn take_show_request() -> bool {
        false
    }

    pub fn play_sound(_: bool) {}

    pub fn set_start_with_system(_: bool) -> Result<(), String> {
        Ok(())
    }

    pub fn installed_browsers() -> Vec<(String, std::path::PathBuf)> {
        Vec::new()
    }

    pub fn hide_window(_: &Window) {}

    pub fn shown_window_rect(_: usize) -> Option<gpui::Bounds<gpui::Pixels>> {
        None
    }

    pub fn is_cloaked(_: &Window) -> bool {
        false
    }

    pub fn move_window_by(_: &Window, _: gpui::Pixels) {}

    pub fn show_window(_: &Window) {}

    pub fn bring_to_front(_: &Window) {}

    pub fn refuse_file_drops(_: &Window) {}

    pub fn accept_dropped_links(_: &Window) {}

    pub fn cloak(_: &Window, _: bool) {}
}
