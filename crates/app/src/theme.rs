use std::sync::atomic::{AtomicU8, Ordering};

use gpui::{Hsla, Window, WindowAppearance, rgb, rgba};
use tachyon_engine::{ThemeMode, WindowStyle};

use crate::platform::{self, Backdrop};

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn ca(hex: u32) -> Hsla {
    rgba(hex).into()
}

const CLEAR: u32 = 0x00000000;

/// The style every window is drawn with, as an index into [`WindowStyle::ALL`].
static STYLE: AtomicU8 = AtomicU8::new(CLASSIC as u8);

/// Index of `WindowStyle::Classic`, the last style.
const CLASSIC: usize = WindowStyle::ALL.len() - 1;

/// Light/dark override, as an index into [`ThemeMode::ALL`].
static MODE: AtomicU8 = AtomicU8::new(0);

pub fn set_mode(mode: ThemeMode) {
    let index = ThemeMode::ALL.iter().position(|m| *m == mode).unwrap_or(0);
    MODE.store(index as u8, Ordering::Relaxed);
}

fn mode() -> ThemeMode {
    ThemeMode::ALL[usize::from(MODE.load(Ordering::Relaxed)).min(ThemeMode::ALL.len() - 1)]
}

/// Applies the user's window style. Glass styles fall back to `Solid` where the
/// system can't draw backdrops, and only `Classic` exists off Windows. Windows pick
/// the change up on their next render.
pub fn set_style(style: WindowStyle) {
    let effective = match style {
        _ if !cfg!(windows) => WindowStyle::Classic,
        s if s.is_glass() && !platform::supports_backdrops() => WindowStyle::Solid,
        s => s,
    };
    let index = WindowStyle::ALL.iter().position(|s| *s == effective).unwrap_or(CLASSIC);
    STYLE.store(index as u8, Ordering::Relaxed);
}

pub fn style() -> WindowStyle {
    WindowStyle::ALL[usize::from(STYLE.load(Ordering::Relaxed)).min(WindowStyle::ALL.len() - 1)]
}

/// The system backdrop that goes with the current style.
pub fn backdrop() -> Backdrop {
    match style() {
        WindowStyle::Mica => Backdrop::Mica,
        WindowStyle::LiquidGlass | WindowStyle::LiquidSmoke => Backdrop::Blur,
        WindowStyle::Solid | WindowStyle::Classic => Backdrop::None,
    }
}

/// Colours and shapes for one appearance. Looked up per render so windows follow the
/// system's light/dark setting and the chosen style live.
#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    /// Behind everything; transparent when a system backdrop shows through.
    pub window: Hsla,
    /// Dialog bodies. Transparent over a backdrop: GPUI blends stacked translucent
    /// fills towards grey, so on glass only one translucent layer may cover a pixel.
    pub bg: Hsla,
    /// Lists and cards: the one translucent layer over a backdrop.
    pub layer: Hsla,
    /// Sidebar and secondary panels.
    pub surface: Hsla,
    /// Toolbar, table header and dialog footers.
    pub chrome: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub accent_soft: Hsla,
    pub on_accent: Hsla,
    pub hover: Hsla,
    pub pressed: Hsla,
    pub row_alt: Hsla,
    pub success: Hsla,
    pub danger: Hsla,
    pub warning: Hsla,
    pub track: Hsla,
    pub button: Hsla,
    pub button_hover: Hsla,
    /// Text boxes.
    pub input: Hsla,
    /// Text box with keyboard focus.
    pub input_focus: Hsla,
    /// Read-only value fields.
    pub input_readonly: Hsla,
    /// Thin light line along the top edge of glass panels; transparent otherwise.
    pub highlight: Hsla,
    /// Corner radius of buttons, inputs and other controls.
    pub radius: f32,
    /// Corner radius of cards and panels.
    pub radius_large: f32,
    /// Standard control height.
    pub control_height: f32,
    /// Windows 11 styles lay dialogs out as labelled boxes, like IDM.
    pub fluent: bool,
    /// Liquid Glass: floating, lit glass panels and capsule buttons.
    pub liquid: bool,
}

impl Theme {
    /// Tachyon's original light palette.
    fn classic_light() -> Self {
        Self {
            dark: false,
            window: c(0xffffff),
            bg: c(0xffffff),
            layer: c(0xffffff),
            surface: c(0xf4f4f6),
            chrome: c(0xf7f7f9),
            border: c(0xe3e3e8),
            border_strong: c(0xc9c9d0),
            text: c(0x1d1d1f),
            muted: c(0x6b6b72),
            faint: c(0xa1a1a8),
            accent: c(0x1f6feb),
            accent_hover: c(0x1a5fd0),
            accent_soft: ca(0x1f6feb24),
            on_accent: c(0xffffff),
            hover: ca(0x0000000b),
            pressed: ca(0x00000016),
            row_alt: c(0xfafafb),
            success: c(0x1f9d55),
            danger: c(0xd93025),
            warning: c(0xc77700),
            track: c(0xe7e7ec),
            button: c(0xffffff),
            button_hover: c(0xf2f2f5),
            input: c(0xffffff),
            input_focus: c(0xffffff),
            input_readonly: c(0xf4f4f6),
            highlight: ca(CLEAR),
            radius: 6.,
            radius_large: 8.,
            control_height: 28.,
            fluent: false,
            liquid: false,
        }
    }

    /// Tachyon's original dark palette.
    fn classic_dark() -> Self {
        Self {
            dark: true,
            window: c(0x1c1c1e),
            bg: c(0x1c1c1e),
            layer: c(0x1c1c1e),
            surface: c(0x232326),
            chrome: c(0x2a2a2d),
            border: c(0x37373b),
            border_strong: c(0x4a4a50),
            text: c(0xf2f2f5),
            muted: c(0xa0a0a8),
            faint: c(0x6c6c74),
            accent: c(0x3b82f6),
            accent_hover: c(0x5593f7),
            accent_soft: ca(0x3b82f633),
            on_accent: c(0xffffff),
            hover: ca(0xffffff0f),
            pressed: ca(0xffffff1c),
            row_alt: c(0x202023),
            success: c(0x34c27a),
            danger: c(0xff5f57),
            warning: c(0xf5a524),
            track: c(0x38383d),
            button: c(0x323236),
            button_hover: c(0x3b3b40),
            input: c(0x1c1c1e),
            input_focus: c(0x1c1c1e),
            input_readonly: c(0x232326),
            highlight: ca(CLEAR),
            radius: 6.,
            radius_large: 8.,
            control_height: 28.,
            fluent: false,
            liquid: false,
        }
    }

    /// Liquid Glass, as Windows "glass" themes do it: one smoky tint over a light
    /// blur, and outlined capsule controls with only a whisper of fill.
    fn liquid(dark: bool, smoke: bool) -> Self {
        let accent = accent_color(dark).unwrap_or(c(if dark { 0x60cdff } else { 0x005fb8 }));
        let (fg, bg) = if dark { (0xffffff00_u32, 0x000000_u32) } else { (0x00000000_u32, 0xffffff_u32) };
        // Foreground (white on dark, black on light) at an alpha.
        let ink = |alpha: u32| ca(fg | alpha);
        let _ = bg;
        Self {
            dark,
            // Clear glass barely tints what is behind; Smoke darkens or greys it.
            window: match (smoke, dark) {
                (false, true) => ca(0x1c1c2026),
                (false, false) => ca(0xffffff1f),
                (true, true) => ca(0x0e0e1073),
                (true, false) => ca(0xd9dbe073),
            },
            bg: ca(CLEAR),
            layer: if dark { ca(0xffffff08) } else { ca(0xffffff26) },
            surface: ca(CLEAR),
            chrome: ca(CLEAR),
            border: ink(if dark { 0x1f } else { 0x14 }),
            border_strong: ink(if dark { 0x38 } else { 0x26 }),
            text: if dark { c(0xffffff) } else { ca(0x000000e6) },
            muted: ink(if dark { 0xb8 } else { 0x99 }),
            faint: ink(if dark { 0x73 } else { 0x66 }),
            accent,
            accent_hover: accent.opacity(0.9),
            accent_soft: accent.opacity(0.2),
            on_accent: c(if dark { 0x000000 } else { 0xffffff }),
            hover: ink(if dark { 0x14 } else { 0x0d }),
            pressed: ink(if dark { 0x0a } else { 0x08 }),
            row_alt: ca(CLEAR),
            success: c(if dark { 0x6ccb5f } else { 0x0f7b0f }),
            danger: c(if dark { 0xff99a4 } else { 0xc42b1c }),
            warning: c(if dark { 0xfce100 } else { 0x9d5d00 }),
            track: ink(if dark { 0x26 } else { 0x1a }),
            button: ink(if dark { 0x0f } else { 0x08 }),
            button_hover: ink(if dark { 0x1f } else { 0x12 }),
            input: ink(if dark { 0x0d } else { 0x08 }),
            input_focus: ink(if dark { 0x1a } else { 0x0d }),
            input_readonly: ink(if dark { 0x08 } else { 0x05 }),
            highlight: if dark { ca(0xffffff40) } else { ca(0xffffffcc) },
            radius: 14.,
            radius_large: 16.,
            control_height: 28.,
            fluent: true,
            liquid: true,
        }
    }

    /// Windows 11 (Fluent) light palette. Over a backdrop, surfaces are translucent
    /// layers, as in File Explorer and Settings.
    fn fluent_light(style: WindowStyle) -> Self {
        let glass = style.is_glass();
        let liquid = style == WindowStyle::LiquidGlass;
        let accent = accent_color(false).unwrap_or(c(0x005fb8));
        let base = if glass { ca(CLEAR) } else { c(0xf3f3f3) };
        Self {
            dark: false,
            window: base,
            bg: if glass { ca(CLEAR) } else { c(0xffffff) },
            layer: match style {
                WindowStyle::LiquidGlass => ca(0xffffff73),
                WindowStyle::Mica => ca(0xffffffb3),
                _ => c(0xffffff),
            },
            surface: base,
            chrome: base,
            border: ca(0x0000000f),
            border_strong: ca(0x00000029),
            text: ca(0x000000e4),
            muted: ca(0x0000009b),
            faint: ca(0x00000072),
            accent,
            accent_hover: accent.opacity(0.9),
            accent_soft: accent.opacity(0.12),
            on_accent: c(0xffffff),
            hover: ca(0x00000009),
            pressed: ca(0x00000006),
            row_alt: ca(0x00000004),
            success: c(0x0f7b0f),
            danger: c(0xc42b1c),
            warning: c(0x9d5d00),
            track: ca(0x00000020),
            button: match style {
                WindowStyle::LiquidGlass => ca(0xffffff8c),
                WindowStyle::Mica => ca(0xffffffb3),
                _ => c(0xfbfbfb),
            },
            button_hover: if glass { ca(0xffffffe0) } else { c(0xf6f6f6) },
            input: match style {
                WindowStyle::LiquidGlass => ca(0xffffff8c),
                WindowStyle::Mica => ca(0xffffffb3),
                _ => c(0xffffff),
            },
            input_focus: c(0xffffff),
            input_readonly: c(0xf7f7f7),
            highlight: if liquid { ca(0xffffffb3) } else { ca(CLEAR) },
            radius: if liquid { 8. } else { 4. },
            radius_large: if liquid { 14. } else { 8. },
            control_height: 28.,
            fluent: true,
            liquid,
        }
    }

    /// Windows 11 (Fluent) dark palette.
    fn fluent_dark(style: WindowStyle) -> Self {
        let glass = style.is_glass();
        let liquid = style == WindowStyle::LiquidGlass;
        let accent = accent_color(true).unwrap_or(c(0x60cdff));
        let base = if glass { ca(CLEAR) } else { c(0x202020) };
        Self {
            dark: true,
            window: base,
            bg: if glass { ca(CLEAR) } else { c(0x272727) },
            layer: match style {
                WindowStyle::LiquidGlass => ca(0x2828285c),
                WindowStyle::Mica => ca(0x3a3a3a4d),
                _ => c(0x272727),
            },
            surface: base,
            chrome: base,
            border: ca(0xffffff12),
            border_strong: ca(0xffffff24),
            text: c(0xffffff),
            muted: ca(0xffffffc8),
            faint: ca(0xffffff8b),
            accent,
            accent_hover: accent.opacity(0.9),
            accent_soft: accent.opacity(0.16),
            on_accent: c(0x000000),
            hover: ca(0xffffff0f),
            pressed: ca(0xffffff0a),
            row_alt: ca(0xffffff05),
            success: c(0x6ccb5f),
            danger: c(0xff99a4),
            warning: c(0xfce100),
            track: ca(0xffffff28),
            button: if glass { ca(0xffffff0f) } else { c(0x2d2d2d) },
            button_hover: if glass { ca(0xffffff15) } else { c(0x323232) },
            input: if glass { ca(0xffffff0f) } else { c(0x2d2d2d) },
            input_focus: c(0x1f1f1f),
            input_readonly: c(0x2b2b2b),
            highlight: if liquid { ca(0xffffff2e) } else { ca(CLEAR) },
            radius: if liquid { 8. } else { 4. },
            radius_large: if liquid { 14. } else { 8. },
            control_height: 28.,
            fluent: true,
            liquid,
        }
    }

    pub fn for_appearance(appearance: WindowAppearance) -> Self {
        let dark = match mode() {
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
            ThemeMode::System => matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark),
        };
        match (style(), dark) {
            (WindowStyle::Classic, false) => Self::classic_light(),
            (WindowStyle::Classic, true) => Self::classic_dark(),
            (WindowStyle::LiquidGlass, dark) => Self::liquid(dark, false),
            (WindowStyle::LiquidSmoke, dark) => Self::liquid(dark, true),
            (style, false) => Self::fluent_light(style),
            (style, true) => Self::fluent_dark(style),
        }
    }

    /// The theme for `window`. Also keeps the native title bar and backdrop in step
    /// with the appearance and style, so every window that renders stays consistent.
    pub fn of(window: &mut Window) -> Self {
        let theme = Self::for_appearance(window.appearance());
        platform::sync_chrome(window, backdrop(), theme.dark);
        theme
    }
}

/// The user's Windows accent colour in the shade Fluent uses on this background:
/// "Dark 1" on light backgrounds and "Light 2" on dark ones, so contrast holds.
#[cfg(windows)]
fn accent_color(dark: bool) -> Option<Hsla> {
    use std::sync::OnceLock;
    static PALETTE: OnceLock<Option<[u32; 8]>> = OnceLock::new();
    let palette = (*PALETTE.get_or_init(read_accent_palette))?;
    Some(c(palette[if dark { 1 } else { 4 }]))
}

#[cfg(not(windows))]
fn accent_color(_: bool) -> Option<Hsla> {
    None
}

/// Reads `Explorer\Accent\AccentPalette`: eight RGBA colours from lightest to darkest.
#[cfg(windows)]
fn read_accent_palette() -> Option<[u32; 8]> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RegGetValueW};
    let key: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent\0".encode_utf16().collect();
    let value: Vec<u16> = "AccentPalette\0".encode_utf16().collect();
    let mut bytes = [0u8; 32];
    let mut len = bytes.len() as u32;
    // SAFETY: the strings are NUL-terminated and `len` is the buffer size in bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_BINARY,
            std::ptr::null_mut(),
            bytes.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if status != 0 || len != 32 {
        return None;
    }
    let mut palette = [0u32; 8];
    for (slot, px) in palette.iter_mut().zip(bytes.chunks_exact(4)) {
        *slot = u32::from(px[0]) << 16 | u32::from(px[1]) << 8 | u32::from(px[2]);
    }
    Some(palette)
}
