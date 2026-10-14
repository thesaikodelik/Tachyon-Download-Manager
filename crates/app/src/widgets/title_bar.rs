//! The title bar of the Windows 11 styles, drawn by Tachyon so it shares the
//! window's glass. Windows still handles it natively: dragging, double-click to
//! maximize, and snap layouts on the maximize button.

use std::sync::{Arc, OnceLock};

use gpui::{AnyElement, Image, ImageFormat, SharedString, Window, WindowControlArea, div, img, prelude::*, px, rgb};

use crate::platform;
use crate::theme::Theme;

const HEIGHT: f32 = 32.;
const BUTTON_WIDTH: f32 = 46.;

fn app_icon() -> Arc<Image> {
    static ICON: OnceLock<Arc<Image>> = OnceLock::new();
    ICON.get_or_init(|| {
        Arc::new(Image::from_bytes(ImageFormat::Png, include_bytes!("../../assets/windows/tray.png").to_vec()))
    })
    .clone()
}

/// Windows 11 has Segoe Fluent Icons; Windows 10 the same glyphs in Segoe MDL2 Assets.
fn glyph_font() -> &'static str {
    if platform::supports_backdrops() { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" }
}

fn caption_button(
    id: &'static str,
    glyph: &'static str,
    area: WindowControlArea,
    close: bool,
    theme: &Theme,
) -> impl IntoElement {
    let hover_bg = if close { rgb(0xc42b1c).into() } else { theme.hover };
    let text = theme.text;
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .w(px(BUTTON_WIDTH))
        .h_full()
        .font_family(glyph_font())
        .text_size(px(10.))
        .text_color(text)
        .window_control_area(area)
        .hover(move |s| {
            let s = s.bg(hover_bg);
            if close { s.text_color(rgb(0xffffff)) } else { s }
        })
        .child(glyph)
}

/// The title bar, or nothing when the window has Windows' own (Classic style, or a
/// window opened before the style changed).
pub fn title_bar(title: impl Into<SharedString>, maximizable: bool, window: &Window, theme: &Theme) -> AnyElement {
    if !platform::has_custom_title_bar(window) {
        return div().into_any_element();
    }
    let active = window.is_window_active();
    let maximize_glyph = if window.is_maximized() { "\u{E923}" } else { "\u{E922}" };
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(HEIGHT))
        .w_full()
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .items_center()
                .gap(px(10.))
                .pl(px(12.))
                .window_control_area(WindowControlArea::Drag)
                .child(img(app_icon()).size(px(16.)).flex_none())
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(if active { theme.text } else { theme.faint })
                        .truncate()
                        .child(title.into()),
                ),
        )
        .child(caption_button("caption-min", "\u{E921}", WindowControlArea::Min, false, theme))
        .when(maximizable, |this| {
            this.child(caption_button("caption-max", maximize_glyph, WindowControlArea::Max, false, theme))
        })
        .child(caption_button("caption-close", "\u{E8BB}", WindowControlArea::Close, true, theme))
        .into_any_element()
}
