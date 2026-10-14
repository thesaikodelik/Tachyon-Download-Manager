//! A small dialog for things Tachyon has to tell the user: a download limit
//! reached, a threat found by the virus scanner…

use std::rc::Rc;

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
    /// A button that does something, shown next to "Not now" instead of OK.
    action: Option<(SharedString, Rc<dyn Fn(&mut App)>)>,
}

/// Shows a message with an OK button.
pub fn show(kind: Kind, title: &str, heading: impl Into<SharedString>, body: impl Into<SharedString>, cx: &mut App) {
    let heading = heading.into();
    let body = body.into();
    let window_title = SharedString::from(title.to_string());
    let bounds = app::centered(size(px(440.), px(150.)), cx);
    app::open_styled(app::dialog_options(title, bounds), cx, |_, cx| {
        cx.new(|_| MessageDialog { title: window_title, kind, heading, body, action: None })
    })
    .ok();
    cx.activate(true);
}

/// Asks the user to do something: `action` runs if they press the `label` button.
pub fn ask(
    title: &str,
    heading: impl Into<SharedString>,
    body: impl Into<SharedString>,
    label: impl Into<SharedString>,
    action: impl Fn(&mut App) + 'static,
    cx: &mut App,
) {
    let heading = heading.into();
    let body = body.into();
    let window_title = SharedString::from(title.to_string());
    let action: Option<(SharedString, Rc<dyn Fn(&mut App)>)> = Some((label.into(), Rc::new(action)));
    let bounds = app::centered(size(px(440.), px(150.)), cx);
    app::open_styled(app::dialog_options(title, bounds), cx, |_, cx| {
        cx.new(|_| MessageDialog { title: window_title, kind: Kind::Info, heading, body, action })
    })
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
                            .gap(px(8.))
                            .px(px(18.))
                            .py(px(10.))
                            .bg(theme.chrome)
                            .border_t_1()
                            .border_color(theme.border)
                            .child(match self.action.clone() {
                                Some((label, action)) => div()
                                    .flex()
                                    .gap(px(8.))
                                    .child(Button::new("act", label).primary().on_click(move |_, window, cx| {
                                        window.remove_window();
                                        action(cx);
                                    }))
                                    .child(Button::new("not-now", "Not now").on_click(|_, window, _| window.remove_window()))
                                    .into_any_element(),
                                None => div()
                                    .w(px(110.))
                                    .flex()
                                    .child(Button::new("ok", "OK").primary().fill().on_click(|_, window, _| window.remove_window()))
                                    .into_any_element(),
                            }),
                    ),
            ))
    }
}
