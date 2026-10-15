//! "Add URL" → "Download File Info", IDM's two-step add flow in one window.

use std::sync::Arc;

use gpui::{
    AnyElement, AppContext, Context, Entity, Focusable, FontWeight, PathPromptOptions, SharedString,
    Subscription, Window, anchored, deferred, div, prelude::*, px, size,
};
use tachyon_engine::{
    AddRequest, Credentials, DownloadSnapshot, DownloadStatus, DuplicateAction, Engine, GENERAL, MediaInfo, MediaSpec,
    ProbeInfo, RequestSpec, clean_file_name, derive_file_name, extension_of,
};

use crate::app;
use crate::assets::Icon;
use crate::file_icons::file_icon_element;
use crate::format::{self, display_path, expand_home};
use crate::theme::Theme;
use crate::widgets::text_input::{InputEvent, TextInput};
use crate::widgets::{
    Button, Checkbox, card, dropdown_trigger, field_row_inline, fit_window_height, title_bar, icon, menu_item, menu_panel,
    truncated,
};

/// Width of the captions to the left of the fields on the Windows 11 layout.
const LABEL_WIDTH: f32 = 74.;
/// Width of the category drop-down and of the list it opens.
const CATEGORY_PICKER_WIDTH: f32 = 200.;
const CONTACTING: &str = "Contacting the server…";

pub enum AddPrefill {
    /// Use the clipboard if it holds a link.
    FromClipboard,
    Url(String),
    /// Handed over by the browser extension.
    Captured(Box<AddRequest>),
}

#[derive(PartialEq, Eq)]
enum Stage {
    EnterUrl,
    Probing,
    Info,
}

pub struct AddDialog {
    engine: Arc<Engine>,
    stage: Stage,
    url: Entity<TextInput>,
    /// The address on the info page: selectable and copyable, not editable.
    url_display: Entity<TextInput>,
    use_auth: bool,
    username: Entity<TextInput>,
    password: Entity<TextInput>,
    file_name: Entity<TextInput>,
    folder: Entity<TextInput>,
    description: Entity<TextInput>,
    category: String,
    category_chosen: bool,
    category_menu: bool,
    folder_chosen: bool,
    /// IDM's "Remember this path": make the chosen folder the category's default.
    remember_folder: bool,
    /// The same link is already in the list and the user is asked what to do.
    duplicate: Option<DownloadSnapshot>,
    /// The user chose to add the link again anyway.
    allow_duplicate: bool,
    /// Headers (referrer, cookies…) that came with a captured link.
    base_request: RequestSpec,
    captured_name: Option<String>,
    probe: Option<ProbeInfo>,
    probe_error: Option<String>,
    /// The address is a video page (YouTube…): its qualities, and the one picked.
    media: Option<MediaInfo>,
    media_choice: usize,
    quality_menu: bool,
    /// What the dialog says while it checks the address.
    probing_note: &'static str,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

fn is_http_url(text: &str) -> bool {
    url::Url::parse(text.trim())
        .map(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
        .unwrap_or(false)
}

impl AddDialog {
    pub fn new(prefill: AddPrefill, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let engine = app::engine(cx);
        let (initial_url, base_request, captured_name, description) = match prefill {
            AddPrefill::FromClipboard => {
                let clip = cx
                    .read_from_clipboard()
                    .and_then(|c| c.text())
                    .map(|t| t.trim().to_string())
                    .filter(|t| is_http_url(t))
                    .unwrap_or_default();
                (clip, RequestSpec::default(), None, String::new())
            }
            AddPrefill::Url(url) => (url, RequestSpec::default(), None, String::new()),
            AddPrefill::Captured(request) => {
                let request = *request;
                (request.request.url.clone(), request.request, request.file_name, request.description)
            }
        };
        let captured = !base_request.url.is_empty();

        let url = cx.new(|cx| TextInput::new(cx, "https://example.com/file.zip").with_text(initial_url));
        let url_display = cx.new(|cx| TextInput::new(cx, "").read_only());
        let username = cx.new(|cx| TextInput::new(cx, "User name"));
        let password = cx.new(|cx| TextInput::new(cx, "Password").masked());
        let file_name = cx.new(|cx| TextInput::new(cx, "File name"));
        let folder = cx.new(|cx| TextInput::new(cx, "Folder"));
        let description = cx.new(|cx| TextInput::new(cx, "Optional").with_text(description));

        let subscriptions = vec![
            cx.subscribe_in(&url, window, |this, _, event, window, cx| match event {
                InputEvent::Submit => this.check_address(window, cx),
                InputEvent::Cancel => window.remove_window(),
                InputEvent::Changed => {}
            }),
            cx.subscribe_in(&password, window, |this, _, event, window, cx| {
                if let InputEvent::Submit = event {
                    this.check_address(window, cx)
                }
            }),
            cx.subscribe_in(&file_name, window, |this, _, event, window, cx| match event {
                InputEvent::Changed => this.on_name_changed(cx),
                InputEvent::Submit => this.add(true, window, cx),
                InputEvent::Cancel => window.remove_window(),
            }),
            cx.subscribe_in(&folder, window, |this, _, event, window, cx| match event {
                InputEvent::Changed => this.folder_chosen = true,
                InputEvent::Submit => this.add(true, window, cx),
                InputEvent::Cancel => window.remove_window(),
            }),
            cx.subscribe_in(&description, window, |this, _, event, window, cx| match event {
                InputEvent::Submit => this.add(true, window, cx),
                InputEvent::Cancel => window.remove_window(),
                InputEvent::Changed => {}
            }),
        ];

        let mut this = Self {
            engine,
            stage: Stage::EnterUrl,
            url,
            url_display,
            use_auth: false,
            username,
            password,
            file_name,
            folder,
            description,
            category: GENERAL.to_string(),
            category_chosen: false,
            category_menu: false,
            folder_chosen: false,
            remember_folder: false,
            duplicate: None,
            allow_duplicate: false,
            base_request,
            captured_name,
            probe: None,
            probe_error: None,
            media: None,
            media_choice: 0,
            quality_menu: false,
            probing_note: CONTACTING,
            error: None,
            _subscriptions: subscriptions,
        };
        if captured {
            this.check_address(window, cx);
        } else {
            let url = this.url.clone();
            url.update(cx, |input, cx| input.select_all_text(cx));
            window.focus(&url.read(cx).focus_handle(cx));
            this.resize(window);
        }
        this
    }

    fn resize(&self, window: &mut Window) {
        let fluent = Theme::of(window).fluent;
        if fluent {
            // The Windows 11 layout sizes the window to its content (fit_window_height).
            return;
        }
        let height = match self.stage {
            Stage::EnterUrl | Stage::Probing if self.use_auth => if fluent { 208. } else { 330. },
            Stage::EnterUrl | Stage::Probing => if fluent { 136. } else { 230. },
            Stage::Info => if fluent { 280. } else { 450. },
        };
        window.resize(size(px(if fluent { 580. } else { 560. }), px(height)));
    }

    fn request_spec(&self, cx: &Context<Self>) -> RequestSpec {
        let mut spec = self.base_request.clone();
        spec.url = self.url.read(cx).text().trim().to_string();
        spec.credentials = self.use_auth.then(|| Credentials {
            username: self.username.read(cx).text(),
            password: self.password.read(cx).text(),
        });
        spec
    }

    /// Validates the address and asks the server about the file.
    pub(crate) fn check_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let spec = self.request_spec(cx);
        if !is_http_url(&spec.url) {
            self.error = Some("Enter a web address starting with http:// or https://".into());
            cx.notify();
            return;
        }
        self.error = None;
        self.stage = Stage::Probing;
        self.probing_note = CONTACTING;
        cx.notify();
        let probe = self.engine.probe(spec.clone());
        let engine = self.engine.clone();
        let already_video = spec.media.is_some();
        cx.spawn_in(window, async move |this, cx| {
            let result = probe.await;
            // A web page rather than a file: it may be a video page (YouTube and the
            // like), whose video downloads in a quality picked here.
            let page = !already_video
                && match &result {
                    Ok(info) => info.content_type.as_deref().is_some_and(|t| t.starts_with("text/html")),
                    Err(_) => true,
                };
            if page {
                this.update(cx, |this, cx| {
                    this.probing_note = "Looking for a video on this page…";
                    cx.notify();
                })
                .ok();
                if let Ok(info) = engine.media_formats(spec.url.clone()).await
                    && !info.choices.is_empty()
                {
                    this.update_in(cx, |this, window, cx| this.on_media(spec, info, window, cx)).ok();
                    return;
                }
            }
            this.update_in(cx, |this, window, cx| this.on_probed(spec, result, window, cx)).ok();
        })
        .detach();
    }

    fn on_probed(
        &mut self,
        spec: RequestSpec,
        result: Result<ProbeInfo, tachyon_engine::EngineError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = match &result {
            Ok(info) => {
                self.probe = Some(info.clone());
                self.probe_error = None;
                info.file_name.clone()
            }
            Err(e) => {
                self.probe = None;
                self.probe_error = Some(e.to_string());
                derive_file_name(None, &spec.url, None)
            }
        };
        let name = self.captured_name.clone().unwrap_or(name);
        // Room for the Resumable / Can't resume badge at the end of the address.
        let badge_space = match &result {
            Ok(info) if info.resumable => px(104.),
            Ok(_) => px(116.),
            Err(_) => px(0.),
        };
        self.url_display.update(cx, |input, cx| {
            input.set_text(spec.url.clone(), cx);
            input.set_trailing_space(badge_space, cx);
        });
        window.set_window_title("Download File Info");
        let (category, folder) = self.engine.default_target(&name);
        self.category = category;
        self.file_name.update(cx, |input, cx| input.set_text(name, cx));
        self.folder.update(cx, |input, cx| input.set_text(display_path(&folder), cx));
        self.folder_chosen = false;
        self.category_chosen = false;
        self.stage = Stage::Info;
        self.resize(window);
        let focus = self.file_name.read(cx).focus_handle(cx);
        window.focus(&focus);
        cx.notify();
    }

    fn on_name_changed(&mut self, cx: &mut Context<Self>) {
        if self.category_chosen || self.stage != Stage::Info {
            return;
        }
        let name = self.file_name.read(cx).text();
        let (category, folder) = self.engine.default_target(&name);
        if category != self.category {
            self.category = category;
            if !self.folder_chosen {
                self.folder.update(cx, |input, cx| input.set_text(display_path(&folder), cx));
                self.folder_chosen = false;
            }
            cx.notify();
        }
    }

    /// The address is a video page: offer its qualities, the best picked.
    fn on_media(&mut self, mut spec: RequestSpec, info: MediaInfo, window: &mut Window, cx: &mut Context<Self>) {
        self.media = Some(info);
        self.media_choice = 0;
        let probe = self.apply_quality(cx);
        self.captured_name = Some(probe.file_name.clone());
        spec.media = self.base_request.media.clone();
        self.on_probed(spec, Ok(probe), window, cx);
    }

    /// Makes the picked quality the download's request and the file's details.
    fn apply_quality(&mut self, cx: &mut Context<Self>) -> ProbeInfo {
        let Some(info) = self.media.as_ref() else { return ProbeInfo::default() };
        let Some(choice) = info.choices.get(self.media_choice) else { return ProbeInfo::default() };
        self.base_request.media = Some(MediaSpec { format: choice.format.clone(), size: choice.size, cookies: Vec::new() });
        let probe = ProbeInfo {
            final_url: self.url.read(cx).text().trim().to_string(),
            total: choice.size,
            resumable: true,
            file_name: format!("{}.{}", clean_file_name(&info.title), choice.ext),
            content_type: Some(if choice.ext == "m4a" { "audio/mp4" } else { "video/mp4" }.into()),
            ..Default::default()
        };
        self.probe = Some(probe.clone());
        probe
    }

    #[cfg(feature = "dev-snapshots")]
    pub(crate) fn dev_choose_quality(&mut self, index: usize, cx: &mut Context<Self>) {
        self.choose_quality(index, cx);
    }

    fn choose_quality(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.media.as_ref().is_none_or(|m| index >= m.choices.len()) {
            return;
        }
        self.media_choice = index;
        self.quality_menu = false;
        let probe = self.apply_quality(cx);
        // A name the user typed stays; only its extension follows the quality.
        let current = self.file_name.read(cx).text();
        let stem = current.rsplit_once('.').map(|(stem, _)| stem.to_string()).unwrap_or(current);
        let ext = extension_of(&probe.file_name).unwrap_or_else(|| "mp4".into());
        self.file_name.update(cx, |input, cx| input.set_text(format!("{stem}.{ext}"), cx));
        cx.notify();
    }

    /// The video's qualities, as a drop-down like the category's.
    fn render_quality_picker(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let theme_copy = *theme;
        let choices = self.media.as_ref().map(|m| m.choices.clone()).unwrap_or_default();
        let label = |c: &tachyon_engine::MediaChoice| match c.size {
            Some(size) => format!("{} \u{b7} {}", c.label, format::bytes(size)),
            None => c.label.clone(),
        };
        let current = choices.get(self.media_choice).map(label).unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .when(self.quality_menu, |this| {
                this.child(
                    deferred(
                        anchored().snap_to_window_with_margin(px(8.)).child(
                            menu_panel(&theme_copy)
                                .id("quality-menu")
                                .w(px(CATEGORY_PICKER_WIDTH))
                                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                    this.quality_menu = false;
                                    cx.notify();
                                }))
                                .children(choices.iter().enumerate().map(|(ix, c)| {
                                    menu_item(("quality", ix), None, label(c), ix == self.media_choice, &theme_copy)
                                        .on_click(cx.listener(move |this, _, _, cx| this.choose_quality(ix, cx)))
                                })),
                        ),
                    )
                    .with_priority(1),
                )
            })
            .child(
                dropdown_trigger("quality", Some(Icon::Video), current, self.quality_menu, theme).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.quality_menu = !this.quality_menu;
                        cx.notify();
                    }),
                ),
            )
    }

    fn choose_category(&mut self, category: String, cx: &mut Context<Self>) {
        self.category = category.clone();
        self.category_chosen = true;
        self.category_menu = false;
        if !self.folder_chosen {
            let folder = self.engine.folder_for_category(&category);
            self.folder.update(cx, |input, cx| input.set_text(display_path(&folder), cx));
            self.folder_chosen = false;
        }
        cx.notify();
    }

    fn browse_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                this.update(cx, |this, cx| {
                    this.folder.update(cx, |input, cx| input.set_text(display_path(&path), cx));
                    this.folder_chosen = true;
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    #[cfg(feature = "dev-snapshots")]
    pub(crate) fn dev_toggle_category_menu(&mut self, cx: &mut Context<Self>) {
        self.category_menu = !self.category_menu;
        cx.notify();
    }

    /// Handles a link that is already in the list, per Options → Downloads.
    /// Returns true when it was dealt with and nothing should be added.
    fn handle_duplicate(&mut self, start: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.allow_duplicate {
            return false;
        }
        let Some(existing) = self.engine.find_same(&self.request_spec(cx)) else {
            return false;
        };
        match self.engine.settings().duplicate_action {
            DuplicateAction::AddNumbered => false,
            DuplicateAction::Ask => {
                self.duplicate = Some(existing);
                cx.notify();
                true
            }
            DuplicateAction::Overwrite => {
                self.download_over(existing, start, window, cx);
                true
            }
            DuplicateAction::ShowExisting => {
                Self::show_existing(&existing, cx);
                window.remove_window();
                true
            }
        }
    }

    /// Downloads the existing entry again from scratch, replacing its file.
    fn download_over(&mut self, existing: DownloadSnapshot, start: bool, window: &mut Window, cx: &mut Context<Self>) {
        if existing.status.is_active() {
            self.error = Some("That download is running. Stop it first, or add the link again.".into());
            cx.notify();
            return;
        }
        if existing.status == DownloadStatus::Completed {
            let _ = std::fs::remove_file(existing.path());
        }
        let result = self.engine.redownload(existing.id).and_then(|()| {
            if start { self.engine.start(existing.id) } else { Ok(()) }
        });
        match result {
            Ok(()) => {
                if start && self.engine.settings().show_progress_window {
                    app::open_progress_window(existing.id, cx);
                }
                window.remove_window();
            }
            Err(e) => {
                self.error = Some(e.to_string());
                cx.notify();
            }
        }
    }

    fn show_existing(existing: &DownloadSnapshot, cx: &mut Context<Self>) {
        if existing.status.is_active() {
            app::open_progress_window(existing.id, cx);
        } else {
            app::open_main_window(cx);
        }
    }

    /// A quiet Windows-style info bar; the choices replace the dialog's buttons.
    fn render_duplicate(&self, existing: &DownloadSnapshot, theme: &Theme) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .py(px(9.))
            .rounded(px(theme.radius_large))
            .border_1()
            .border_color(theme.border)
            .bg(theme.layer)
            .child(icon(Icon::Info, px(16.), theme.accent))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("This link is already in your list"))
            .child(div().text_color(theme.muted).child(format::status_line(existing)))
    }

    /// Buttons while a duplicate is waiting for an answer.
    fn duplicate_buttons(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![
            Button::new("dup-again", "Add again")
                .primary()
                .fill()
                .on_click(cx.listener(|this, _, window, cx| {
                    this.allow_duplicate = true;
                    this.duplicate = None;
                    this.add(true, window, cx);
                }))
                .into_any_element(),
            Button::new("dup-over", "Download over it")
                .fill()
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some(existing) = this.duplicate.take() {
                        this.download_over(existing, true, window, cx);
                    }
                }))
                .into_any_element(),
            Button::new("dup-show", "Show existing")
                .fill()
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some(existing) = this.duplicate.take() {
                        Self::show_existing(&existing, cx);
                        window.remove_window();
                    }
                }))
                .into_any_element(),
            Button::new("cancel", "Cancel").fill().on_click(|_, window, _| window.remove_window()).into_any_element(),
        ]
    }

    pub(crate) fn add(&mut self, start: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.stage != Stage::Info {
            return;
        }
        if self.handle_duplicate(start, window, cx) {
            return;
        }
        let name = self.file_name.read(cx).text().trim().to_string();
        if name.is_empty() {
            self.error = Some("Enter a file name.".into());
            cx.notify();
            return;
        }
        let folder = self.folder.read(cx).text();
        if self.remember_folder
            && let Err(e) = self.engine.remember_folder(&self.category, expand_home(folder.trim()))
        {
            log::warn!("could not remember the folder: {e}");
        }
        let request = AddRequest {
            request: self.request_spec(cx),
            file_name: Some(name),
            directory: Some(expand_home(folder.trim())),
            category: Some(self.category.clone()),
            description: self.description.read(cx).text(),
            start,
            queue: !start,
            probed: self.probe.clone(),
        };
        match self.engine.add(request) {
            Ok(id) => {
                if start && self.engine.settings().show_progress_window {
                    app::open_progress_window(id, cx);
                }
                window.remove_window();
            }
            Err(e) => {
                self.error = Some(e.to_string());
                cx.notify();
            }
        }
    }

    fn labeled(label: &'static str, field: impl IntoElement, theme: &Theme) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(div().w(px(84.)).flex_none().text_color(theme.muted).child(label))
            .child(div().flex_1().min_w_0().child(field))
    }

    fn render_enter_url(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("Enter the address to download"))
            .child(Self::labeled("Address", self.url.clone(), theme))
            .child(
                div().pl(px(96.)).child(
                    Checkbox::new("use-auth", "Use authorization", self.use_auth).on_toggle(cx.listener(
                        |this, checked: &bool, window, cx| {
                            this.use_auth = *checked;
                            this.resize(window);
                            cx.notify();
                        },
                    )),
                ),
            )
            .when(self.use_auth, |this| {
                this.child(Self::labeled("Login", self.username.clone(), theme))
                    .child(Self::labeled("Password", self.password.clone(), theme))
            })
            .into_any_element()
    }

    fn render_info(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let url = self.url.read(cx).text();
        let (size_text, resume_ok) = match &self.probe {
            Some(info) => (
                info.total.map(format::bytes).unwrap_or_else(|| "Unknown size".into()),
                Some(info.resumable),
            ),
            None => ("Unknown size".into(), None),
        };
        let settings = self.engine.settings();
        let mut categories: Vec<String> = settings.categories.iter().map(|c| c.name.clone()).collect();
        categories.push(GENERAL.to_string());

        let info_line = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .py(px(10.))
            .rounded(px(8.))
            .bg(theme.surface)
            .child(icon(Icon::for_category(&self.category), px(28.), theme.accent))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(div().w_full().text_color(theme.muted).text_size(px(12.)).child(truncated(url)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().font_weight(FontWeight::MEDIUM).child(size_text))
                            .child(match resume_ok {
                                Some(true) => div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.))
                                    .text_size(px(12.))
                                    .text_color(theme.success)
                                    .child(icon(Icon::Check, px(12.), theme.success))
                                    .child("Resume supported"),
                                Some(false) => div()
                                    .text_size(px(12.))
                                    .text_color(theme.warning)
                                    .child("Resume not supported · single connection"),
                                None => div(),
                            }),
                    ),
            );

        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("Download File Info"))
            .child(info_line)
            .when_some(self.probe_error.as_ref(), |this, e| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(theme.warning)
                        .child(icon(Icon::Alert, px(14.), theme.warning))
                        .child(format!("Couldn't check the file ({e}). You can still try downloading it.")),
                )
            })
            .child(Self::labeled(
                "Category",
                div().flex().flex_wrap().gap(px(6.)).children(categories.into_iter().map(|c| {
                    let active = c == self.category;
                    let theme = *theme;
                    div()
                        .id(SharedString::from(format!("cat-{c}")))
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .h(px(24.))
                        .px(px(9.))
                        .rounded(px(12.))
                        .border_1()
                        .text_size(px(12.))
                        .cursor_pointer()
                        .border_color(if active { theme.accent } else { theme.border_strong })
                        .bg(if active { theme.accent_soft } else { theme.bg })
                        .text_color(if active { theme.accent } else { theme.text })
                        .child(icon(Icon::for_category(&c), px(12.), if active { theme.accent } else { theme.muted }))
                        .child(c.clone())
                        .on_click(cx.listener(move |this, _, _, cx| this.choose_category(c.clone(), cx)))
                })),
                theme,
            ))
            .child(Self::labeled(
                "Save to",
                div()
                    .flex()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().child(self.folder.clone()))
                    .child(
                        Button::new("browse", "Choose…")
                            .icon(Icon::Folder)
                            .on_click(cx.listener(|this, _, window, cx| this.browse_folder(window, cx))),
                    ),
                theme,
            ))
            .child(Self::labeled("File name", self.file_name.clone(), theme))
            .child(Self::labeled("Description", self.description.clone(), theme))
            .into_any_element()
    }

    fn render_enter_url_fluent(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(field_row_inline("Address", LABEL_WIDTH, self.url.clone(), theme))
            .child(
                div().pl(px(LABEL_WIDTH + 10.)).child(
                    Checkbox::new("use-auth", "Use authorization", self.use_auth).on_toggle(cx.listener(
                        |this, checked: &bool, window, cx| {
                            this.use_auth = *checked;
                            this.resize(window);
                            cx.notify();
                        },
                    )),
                ),
            )
            .when(self.use_auth, |this| {
                this.child(field_row_inline("User name", LABEL_WIDTH, self.username.clone(), theme))
                    .child(field_row_inline("Password", LABEL_WIDTH, self.password.clone(), theme))
            })
            .into_any_element()
    }

    /// Category drop-down with its menu, which opens below the trigger.
    fn render_category_picker(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.engine.settings();
        let mut categories: Vec<String> = settings.categories.iter().map(|c| c.name.clone()).collect();
        categories.push(GENERAL.to_string());
        let theme_copy = *theme;
        div()
            .flex()
            .flex_col()
            // As in Windows 11, the list opens over the drop-down itself, aligned with
            // its top and exactly as wide, so it never sits half on and half off it.
            .when(self.category_menu, |this| {
                this.child(
                    deferred(
                        anchored().snap_to_window_with_margin(px(8.)).child(
                            menu_panel(&theme_copy)
                                .id("category-menu")
                                .w(px(CATEGORY_PICKER_WIDTH))
                                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                    this.category_menu = false;
                                    cx.notify();
                                }))
                                .children(categories.into_iter().map(|c| {
                                    let selected = c == self.category;
                                    menu_item(
                                        SharedString::from(format!("cat-{c}")),
                                        Some(Icon::for_category(&c)),
                                        c.clone(),
                                        selected,
                                        &theme_copy,
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| this.choose_category(c.clone(), cx)))
                                })),
                        ),
                    )
                    .with_priority(1),
                )
            })
            .child(
                dropdown_trigger(
                    "category",
                    Some(Icon::for_category(&self.category)),
                    self.category.clone(),
                    self.category_menu,
                    theme,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.category_menu = !this.category_menu;
                    cx.notify();
                })),
            )
    }


    /// The address, with whether it can be resumed at its end. The address stops
    /// short of the badge (see `on_probed`), so the two never overlap.
    fn render_url_field(&self, theme: &Theme) -> impl IntoElement {
        let badge = match self.probe.as_ref().map(|info| info.resumable) {
            Some(true) => Some((Icon::Check, "Resumable", theme.success)),
            Some(false) => Some((Icon::Alert, "Can't resume", theme.warning)),
            None => None,
        };
        // Pale tinted badges fade into glass, so on glass they are solid: white text on
        // light glass, dark text on Smoke (as Windows 11's dark badges).
        let solid_badges = theme.liquid;
        let on_badge: gpui::Hsla = if theme.dark { gpui::black().opacity(0.85) } else { gpui::white() };
        div().relative().child(self.url_display.clone()).when_some(badge, |this, (glyph, label, color)| {
            this.child(
                div()
                    .absolute()
                    .top(px(1.))
                    .bottom(px(1.))
                    .right(px(1.))
                    .flex()
                    .child(
                        div().h_full().flex().items_center().pr(px(5.)).child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .px(px(7.))
                                .py(px(1.))
                                .rounded_full()
                                .bg(if solid_badges { color } else { color.opacity(0.12) })
                                .text_size(px(11.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(if solid_badges { on_badge } else { color })
                                .child(icon(glyph, px(11.), if solid_badges { on_badge } else { color }))
                                .child(label),
                        ),
                    ),
            )
        })
    }

    /// The file at a glance, like the type icon and size IDM shows beside the form.
    fn render_file_card(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.file_name.read(cx).text();
        let extension = extension_of(&name).map(|e| e.to_uppercase()).unwrap_or_else(|| "FILE".into());
        let size_text = self
            .probe
            .as_ref()
            .and_then(|info| info.total)
            .map(format::bytes)
            .unwrap_or_else(|| "Unknown size".into());
        card(theme)
            .w(px(112.))
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(5.))
            .px(px(8.))
            .py(px(10.))
            .child(file_icon_element(
                &std::path::Path::new(&self.folder.read(cx).text()).join(&name),
                &self.category,
                px(32.),
                theme,
            ))
            .child(
                div()
                    .px(px(8.))
                    .py(px(1.))
                    .rounded(px(theme.radius))
                    .map(|this| {
                        if theme.liquid && !theme.dark {
                            this.bg(theme.icon).text_color(gpui::white())
                        } else {
                            this.bg(theme.accent_soft).text_color(theme.accent)
                        }
                    })
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(extension),
            )
            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(size_text))
    }

    fn render_info_fluent(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let remember_label = format!("Remember this folder for \u{201c}{}\u{201d}", self.category);
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(field_row_inline("URL", LABEL_WIDTH, self.render_url_field(theme), theme))
            .when(self.media.is_some(), |this| {
                this.child(field_row_inline(
                    "Quality",
                    LABEL_WIDTH,
                    div().w(px(CATEGORY_PICKER_WIDTH)).child(self.render_quality_picker(theme, cx)),
                    theme,
                ))
            })
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
                            .child(field_row_inline(
                                "Category",
                                LABEL_WIDTH,
                                div().w(px(CATEGORY_PICKER_WIDTH)).child(self.render_category_picker(theme, cx)),
                                theme,
                            ))
                            .child(field_row_inline(
                                "Save to",
                                LABEL_WIDTH,
                                div()
                                    .flex()
                                    .gap(px(6.))
                                    .child(div().flex_1().min_w_0().child(self.folder.clone()))
                                    .child(
                                        Button::new("browse", "Browse…")
                                            .on_click(cx.listener(|this, _, window, cx| this.browse_folder(window, cx))),
                                    ),
                                theme,
                            ))
                            .child(field_row_inline("File name", LABEL_WIDTH, self.file_name.clone(), theme))
                            .child(div().pl(px(LABEL_WIDTH + 10.)).child(
                                Checkbox::new("remember", remember_label, self.remember_folder).on_toggle(
                                    cx.listener(|this, checked: &bool, _, cx| {
                                        this.remember_folder = *checked;
                                        cx.notify();
                                    }),
                                ),
                            )),
                    )
                    .child(self.render_file_card(theme, cx)),
            )
            .child(field_row_inline("Description", LABEL_WIDTH, self.description.clone(), theme))
            .when_some(self.duplicate.clone(), |this, existing| this.child(self.render_duplicate(&existing, theme)))
            .when_some(self.probe_error.as_ref(), |this, e| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(theme.warning)
                        .child(icon(Icon::Alert, px(14.), theme.warning))
                        .child(format!("Couldn't check the file ({e}). You can still try downloading it.")),
                )
            })
            .into_any_element()
    }

    /// Windows 11 dialog: the form on a lighter layer, buttons on the darker base,
    /// primary action first and all buttons the same width.
    fn render_fluent(&mut self, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let body = match self.stage {
            Stage::EnterUrl => self.render_enter_url_fluent(theme, cx),
            Stage::Probing => div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(self.render_enter_url_fluent(theme, cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_color(theme.muted)
                        .child(icon(Icon::Globe, px(14.), theme.accent))
                        .child(self.probing_note),
                )
                .into_any_element(),
            Stage::Info => self.render_info_fluent(theme, cx),
        };
        let buttons: Vec<AnyElement> = match self.stage {
            Stage::Info if self.duplicate.is_some() => self.duplicate_buttons(cx),
            Stage::EnterUrl | Stage::Probing => vec![
                Button::new("ok", "OK")
                    .primary()
                    .fill()
                    .disabled(self.stage == Stage::Probing)
                    .on_click(cx.listener(|this, _, window, cx| this.check_address(window, cx)))
                    .into_any_element(),
                Button::new("cancel", "Cancel").fill().on_click(|_, window, _| window.remove_window()).into_any_element(),
            ],
            Stage::Info => vec![
                Button::new("start", "Start Download")
                    .primary()
                    .fill()
                    .on_click(cx.listener(|this, _, window, cx| this.add(true, window, cx)))
                    .into_any_element(),
                Button::new("later", "Download Later")
                    .fill()
                    .on_click(cx.listener(|this, _, window, cx| this.add(false, window, cx)))
                    .into_any_element(),
                Button::new("cancel", "Cancel").fill().on_click(|_, window, _| window.remove_window()).into_any_element(),
            ],
        };
        let button_count = buttons.len() as f32;
        div()
            .size_full()
            .bg(theme.window)
            .text_color(theme.text)
            .text_size(px(13.))
            .child(fit_window_height(
                div()
                    .flex()
                    .flex_col()
                    .child(title_bar(if self.stage == Stage::Info { "Download File Info" } else { "Add Download" }, false, window, theme))
                    .child(
                div()
                    .px(px(16.))
                    .py(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .bg(theme.bg)
                    .child(body)
                    .when_some(self.error.as_ref(), |this, e| {
                        this.child(div().text_color(theme.danger).text_size(px(12.)).child(e.clone()))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .justify_end()
                    .px(px(16.))
                    .py(px(10.))
                    .bg(theme.chrome)
                    .border_t_1()
                    .border_color(theme.border)
                    .child(div().flex().gap(px(8.)).w(px(124. * button_count)).children(buttons)),
            ),
            ))
            .into_any_element()
    }
}

impl Render for AddDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        if theme.fluent {
            return self.render_fluent(&theme, window, cx);
        }
        let body = match self.stage {
            Stage::EnterUrl => self.render_enter_url(&theme, cx),
            Stage::Probing => div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(self.render_enter_url(&theme, cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_color(theme.muted)
                        .child(icon(Icon::Globe, px(14.), theme.accent))
                        .child(self.probing_note),
                )
                .into_any_element(),
            Stage::Info => self.render_info(&theme, cx),
        };

        let buttons: Vec<AnyElement> = match self.stage {
            Stage::EnterUrl | Stage::Probing => vec![
                Button::new("cancel", "Cancel")
                    .on_click(|_, window, _| window.remove_window())
                    .into_any_element(),
                Button::new("ok", "OK")
                    .primary()
                    .disabled(self.stage == Stage::Probing)
                    .on_click(cx.listener(|this, _, window, cx| this.check_address(window, cx)))
                    .into_any_element(),
            ],
            Stage::Info => vec![
                Button::new("cancel", "Cancel")
                    .on_click(|_, window, _| window.remove_window())
                    .into_any_element(),
                Button::new("later", "Download Later")
                    .icon(Icon::List)
                    .on_click(cx.listener(|this, _, window, cx| this.add(false, window, cx)))
                    .into_any_element(),
                Button::new("start", "Start Download")
                    .primary()
                    .icon(Icon::ArrowDown)
                    .on_click(cx.listener(|this, _, window, cx| this.add(true, window, cx)))
                    .into_any_element(),
            ],
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .text_color(theme.text)
            .text_size(px(13.))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .p(px(20.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(body)
                    .when_some(self.error.as_ref(), |this, e| {
                        this.child(div().text_color(theme.danger).text_size(px(12.)).child(e.clone()))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .px(px(20.))
                    .py(px(12.))
                    .bg(theme.chrome)
                    .border_t_1()
                    .border_color(theme.border)
                    .child(div().flex_1())
                    .children(buttons),
            )
            .into_any_element()
    }
}
