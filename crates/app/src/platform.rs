//! Native window chrome that GPUI doesn't cover: Windows 11 backdrops (Mica or
//! Acrylic), a title bar that follows dark mode, and hiding the main window to the
//! notification area instead of closing it.
//!
//! Everything here is a no-op on other platforms.

use gpui::Window;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backdrop {
    /// Plain opaque window.
    None,
    /// A subtle tint taken from the desktop wallpaper.
    Mica,
    /// Frosted glass.
    Acrylic,
}

/// Whether this system can draw [`Backdrop::Mica`] and [`Backdrop::Acrylic`]
/// (Windows 11 22H2, build 22621, or later).
pub fn supports_backdrops() -> bool {
    imp::supports_backdrops()
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

/// Hides a window without destroying it, so the app keeps running in the tray.
pub fn hide_window(window: &Window) {
    imp::hide_window(window)
}

/// Shows a window hidden with [`hide_window`] and brings it to the front.
pub fn show_window(window: &Window) {
    imp::show_window(window)
}

#[cfg(windows)]
mod imp {
    use std::sync::OnceLock;

    use gpui::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Dwm::{
        DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
        DWMWA_USE_IMMERSIVE_DARK_MODE, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
    };
    use windows_sys::Win32::UI::Controls::MARGINS;
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
        *SUPPORTED.get_or_init(|| build_number().is_some_and(|build| build >= 22621))
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

    pub fn style_window(window: &mut Window, backdrop: Backdrop, dark: bool) -> bool {
        let Some(hwnd) = hwnd(window) else { return false };
        set_attribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, i32::from(dark));
        let active = backdrop != Backdrop::None && supports_backdrops();
        let kind = match backdrop {
            _ if !active => DWMSBT_NONE,
            Backdrop::Mica => DWMSBT_MAINWINDOW,
            _ => DWMSBT_TRANSIENTWINDOW,
        };
        // GPUI clears each frame to transparent, so once the frame extends over the
        // whole client area the system backdrop shows wherever the app paints nothing.
        let margins = MARGINS {
            cxLeftWidth: if active { -1 } else { 0 },
            cxRightWidth: if active { -1 } else { 0 },
            cyTopHeight: if active { -1 } else { 0 },
            cyBottomHeight: if active { -1 } else { 0 },
        };
        // SAFETY: `margins` outlives the call.
        unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins) };
        set_attribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, kind);
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

    pub fn hide_window(window: &Window) {
        if let Some(hwnd) = hwnd(window) {
            // SAFETY: `hwnd` belongs to a live GPUI window.
            unsafe { ShowWindow(hwnd, SW_HIDE) };
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

    pub fn style_window(_: &mut Window, _: Backdrop, _: bool) -> bool {
        false
    }

    pub fn sync_chrome(_: &mut Window, _: Backdrop, _: bool) {}

    pub fn native_handle(_: &Window) -> Option<usize> {
        None
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

    pub fn hide_window(_: &Window) {}

    pub fn show_window(_: &Window) {}
}
