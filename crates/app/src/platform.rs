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
/// `backdrop` if the system supports it. Returns whether a backdrop is active, in
/// which case the window's content should leave its background translucent.
pub fn style_window(window: &mut Window, backdrop: Backdrop, dark: bool) -> bool {
    imp::style_window(window, backdrop, dark)
}

/// Calls [`style_window`] only when `backdrop` or `dark` changed for this window,
/// so it is cheap enough to run on every render.
pub fn sync_chrome(window: &mut Window, backdrop: Backdrop, dark: bool) {
    imp::sync_chrome(window, backdrop, dark)
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

    use super::Backdrop;

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

    use super::Backdrop;

    pub fn supports_backdrops() -> bool {
        false
    }

    pub fn style_window(_: &mut Window, _: Backdrop, _: bool) -> bool {
        false
    }

    pub fn sync_chrome(_: &mut Window, _: Backdrop, _: bool) {}

    pub fn hide_window(_: &Window) {}

    pub fn show_window(_: &Window) {}
}
