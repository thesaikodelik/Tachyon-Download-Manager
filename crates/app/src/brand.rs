//! The "Tachyon" mark in the toolbar: holographic on hover, and a click shows who
//! made it, the macOS original first.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt, ClipboardItem, Context, Corner, FontWeight, Hsla, SharedString, Window, anchored,
    deferred, div, hsla, linear_color_stop, linear_gradient, prelude::*, px, svg,
};

use crate::assets::Icon;
use crate::theme::Theme;
use crate::widgets::{Button, icon, menu_panel, separator, truncated};

const ORIGINAL_AUTHOR: &str = "sonatina-dream";
const ORIGINAL_URL: &str = "https://github.com/sonatina-dream/Tachyon-Download-Manager";
const WINDOWS_AUTHOR: &str = "thesaikodelik";
const WINDOWS_URL: &str = "https://github.com/thesaikodelik/Tachyon-Download-Manager";

#[derive(Default)]
pub struct Brand {
    hovered: bool,
    open: bool,
    copied: bool,
}

/// A colour on the holographic cycle: cyan, violet, pink, gold…
fn holo(t: f32, dark: bool, alpha: f32) -> Hsla {
    hsla((0.5 + t).fract(), 0.85, if dark { 0.72 } else { 0.5 }, alpha)
}

impl Brand {
    #[cfg(feature = "dev-snapshots")]
    pub(crate) fn dev_set(&mut self, hovered: bool, open: bool, cx: &mut Context<Self>) {
        self.hovered = hovered;
        self.open = open;
        cx.notify();
    }

    fn credit_row(
        &self,
        id: &'static str,
        role: &'static str,
        name: &'static str,
        url: &'static str,
        theme: &Theme,
    ) -> impl IntoElement {
        let hover = theme.hover;
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(8.))
            .py(px(6.))
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
                    .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(name))
                    .child(
                        div()
                            .w_full()
                            .text_size(px(11.))
                            .text_color(theme.muted)
                            .child(truncated(url.trim_start_matches("https://"))),
                    ),
            )
            .child(icon(Icon::External, px(13.), theme.muted))
    }

    fn render_card(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        menu_panel(theme)
            .id("brand-card")
            .w(px(320.))
            .mt(px(22.))
            .p(px(10.))
            .gap(px(6.))
            .text_color(theme.text)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.open = false;
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(8.))
                    .pt(px(4.))
                    .child(icon(Icon::Bolt, px(24.), theme.icon))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child("Tachyon Download Manager"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.muted)
                                    .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                            ),
                    ),
            )
            .child(div().py(px(2.)).child(separator(theme)))
            .child(self.credit_row("credit-original", "Created by (macOS)", ORIGINAL_AUTHOR, ORIGINAL_URL, theme))
            .child(self.credit_row("credit-windows", "Windows edition by", WINDOWS_AUTHOR, WINDOWS_URL, theme))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .px(px(8.))
                    .pt(px(6.))
                    .pb(px(2.))
                    .child(Button::new("open-github", "Open on GitHub").primary().fill().on_click(|_, _, cx| {
                        cx.open_url(WINDOWS_URL)
                    }))
                    .child(
                        Button::new("copy-link", if self.copied { "Copied" } else { "Copy link" })
                            .fill()
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(WINDOWS_URL.to_string()));
                                this.copied = true;
                                cx.notify();
                            })),
                    ),
            )
    }
}

impl Render for Brand {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        let lit = self.hovered || self.open;
        let dark = theme.dark;
        let cycle = Animation::new(Duration::from_millis(3200)).repeat();
        let bolt = svg().path(Icon::Bolt.path()).size(px(18.)).flex_none();
        let word = div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(SharedString::from("Tachyon"));
        div()
            .id("brand")
            .relative()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(32.))
            .px(px(10.))
            .mr(px(2.))
            .rounded_full()
            .cursor_pointer()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.hovered = *hovered;
                cx.notify();
            }))
            .on_click(cx.listener(|this, _, _, cx| {
                this.open = !this.open;
                this.copied = false;
                cx.notify();
            }))
            // Holographic foil: a glassy capsule with an iridescent sheen moving across it.
            .when(lit, |this| {
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .rounded_full()
                        .border_1()
                        .border_color(hsla(0., 0., 1., if dark { 0.25 } else { 0.7 }))
                        .with_animation("holo-foil", cycle.clone(), move |foil, t| {
                            foil.bg(linear_gradient(
                                90.,
                                linear_color_stop(holo(t, dark, 0.22), 0.),
                                linear_color_stop(holo(t + 0.45, dark, 0.22), 1.),
                            ))
                        }),
                )
            })
            .map(|this| {
                if lit {
                    this.child(bolt.with_animation("holo-bolt", cycle.clone(), move |bolt, t| {
                        bolt.text_color(holo(t, dark, 1.))
                    }))
                    .child(word.with_animation("holo-word", cycle.clone(), move |word, t| {
                        word.text_color(holo(t + 0.12, dark, 1.))
                    }))
                } else {
                    this.child(bolt.text_color(theme.icon)).child(word.text_color(theme.icon))
                }
            })
            .when(self.open, |this| {
                this.child(
                    deferred(
                        anchored()
                            .anchor(Corner::TopRight)
                            .snap_to_window_with_margin(px(8.))
                            .child(self.render_card(&theme, cx)),
                    )
                    .with_priority(1),
                )
            })
    }
}
