//! A small dialog for things Tachyon has to tell the user: a download limit
//! reached, a threat found by the virus scanner…

use gpui::{App, AppContext, Context, FontWeight, SharedString, Window, div, prelude::*, px, size};

use crate::app;
use crate::assets::Icon;
use crate::theme::Theme;
use crate::widgets::{Button, fit_window_height, icon, title_bar};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Info,
    Warning,
}

pub struct MessageDialog {
    title: SharedString,
    kind: Kind,
    heading: SharedString,
    body: SharedString,
}

/// Shows a message with an OK button.
pub fn show(kind: Kind, title: &str, heading: impl Into<SharedString>, body: impl Into<SharedString>, cx: &mut App) {
    let heading = heading.into();
    let body = body.into();
    let window_title = SharedString::from(title.to_string());
    let bounds = app::centered(size(px(440.), px(150.)), cx);
    cx.open_window(app::dialog_options(title, bounds), |_, cx| cx.new(|_| MessageDialog { title: window_title, kind, heading, body }))
        .ok();
    cx.activate(true);
}

impl Render for MessageDialog {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        let (glyph, color) = match self.kind {
            Kind::Info => (Icon::Info, theme.accent),
            Kind::Warning => (Icon::Alert, theme.warning),
        };
        div()
            .size_full()
            .bg(theme.window)
            .text_color(theme.text)
            .text_size(px(13.))
            .child(fit_window_height(
                div()
                    .flex()
                    .flex_col()
                    .child(title_bar(self.title.clone(), false, window, &theme))
                    .child(
                        div()
                            .flex()
                            .gap(px(12.))
                            .px(px(18.))
                            .py(px(16.))
                            .bg(theme.bg)
                            .child(icon(glyph, px(28.), color))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(4.))
                                    .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(self.heading.clone()))
                                    .child(div().text_color(theme.muted).child(self.body.clone())),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .px(px(18.))
                            .py(px(10.))
                            .bg(theme.chrome)
                            .border_t_1()
                            .border_color(theme.border)
                            .child(
                                div().w(px(110.)).flex().child(
                                    Button::new("ok", "OK").primary().fill().on_click(|_, window, _| window.remove_window()),
                                ),
                            ),
                    ),
            ))
    }
}
