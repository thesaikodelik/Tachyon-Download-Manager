//! IDM's "Download all links": the links the browser extension found on a page,
//! filtered by type, to tick and download at once or put in the queue.

use std::collections::BTreeMap;
use std::sync::Arc;

use gpui::{
    AnyElement, AppContext, Context, Entity, FontWeight, PathPromptOptions, SharedString, Subscription, Window, div,
    prelude::*, px,
};
use tachyon_engine::{AddRequest, Engine, GENERAL, LinkBatch, RequestSpec, derive_file_name, extension_of};

use crate::app;
use crate::assets::Icon;
use crate::format::{self, display_path, expand_home};
use crate::theme::Theme;
use crate::widgets::text_input::{InputEvent, TextInput};
use crate::widgets::{Button, Checkbox, icon, separator, title_bar, truncated, truncated_name};

/// Sizes are asked for this many links at a time.
const PROBES_AT_ONCE: usize = 4;

struct LinkItem {
    url: String,
    name: String,
    category: String,
    /// A file (by its type), not a page.
    file: bool,
    checked: bool,
    size: Option<u64>,
}

/// Which links the list shows.
#[derive(Clone, PartialEq, Eq)]
enum Filter {
    Files,
    Category(String),
    All,
}

pub struct LinksWindow {
    engine: Arc<Engine>,
    page: String,
    title: String,
    items: Vec<LinkItem>,
    filter: Filter,
    search: Entity<TextInput>,
    /// Empty: each file goes to its category's folder.
    folder: Entity<TextInput>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl LinksWindow {
    pub fn new(batch: LinkBatch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let engine = app::engine(cx);
        let settings = engine.settings();
        let wanted: Vec<String> = settings
            .capture_extensions
            .iter()
            .cloned()
            .chain(settings.categories.iter().flat_map(|c| c.extensions.clone()))
            .collect();
        let items: Vec<LinkItem> = batch
            .links
            .iter()
            .map(|link| {
                let name = derive_file_name(None, &link.url, None);
                let file = extension_of(&name).is_some_and(|ext| wanted.iter().any(|w| w.eq_ignore_ascii_case(&ext)));
                let (category, _) = engine.default_target(&name);
                LinkItem { url: link.url.clone(), name, category, file, checked: file, size: None }
            })
            .collect();
        let filter = if items.iter().any(|i| i.file) { Filter::Files } else { Filter::All };
        let search = cx.new(|cx| TextInput::new(cx, "Search"));
        let folder = cx.new(|cx| TextInput::new(cx, "Each type's own folder"));
        let subscriptions = vec![cx.subscribe_in(&search, window, |_, _, event, _, cx| {
            if let InputEvent::Changed = event {
                cx.notify();
            }
        })];
        let mut this = Self {
            engine,
            page: batch.page,
            title: batch.title,
            items,
            filter,
            search,
            folder,
            error: None,
            _subscriptions: subscriptions,
        };
        this.fetch_sizes(window, cx);
        this
    }

    /// Asks the servers for the files' sizes (and real names) in the background, a
    /// few at a time; the list fills in as they answer.
    fn fetch_sizes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let jobs: Vec<(usize, String)> =
            self.items.iter().enumerate().filter(|(_, i)| i.file).map(|(ix, i)| (ix, i.url.clone())).collect();
        let referer = Some(self.page.clone()).filter(|p| !p.is_empty());
        for chunk in jobs.chunks(jobs.len().div_ceil(PROBES_AT_ONCE).max(1)) {
            let chunk = chunk.to_vec();
            let engine = self.engine.clone();
            let referer = referer.clone();
            cx.spawn_in(window, async move |this, cx| {
                for (ix, url) in chunk {
                    let spec = RequestSpec { url, referer: referer.clone(), ..Default::default() };
                    let Ok(info) = engine.probe(spec).await else { continue };
                    let alive = this
                        .update(cx, |this, cx| {
                            if let Some(item) = this.items.get_mut(ix) {
                                item.size = info.total;
                                if !info.file_name.is_empty() {
                                    item.name = info.file_name.clone();
                                }
                            }
                            cx.notify();
                        })
                        .is_ok();
                    if !alive {
                        return;
                    }
                }
            })
            .detach();
        }
    }

    fn visible(&self, cx: &Context<Self>) -> Vec<usize> {
        let query = self.search.read(cx).text().trim().to_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter(|(_, i)| match &self.filter {
                Filter::Files => i.file,
                Filter::Category(c) => i.file && &i.category == c,
                Filter::All => true,
            })
            .filter(|(_, i)| query.is_empty() || i.name.to_lowercase().contains(&query) || i.url.to_lowercase().contains(&query))
            .map(|(ix, _)| ix)
            .collect()
    }

    fn set_visible_checked(&mut self, checked: bool, cx: &mut Context<Self>) {
        for ix in self.visible(cx) {
            self.items[ix].checked = checked;
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
                })
                .ok();
            }
        })
        .detach();
    }

    /// Adds the ticked links to the queue, and starts it unless only queuing.
    fn add(&mut self, start: bool, window: &mut Window, cx: &mut Context<Self>) {
        let folder = self.folder.read(cx).text();
        let directory = Some(folder.trim()).filter(|f| !f.is_empty()).map(expand_home);
        let referer = Some(self.page.clone()).filter(|p| !p.is_empty());
        let mut added = 0;
        for item in self.items.iter().filter(|i| i.checked) {
            let request = RequestSpec { url: item.url.clone(), referer: referer.clone(), ..Default::default() };
            if self.engine.find_same(&request).is_some() {
                continue;
            }
            let result = self.engine.add(AddRequest {
                request,
                directory: directory.clone(),
                category: Some(item.category.clone()),
                queue: true,
                ..Default::default()
            });
            match result {
                Ok(_) => added += 1,
                Err(e) => log::warn!("could not add {}: {e}", item.url),
            }
        }
        if added == 0 && !self.items.iter().any(|i| i.checked) {
            self.error = Some("Tick the links to download.".into());
            cx.notify();
            return;
        }
        if start {
            self.engine.start_queue();
        }
        window.remove_window();
    }

    #[cfg(feature = "dev-snapshots")]
    pub(crate) fn dev_command(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        match name {
            "links-start" => self.add(true, window, cx),
            "links-queue" => self.add(false, window, cx),
            _ => {
                self.filter = Filter::All;
                cx.notify();
            }
        }
    }

    fn render_filters(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for item in self.items.iter().filter(|i| i.file) {
            *counts.entry(item.category.clone()).or_default() += 1;
        }
        let files = self.items.iter().filter(|i| i.file).count();
        let mut chips: Vec<(Filter, String)> = Vec::new();
        if files > 0 {
            chips.push((Filter::Files, format!("Files {files}")));
            for (category, n) in counts.into_iter().filter(|(c, _)| c != GENERAL) {
                chips.push((Filter::Category(category.clone()), format!("{category} {n}")));
            }
        }
        chips.push((Filter::All, format!("All links {}", self.items.len())));
        let theme = *theme;
        div().flex().flex_wrap().gap(px(6.)).children(chips.into_iter().enumerate().map(|(ix, (filter, label))| {
            let active = filter == self.filter;
            div()
                .id(("filter", ix))
                .px(px(10.))
                .py(px(3.))
                .rounded_full()
                .cursor_pointer()
                .text_size(px(12.))
                .border_1()
                .map(|this| {
                    if active {
                        this.bg(theme.accent_soft).border_color(theme.accent).text_color(theme.accent)
                    } else {
                        this.bg(theme.button).border_color(theme.border).hover(move |s| s.bg(theme.button_hover))
                    }
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.filter = filter.clone();
                    cx.notify();
                }))
                .child(label)
        }))
    }

    fn render_row(&self, ix: usize, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let item = &self.items[ix];
        let theme = *theme;
        let place = url::Url::parse(&item.url)
            .map(|u| format!("{}{}", u.host_str().unwrap_or(""), u.path()))
            .unwrap_or_else(|_| item.url.clone());
        div()
            .id(("link", ix))
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(30.))
            .px(px(10.))
            .rounded(px(theme.radius))
            .hover(move |s| s.bg(theme.hover))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(item) = this.items.get_mut(ix) {
                    item.checked = !item.checked;
                }
                cx.notify();
            }))
            .child(Checkbox::new(("link-check", ix), "", item.checked).on_toggle(cx.listener(move |this, v: &bool, _, cx| {
                if let Some(item) = this.items.get_mut(ix) {
                    item.checked = *v;
                }
                cx.notify();
            })))
            .child(icon(if item.file { Icon::for_category(&item.category) } else { Icon::Globe }, px(16.), theme.icon))
            .child(div().flex_1().min_w_0().child(truncated_name(item.name.clone())))
            .child(
                div()
                    .w(px(80.))
                    .flex_none()
                    .text_color(theme.muted)
                    .child(truncated(item.size.map(format::bytes).unwrap_or_else(|| if item.file { "…".into() } else { String::new() }))),
            )
            .child(div().w(px(230.)).flex_none().text_size(px(12.)).text_color(theme.faint).child(truncated(place)))
            .into_any_element()
    }
}

impl Render for LinksWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        let visible = self.visible(cx);
        let checked = self.items.iter().filter(|i| i.checked).count();
        let checked_size: u64 = self.items.iter().filter(|i| i.checked).filter_map(|i| i.size).sum();
        let heading = if self.title.is_empty() {
            url::Url::parse(&self.page).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default()
        } else {
            self.title.clone()
        };
        let rows: Vec<AnyElement> = visible.iter().map(|&ix| self.render_row(ix, &theme, cx)).collect();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.window)
            .text_color(theme.text)
            .text_size(px(13.))
            .child(title_bar("Download Links", false, window, &theme))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .px(px(16.))
                    .pt(px(6.))
                    .pb(px(10.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(icon(Icon::Globe, px(16.), theme.icon))
                            .child(div().flex_1().min_w_0().font_weight(FontWeight::SEMIBOLD).child(truncated(heading)))
                            .child(div().text_color(theme.muted).child(format!("{} links", self.items.len())))
                            .child(div().w(px(200.)).child(self.search.clone())),
                    )
                    .child(self.render_filters(&theme, cx)),
            )
            .child(
                div()
                    .id("links")
                    .flex_1()
                    .min_h_0()
                    .mx(px(12.))
                    .p(px(4.))
                    .overflow_y_scroll()
                    .rounded(px(theme.radius))
                    .bg(theme.layer)
                    .border_1()
                    .border_color(theme.border)
                    .children(rows)
                    .when(visible.is_empty(), |this| {
                        this.child(div().p(px(12.)).text_color(theme.muted).child("No links of this kind on the page."))
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(16.))
                    .py(px(10.))
                    .child(div().text_color(theme.muted).child("Select:"))
                    .child(Button::new("all", "All").compact().on_click(cx.listener(|this, _, _, cx| this.set_visible_checked(true, cx))))
                    .child(Button::new("none", "None").compact().on_click(cx.listener(|this, _, _, cx| this.set_visible_checked(false, cx))))
                    .child(div().flex_1())
                    .child(div().text_color(theme.muted).child("Save to"))
                    .child(div().w(px(240.)).child(self.folder.clone()))
                    .child(Button::new("browse", "Browse…").on_click(cx.listener(|this, _, window, cx| this.browse_folder(window, cx)))),
            )
            .child(separator(&theme))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .px(px(16.))
                    .py(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .map(|this| match &self.error {
                                Some(e) => this.text_color(theme.danger).child(e.clone()),
                                None if checked_size > 0 => {
                                    this.text_color(theme.muted).child(format!("{checked} selected \u{b7} {}", format::bytes(checked_size)))
                                }
                                None => this.text_color(theme.muted).child(format!("{checked} selected")),
                            }),
                    )
                    .child(
                        Button::new("download", SharedString::from(format!("Download {checked}")))
                            .primary()
                            .disabled(checked == 0)
                            .on_click(cx.listener(|this, _, window, cx| this.add(true, window, cx))),
                    )
                    .child(
                        Button::new("queue", "Add to Queue")
                            .disabled(checked == 0)
                            .on_click(cx.listener(|this, _, window, cx| this.add(false, window, cx))),
                    )
                    .child(Button::new("cancel", "Cancel").on_click(|_, window, _| window.remove_window())),
            )
    }
}
