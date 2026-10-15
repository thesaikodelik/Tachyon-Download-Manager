//! The "Tachyon" mark in the toolbar and the About window it opens, which credits
//! the macOS original first.

use gpui::{AnyElement, App, AppContext, Context, FontWeight, Hsla, Window, div, prelude::*, px, size};

use crate::app;
use crate::assets::Icon;
use crate::theme::Theme;
use crate::widgets::{Button, Tooltip, fit_window_height, icon, separator, title_bar};

const ORIGINAL_AUTHOR: &str = "sonatina-dream";
const ORIGINAL_URL: &str = "https://github.com/sonatina-dream/Tachyon-Download-Manager";
const WINDOWS_AUTHOR: &str = "thesaikodelik";
const WINDOWS_URL: &str = "https://github.com/thesaikodelik/Tachyon-Download-Manager";
/// Support for the Windows edition; the original author's work is credited above it.
const SPONSOR_URL: &str = "https://github.com/sponsors/thesaikodelik";
const WORD: &str = "Tachyon";

const BOLT_SIZE: f32 = 18.;

/// The mark, whose bolt fills from the bottom with the overall progress of running
/// downloads, like a battery charging.
#[derive(Default)]
pub struct Brand {
    /// Overall progress of the running downloads (0 to 1); None when nothing runs.
    progress: Option<f32>,
}

impl Brand {
    pub fn set_progress(&mut self, progress: Option<f32>, cx: &mut Context<Self>) {
        let progress = progress.map(|p| p.clamp(0., 1.));
        // Only redraw for visible changes.
        let changed = match (self.progress, progress) {
            (Some(a), Some(b)) => (a - b).abs() > 0.002,
            (a, b) => a.is_some() != b.is_some(),
        };
        if changed {
            self.progress = progress;
            cx.notify();
        }
    }
}

/// The bolt: solid, or while downloading a faint outline-coloured bolt with the
/// filled part rising from the bottom.
fn bolt(progress: Option<f32>, color: Hsla) -> AnyElement {
    let Some(progress) = progress else {
        return icon(Icon::Bolt, px(BOLT_SIZE), color).into_any_element();
    };
    div()
        .relative()
        .flex_none()
        .size(px(BOLT_SIZE))
        .child(icon(Icon::Bolt, px(BOLT_SIZE), color.opacity(0.28)))
        .child(
            // A window onto the bottom of a full-colour bolt, as tall as the progress.
            div()
                .absolute()
                .left_0()
                .bottom_0()
                .w_full()
                .h(px(BOLT_SIZE * progress))
                .overflow_hidden()
                .child(div().absolute().left_0().bottom_0().child(icon(Icon::Bolt, px(BOLT_SIZE), color))),
        )
        .into_any_element()
}

impl Render for Brand {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        div()
            .id("brand")
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(32.))
            .px(px(8.))
            .cursor_pointer()
            .text_color(theme.icon)
            .on_click(|_, _, cx| show_about(cx))
            .child(bolt(self.progress, theme.icon))
            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(WORD))
    }
}

/// Opens the About window in the middle of the screen.
pub fn show_about(cx: &mut App) {
    let bounds = app::centered(size(px(340.), px(280.)), cx);
    app::open_styled(app::dialog_options("About Tachyon", bounds), cx, |_, cx| cx.new(|_| AboutDialog)).ok();
    cx.activate(true);
}

pub struct AboutDialog;

/// A person to credit; clicking opens their GitHub repository.
fn credit_row(id: &'static str, role: &'static str, name: &'static str, url: &'static str, theme: &Theme) -> impl IntoElement {
    let hover = theme.hover;
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .py(px(7.))
        .rounded(px(theme.radius))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .on_click(move |_, _, cx| cx.open_url(url))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(11.)).text_color(theme.muted).child(role))
                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(name)),
        )
        .child(icon(Icon::External, px(13.), theme.muted))
}

/// A small, optional "Buy me a coffee" link beside OK: support for the Windows
/// edition, as its tooltip says (the About window credits two people).
fn sponsor_link(theme: &Theme) -> impl IntoElement {
    let cup: Hsla = gpui::rgb(0xb7793e).into();
    let (muted, text) = (theme.muted, theme.text);
    div()
        .id("sponsor")
        .flex()
        .items_center()
        .gap(px(5.))
        .px(px(6.))
        .py(px(3.))
        .rounded(px(theme.radius))
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(muted)
        .hover(move |s| s.text_color(text))
        .tooltip(Tooltip::text("Supports the Windows edition by thesaikodelik"))
        .on_click(|_, _, cx| cx.open_url(SPONSOR_URL))
        .child(icon(Icon::Coffee, px(13.), cup))
        .child("Buy me a coffee")
}

impl Render for AboutDialog {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        div()
            .size_full()
            .bg(theme.window)
            .text_color(theme.text)
            .text_size(px(13.))
            .child(fit_window_height(
                div()
                    .flex()
                    .flex_col()
                    .child(title_bar("About Tachyon", false, window, &theme))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(4.))
                            .pt(px(10.))
                            .pb(px(12.))
                            .child(icon(Icon::Bolt, px(44.), theme.icon))
                            .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child("Tachyon Download Manager"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.muted)
                                    .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                            ),
                    )
                    .child(div().px(px(16.)).child(separator(&theme)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .px(px(8.))
                            .py(px(8.))
                            .child(credit_row("credit-original", "Created by (macOS)", ORIGINAL_AUTHOR, ORIGINAL_URL, &theme))
                            .child(credit_row("credit-windows", "Windows edition by", WINDOWS_AUTHOR, WINDOWS_URL, &theme)),
                    )

                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(px(16.))
                            .py(px(12.))
                            .border_t_1()
                            .border_color(theme.border)
                            .child(sponsor_link(&theme))
                            .child(Button::new("ok", "OK").primary().on_click(|_, window, _| window.remove_window())),
                    ),
            ))
    }
}
