//! IDM's "Download complete" dialog.

use std::sync::Arc;

use gpui::{
    AnyElement, AppContext, Context, Entity, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels,
    Point, Window, div, prelude::*, px,
};
use tachyon_engine::{DownloadId, DownloadSnapshot, Engine};

use crate::app;
use crate::assets::Icon;
use crate::file_icons::file_icon_element;
use crate::format;
use crate::platform::{self, DragOutcome};
use crate::theme::Theme;
use crate::widgets::text_input::TextInput;
use crate::widgets::{
    Button, Checkbox, Tooltip, card, field_row, field_row_inline, fit_window_height, icon, separator, truncated,
};

/// How far the pointer has to move with the button down before a drag starts.
const DRAG_THRESHOLD: f32 = 4.;

pub struct CompleteDialog {
    engine: Arc<Engine>,
    download: Option<DownloadSnapshot>,
    dont_show_again: bool,
    url_field: Entity<TextInput>,
    path_field: Entity<TextInput>,
    /// Where the mouse went down on the drag tile, until a drag starts.
    drag_from: Option<Point<Pixels>>,
    drag_note: Option<String>,
}

impl CompleteDialog {
    pub fn new(id: DownloadId, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let engine = app::engine(cx);
        let download = engine.get(id);
        let url = download.as_ref().map(|d| d.url.clone()).unwrap_or_default();
        let path = download.as_ref().map(|d| format::display_path(&d.path())).unwrap_or_default();
        Self {
            download,
            engine,
            dont_show_again: false,
            url_field: cx.new(|cx| TextInput::new(cx, "").read_only().with_text(url)),
            path_field: cx.new(|cx| TextInput::new(cx, "").read_only().with_text(path)),
            drag_from: None,
            drag_note: None,
        }
    }

    fn on_tile_mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(start) = self.drag_from else { return };
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag_from = None;
            return;
        }
        let delta = event.position - start;
        if f32::from(delta.x).abs().max(f32::from(delta.y).abs()) < DRAG_THRESHOLD {
            return;
        }
        self.drag_from = None;
        let (Some(handle), Some(path)) = (platform::native_handle(window), self.download.as_ref().map(|d| d.path()))
        else {
            return;
        };
        // The drag runs a modal loop, so it has to start after this event is handled.
        cx.spawn_in(window, async move |this, cx| {
            let outcome = platform::drag_file_out(handle, &path);
            this.update_in(cx, |this, window, cx| match outcome {
                Ok(DragOutcome::Moved) => this.close(window),
                Ok(DragOutcome::Copied) => {
                    this.drag_note = Some("Copied.".into());
                    cx.notify();
                }
                Ok(DragOutcome::Cancelled) => {}
                Err(e) => {
                    // The details are for the log; the user needs to know what to do.
                    log::warn!("dragging {} failed: {e}", path.display());
                    this.drag_note = Some(if path.exists() {
                        "Couldn't move the file. Use Open folder to move it in Explorer.".into()
                    } else {
                        "The file is no longer in this folder.".into()
                    });
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn close(&mut self, window: &mut Window) {
        if self.dont_show_again {
            let mut settings = self.engine.settings();
            settings.show_complete_dialog = false;
            if let Err(e) = self.engine.update_settings(settings) {
                log::warn!("could not save settings: {e}");
            }
        }
        window.remove_window();
    }
}

impl CompleteDialog {
    /// The IDM layout: exact size, copyable address and path, a tile to drag the
    /// file anywhere, and Open / Open with / Open folder / Close.
    fn render_fluent(&mut self, d: DownloadSnapshot, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let path = d.path();
        let size_line = match d.size {
            Some(size) => format!("{} ({} bytes)", format::bytes(size), format::thousands(size)),
            None => String::new(),
        };
        let label_width = 64.;
        // The file itself, in the same card as on the Download File Info dialog: drag it
        // anywhere. The hand cursor and tooltip say so, as Explorer does for files.
        let exists = path.exists();
        let accent = theme.accent;
        let file_item = card(theme)
            .id("drag-file")
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap(px(5.))
            .w(px(112.))
            .px(px(8.))
            .py(px(10.))
            .tooltip(Tooltip::text(if exists {
                "Drag the file anywhere to move it"
            } else {
                "The file is no longer in this folder"
            }))
            .when(exists, |this| {
                this.cursor(gpui::CursorStyle::OpenHand)
                    .hover(move |s| s.border_color(accent))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, _| this.drag_from = Some(event.position)),
                    )
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.drag_from = None))
                    .on_mouse_move(cx.listener(Self::on_tile_mouse_move))
            })
            .when(!exists, |this| this.opacity(0.45))
            .child(file_icon_element(&path, &d.category, px(32.), theme))
            .child(div().w_full().text_size(px(12.)).text_center().truncate().child(d.file_name.clone()))
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(d.size.map(format::bytes).unwrap_or_default()),
            );

        div()
            .size_full()
            .bg(theme.window)
            .text_color(theme.text)
            .text_size(px(13.))
            .child(fit_window_height(
                div()
                    .flex()
                    .flex_col()
                    .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .px(px(16.))
                    .py(px(14.))
                    .bg(theme.bg)
                    .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .pb(px(2.))
                            .child(icon(Icon::CheckCircle, px(28.), theme.success))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("Download complete"))
                                    .child(div().text_size(px(12.)).text_color(theme.muted).child(size_line))
                                    .when_some(self.drag_note.clone(), |this, note| {
                                        this.child(div().text_size(px(12.)).text_color(theme.warning).child(note))
                                    }),
                            ),
                    )
                    .child(field_row_inline("Address", label_width, self.url_field.clone(), theme))
                    .child(field_row_inline("Saved to", label_width, self.path_field.clone(), theme)),
                    )
                    .child(file_item),
                    ),
            )
            .child(
                // Buttons, then IDM's "Don't show this dialog again" at the very bottom.
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(px(10.))
                    .px(px(16.))
                    .py(px(10.))
                    .bg(theme.chrome)
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(Button::new("open", "Open").primary().fill().on_click(cx.listener({
                        let path = path.clone();
                        move |this, _, window, cx| {
                            cx.open_with_system(&path);
                            this.close(window);
                        }
                    })))
                    .child(Button::new("open-with", "Open with…").fill().on_click(cx.listener({
                        let path = path.clone();
                        move |this, _, window, _| {
                            platform::open_with(&path);
                            this.close(window);
                        }
                    })))
                    .child(Button::new("folder", "Open folder").fill().on_click(cx.listener(
                        move |this, _, window, cx| {
                            cx.reveal_path(&path);
                            this.close(window);
                        },
                    )))
                            .child(
                                Button::new("close", "Close")
                                    .fill()
                                    .on_click(cx.listener(|this, _, window, _| this.close(window))),
                            ),
                    )
                    .child(
                        Checkbox::new("dont-show", "Don't show this dialog again", self.dont_show_again).on_toggle(
                            cx.listener(|this, checked: &bool, _, cx| {
                                this.dont_show_again = *checked;
                                cx.notify();
                            }),
                        ),
                    ),
            ),
            ))
            .into_any_element()
    }
}

impl Render for CompleteDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        let Some(d) = self.download.clone() else {
            return div().size_full().bg(theme.bg).into_any_element();
        };
        if theme.fluent {
            return self.render_fluent(d, &theme, cx);
        }
        let path = d.path();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .text_color(theme.text)
            .text_size(px(13.))
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .p(px(20.))
                    .child(icon(Icon::CheckCircle, px(40.), theme.success))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(4.))
                            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("Download complete"))
                            .child(field_row("Address", truncated(d.url.clone()), &theme))
                            .child(field_row("File name", truncated(d.file_name.clone()), &theme))
                            .child(field_row("Folder", truncated(format::display_path(&d.directory)), &theme))
                            .child(field_row("Size", d.size.map(format::bytes).unwrap_or_default(), &theme)),
                    ),
            )
            .child(
                div().px(px(20.)).pb(px(14.)).child(
                    Checkbox::new("dont-show", "Don't show this dialog again", self.dont_show_again).on_toggle(
                        cx.listener(|this, checked: &bool, _, cx| {
                            this.dont_show_again = *checked;
                            cx.notify();
                        }),
                    ),
                ),
            )
            .child(div().flex_1())
            .child(separator(&theme))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .px(px(20.))
                    .py(px(12.))
                    .bg(theme.chrome)
                    .child(Button::new("folder", format::REVEAL_LABEL).icon(Icon::Folder).on_click({
                        let path = path.clone();
                        cx.listener(move |this, _, window, cx| {
                            cx.reveal_path(&path);
                            this.close(window);
                        })
                    }))
                    .child(Button::new("close", "Close").on_click(cx.listener(|this, _, window, _| this.close(window))))
                    .child(Button::new("open", "Open").primary().icon(Icon::External).on_click(cx.listener(
                        move |this, _, window, cx| {
                            cx.open_with_system(&path);
                            this.close(window);
                        },
                    ))),
            )
            .into_any_element()
    }
}
