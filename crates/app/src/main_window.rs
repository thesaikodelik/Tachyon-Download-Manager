use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, App, AppContext, ClipboardItem, Context, Entity, FocusHandle, Focusable,
    FontWeight, Transformation, canvas, point, radians, svg,
    KeyBinding, MouseButton, MouseDownEvent, Pixels, Point, PromptLevel, SharedString, Task,
    UniformListScrollHandle, Window, actions, anchored, deferred, div, prelude::*, px, uniform_list,
};
use tachyon_engine::{DownloadId, DownloadSnapshot, DownloadStatus, Engine, GENERAL};

use crate::add_dialog::AddPrefill;
use crate::brand::Brand;
use crate::app;
use crate::assets::Icon;
use crate::file_icons::file_icon_element;
use crate::format;
use crate::theme::Theme;
use crate::widgets::{
    Button, ToolbarButton, gliding_progress_bar, icon, progress_bar, separator, title_bar, truncated, truncated_name,
};

actions!(
    main_window,
    [ResumeSelected, StopSelected, DeleteSelected, SelectAllRows, OpenSelected, RevealSelected]
);

const CONTEXT: &str = "MainWindow";
const ROW_HEIGHT: f32 = 30.;
/// How often the list asks the engine for news; bars glide across one interval.
const REFRESH_INTERVAL: Duration = Duration::from_millis(250);
/// How long the sidebar's accent bar takes to slide to a new selection.
const INDICATOR_MOTION: Duration = Duration::from_millis(260);
/// How long a finished download's row stays green, fading.
const FLASH_MOTION: Duration = Duration::from_millis(1500);

/// The sidebar entry that shows `filter`, as keyed in `sidebar_item`.
fn filter_key(filter: &Filter) -> &'static str {
    match filter {
        Filter::All => "all",
        Filter::Unfinished => "unfinished",
        Filter::Finished => "finished",
        Filter::Queue => "queue",
        Filter::Category(name) => match name.as_str() {
            "Compressed" => "cat-compressed",
            "Documents" => "cat-documents",
            "Music" => "cat-music",
            "Programs" => "cat-programs",
            "Video" => "cat-video",
            GENERAL => "cat-general",
            _ => "cat-custom",
        },
    }
}
/// How long folding or unfolding the categories takes.
const TREE_MOTION: Duration = Duration::from_millis(220);
/// The column of the accent bars: All Downloads' bar (just right of the fold
/// chevron), and under it the category tree's thin guide line and the categories'
/// bars, close to their text.
const TREE_LINE_X: f32 = 20.;
/// Where indented category rows start, just past the guide line.
const TREE_INDENT: f32 = 26.;

/// How far the current batch of downloads is, all together: everything that has run
/// since downloading last started (finished ones included, so the bolt never drops
/// back when one completes) plus what waits in a running queue. None when idle.
fn batch_progress(downloads: &[DownloadSnapshot], queue_running: bool, batch: &mut BTreeSet<DownloadId>) -> Option<f32> {
    let waiting = |d: &DownloadSnapshot| queue_running && d.in_queue && d.status != DownloadStatus::Completed;
    if !downloads.iter().any(|d| d.status.is_active() || waiting(d)) {
        batch.clear();
        return None;
    }
    batch.extend(downloads.iter().filter(|d| d.status.is_active() || waiting(d)).map(|d| d.id));
    // Sizes not yet known (or never told) can't count towards a fraction.
    let (done, total) = downloads
        .iter()
        .filter(|d| batch.contains(&d.id))
        .filter_map(|d| d.size.map(|size| (d.downloaded.min(size), size)))
        .fold((0u64, 0u64), |(done, total), (d, s)| (done + d, total + s));
    Some(if total > 0 { done as f32 / total as f32 } else { 0. })
}
const SIDEBAR_WIDTH: f32 = 200.;

pub fn bind_keys(cx: &mut App) {
    let ctx = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("backspace", DeleteSelected, ctx),
        KeyBinding::new("delete", DeleteSelected, ctx),
        KeyBinding::new("secondary-a", SelectAllRows, ctx),
        KeyBinding::new("enter", OpenSelected, ctx),
        KeyBinding::new("secondary-o", OpenSelected, ctx),
        KeyBinding::new("secondary-r", ResumeSelected, ctx),
        KeyBinding::new("secondary-.", StopSelected, ctx),
        KeyBinding::new("secondary-shift-r", RevealSelected, ctx),
    ]);
}

#[derive(Clone, PartialEq, Eq)]
enum Filter {
    All,
    Category(String),
    Unfinished,
    Finished,
    Queue,
}

impl Filter {
    fn matches(&self, d: &DownloadSnapshot) -> bool {
        match self {
            Self::All => true,
            Self::Category(c) => &d.category == c,
            Self::Unfinished => d.status != DownloadStatus::Completed,
            Self::Finished => d.status == DownloadStatus::Completed,
            Self::Queue => d.in_queue,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Column {
    Name,
    Queue,
    Size,
    Status,
    TimeLeft,
    Rate,
    LastTry,
    Description,
}

/// IDM's main list columns. `None` width means the column takes the remaining space.
const COLUMNS: [(Column, &str, Option<f32>); 8] = [
    (Column::Name, "File Name", None),
    (Column::Queue, "Q", Some(30.)),
    (Column::Size, "Size", Some(86.)),
    (Column::Status, "Status", Some(112.)),
    (Column::TimeLeft, "Time Left", Some(80.)),
    (Column::Rate, "Transfer Rate", Some(104.)),
    (Column::LastTry, "Last Try Date", Some(142.)),
    (Column::Description, "Description", Some(150.)),
];

fn compare(column: Column, a: &DownloadSnapshot, b: &DownloadSnapshot) -> Ordering {
    match column {
        Column::Name => a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()),
        Column::Queue => (a.in_queue, a.queue_order).cmp(&(b.in_queue, b.queue_order)),
        Column::Size => a.size.cmp(&b.size),
        Column::Status => a
            .progress()
            .unwrap_or(0.0)
            .partial_cmp(&b.progress().unwrap_or(0.0))
            .unwrap_or(Ordering::Equal),
        Column::TimeLeft => a.eta_secs.cmp(&b.eta_secs),
        Column::Rate => a.speed.cmp(&b.speed),
        Column::LastTry => a.last_try.cmp(&b.last_try),
        Column::Description => a.description.cmp(&b.description),
    }
}

pub(crate) struct ContextMenu {
    pub(crate) position: Point<Pixels>,
}

pub struct MainWindow {
    engine: Arc<Engine>,
    focus_handle: FocusHandle,
    downloads: Vec<DownloadSnapshot>,
    /// Indices into `downloads`, filtered and sorted.
    rows: Vec<usize>,
    filter: Filter,
    sort: (Column, bool),
    selected: BTreeSet<DownloadId>,
    anchor: Option<DownloadId>,
    scroll: UniformListScrollHandle,
    pub(crate) context_menu: Option<ContextMenu>,
    categories_open: bool,
    /// Bumped on every fold or unfold, so each one restarts its animation.
    tree_toggles: usize,
    /// The categories are folding away; they stay drawn until the animation ends.
    tree_closing: bool,
    /// Where each sidebar entry's accent bar was last drawn, by key (window coordinates).
    indicator_slots: Rc<RefCell<HashMap<&'static str, Point<Pixels>>>>,
    /// Where the accent bar was before the last selection change, to slide from.
    indicator_from: Option<Point<Pixels>>,
    indicator_moves: usize,
    /// Each download's progress at the previous refresh, so its bar glides on.
    last_progress: HashMap<DownloadId, f32>,
    refreshes: usize,
    /// Downloads that just finished, flashing green; the value restarts the flash.
    flashes: HashMap<DownloadId, usize>,
    queue_running: bool,
    total_speed: u64,
    pub(crate) brand: Entity<Brand>,
    /// Downloads counted in the bolt's progress (see `batch_progress`).
    batch: BTreeSet<DownloadId>,
    _poll: Task<()>,
}

impl MainWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| {
            loop {
                gpui::Timer::after(Duration::from_millis(250)).await;
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
            }
        });
        cx.observe_window_appearance(window, |_, _, cx| cx.notify()).detach();
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        let mut this = Self {
            engine: app::engine(cx),
            focus_handle,
            downloads: Vec::new(),
            rows: Vec::new(),
            filter: Filter::All,
            sort: (Column::LastTry, false),
            selected: BTreeSet::new(),
            anchor: None,
            scroll: UniformListScrollHandle::new(),
            context_menu: None,
            brand: cx.new(|_| Brand::default()),
            batch: BTreeSet::new(),
            categories_open: true,
            tree_toggles: 0,
            tree_closing: false,
            indicator_slots: Rc::default(),
            indicator_from: None,
            indicator_moves: 0,
            last_progress: HashMap::new(),
            refreshes: 0,
            flashes: HashMap::new(),
            queue_running: false,
            total_speed: 0,
            _poll: poll,
        };
        this.refresh(cx);
        this
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.last_progress = self
            .downloads
            .iter()
            .filter_map(|d| d.progress().map(|p| (d.id, p as f32)))
            .collect();
        let was_running: BTreeSet<DownloadId> = self
            .downloads
            .iter()
            .filter(|d| d.status != DownloadStatus::Completed)
            .map(|d| d.id)
            .collect();
        self.refreshes += 1;
        self.downloads = self.engine.snapshot();
        let finished: Vec<DownloadId> = self
            .downloads
            .iter()
            .filter(|d| d.status == DownloadStatus::Completed && was_running.contains(&d.id))
            .map(|d| d.id)
            .collect();
        for id in finished {
            self.flash(id, cx);
        }
        self.queue_running = self.engine.is_queue_running();
        self.total_speed = self.engine.total_speed();
        let ids: BTreeSet<DownloadId> = self.downloads.iter().map(|d| d.id).collect();
        self.batch.retain(|id| ids.contains(id));
        let progress = batch_progress(&self.downloads, self.queue_running, &mut self.batch);
        self.brand.update(cx, |brand, cx| brand.set_progress(progress, cx));
        self.selected.retain(|id| ids.contains(id));
        self.rebuild_rows();
        cx.notify();
    }

    fn rebuild_rows(&mut self) {
        let (column, ascending) = self.sort;
        let mut rows: Vec<usize> = (0..self.downloads.len())
            .filter(|&i| self.filter.matches(&self.downloads[i]))
            .collect();
        rows.sort_by(|&a, &b| {
            let (a, b) = (&self.downloads[a], &self.downloads[b]);
            let ord = compare(column, a, b).then(a.added_at.cmp(&b.added_at)).then(a.id.cmp(&b.id));
            if ascending { ord } else { ord.reverse() }
        });
        self.rows = rows;
    }

    fn download(&self, id: DownloadId) -> Option<&DownloadSnapshot> {
        self.downloads.iter().find(|d| d.id == id)
    }

    fn selected_downloads(&self) -> Vec<&DownloadSnapshot> {
        self.downloads.iter().filter(|d| self.selected.contains(&d.id)).collect()
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        if filter != self.filter {
            self.indicator_from = self.indicator_slots.borrow().get(filter_key(&self.filter)).copied();
            self.indicator_moves += 1;
        }
        self.filter = filter;
        self.selected.clear();
        self.rebuild_rows();
        cx.notify();
    }

    fn set_sort(&mut self, column: Column, cx: &mut Context<Self>) {
        self.sort = if self.sort.0 == column { (column, !self.sort.1) } else { (column, true) };
        self.rebuild_rows();
        cx.notify();
    }

    // Selection

    fn on_row_mouse_down(&mut self, id: DownloadId, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        self.context_menu = None;
        if event.modifiers.platform {
            if !self.selected.remove(&id) {
                self.selected.insert(id);
            }
            self.anchor = Some(id);
        } else if event.modifiers.shift
            && let Some(anchor) = self.anchor
        {
            let position = |target| self.rows.iter().position(|&r| self.downloads[r].id == target);
            if let (Some(a), Some(b)) = (position(anchor), position(id)) {
                let (lo, hi) = (a.min(b), a.max(b));
                self.selected = self.rows[lo..=hi].iter().map(|&r| self.downloads[r].id).collect();
            }
        } else {
            self.selected = BTreeSet::from([id]);
            self.anchor = Some(id);
            if event.click_count == 2 {
                self.open(id, cx);
            }
        }
        cx.notify();
    }

    fn on_row_right_click(&mut self, id: DownloadId, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        if !self.selected.contains(&id) {
            self.selected = BTreeSet::from([id]);
            self.anchor = Some(id);
        }
        self.context_menu = Some(ContextMenu { position: event.position });
        cx.notify();
    }

    pub(crate) fn select_all(&mut self, _: &SelectAllRows, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = self.rows.iter().map(|&r| self.downloads[r].id).collect();
        cx.notify();
    }

    // Commands

    fn open(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        let Some(d) = self.download(id) else { return };
        if d.status == DownloadStatus::Completed {
            cx.open_with_system(&d.path());
        } else {
            app::open_progress_window(id, cx);
        }
    }

    fn open_selected(&mut self, _: &OpenSelected, _: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<_> = self.selected.iter().copied().collect();
        for id in ids {
            self.open(id, cx);
        }
    }

    fn resume(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<_> = self
            .selected_downloads()
            .iter()
            .filter(|d| !d.status.is_active() && d.status != DownloadStatus::Completed)
            .map(|d| d.id)
            .collect();
        let show_progress = self.engine.settings().show_progress_window && ids.len() == 1;
        for &id in &ids {
            if let Err(e) = self.engine.start(id) {
                log::warn!("cannot resume download {id}: {e}");
            }
        }
        if show_progress {
            app::open_progress_window(ids[0], cx);
        }
        self.refresh(cx);
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        for id in self.selected.clone() {
            self.engine.pause(id);
        }
        self.refresh(cx);
    }

    fn redownload(&mut self, cx: &mut Context<Self>) {
        for id in self.selected.clone() {
            if let Err(e) = self.engine.redownload(id) {
                log::warn!("cannot redownload {id}: {e}");
            }
        }
        self.refresh(cx);
    }

    fn reveal(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.selected_downloads().first() {
            let path = if d.status == DownloadStatus::Completed && d.path().exists() {
                d.path()
            } else {
                d.directory.clone()
            };
            cx.reveal_path(&path);
        }
    }

    fn copy_address(&mut self, cx: &mut Context<Self>) {
        let urls: Vec<String> = self.selected_downloads().iter().map(|d| d.url.clone()).collect();
        if !urls.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(urls.join("\n")));
        }
    }

    fn set_queued(&mut self, queued: bool, cx: &mut Context<Self>) {
        let ids: Vec<_> = self.selected.iter().copied().collect();
        self.engine.set_queued(&ids, queued);
        self.refresh(cx);
    }

    fn move_in_queue(&mut self, up: bool, cx: &mut Context<Self>) {
        if let Some(&id) = self.selected.iter().next() {
            self.engine.move_in_queue(id, up);
            self.refresh(cx);
        }
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<DownloadId> = self.selected.iter().copied().collect();
        if ids.is_empty() {
            return;
        }
        let any_completed = self
            .selected_downloads()
            .iter()
            .any(|d| d.status == DownloadStatus::Completed);
        let message = if ids.len() == 1 {
            format!("Remove “{}” from the list?", self.download(ids[0]).map(|d| d.file_name.as_str()).unwrap_or(""))
        } else {
            format!("Remove {} downloads from the list?", ids.len())
        };
        let detail = "Unfinished downloads lose the data received so far.";
        let answers: &[&str] = if any_completed {
            &["Remove", "Remove and Delete Files", "Cancel"]
        } else {
            &["Remove", "Cancel"]
        };
        let answer = window.prompt(PromptLevel::Warning, &message, Some(detail), answers, cx);
        let engine = self.engine.clone();
        cx.spawn(async move |this, cx| {
            let Ok(choice) = answer.await else { return };
            let delete_files = match (any_completed, choice) {
                (_, 0) => false,
                (true, 1) => true,
                _ => return,
            };
            engine.remove(&ids, delete_files);
            this.update(cx, |this, cx| {
                this.selected.clear();
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    fn delete_completed(&mut self, cx: &mut Context<Self>) {
        self.engine.remove_completed();
        self.refresh(cx);
    }

    // Rendering

    fn render_toolbar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let selection = self.selected_downloads();
        let can_resume = selection
            .iter()
            .any(|d| !d.status.is_active() && d.status != DownloadStatus::Completed);
        let can_stop = selection.iter().any(|d| d.status.is_active());
        let any_active = self.downloads.iter().any(|d| d.status.is_active());
        let any_completed = self.downloads.iter().any(|d| d.status == DownloadStatus::Completed);
        let divider = || div().w(px(1.)).h(px(36.)).mx(px(4.)).bg(theme.border);

        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .px(px(10.))
            .py(px(6.))
            .bg(theme.chrome)
            .when(!theme.fluent, |this| this.border_b_1().border_color(theme.border))
            .child(
                ToolbarButton::new("tb-add", "Add URL", Icon::Add)
                    .tint(if theme.fluent { theme.icon } else { theme.accent })
                    .tooltip(format!("Add a new download ({})", format::shortcut("N")))
                    .on_click(|_, _, cx| app::open_add_dialog(AddPrefill::FromClipboard, cx)),
            )
            .child(divider())
            .child(
                ToolbarButton::new("tb-resume", "Resume", Icon::Play)
                    .tint(if theme.fluent { theme.icon } else { theme.success })
                    .disabled(!can_resume)
                    .on_click(cx.listener(|this, _, _, cx| this.resume(cx))),
            )
            .child(
                ToolbarButton::new("tb-stop", "Stop", Icon::Pause)
                    .disabled(!can_stop)
                    .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
            )
            .child(
                ToolbarButton::new("tb-stop-all", "Stop All", Icon::StopAll)
                    .disabled(!any_active && !self.queue_running)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.engine.pause_all();
                        this.refresh(cx);
                    })),
            )
            .child(divider())
            .child(
                ToolbarButton::new("tb-delete", "Delete", Icon::Trash)
                    .disabled(self.selected.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| this.delete(window, cx))),
            )
            .child(
                ToolbarButton::new("tb-delete-completed", "Delete Done", Icon::TrashCheck)
                    .tooltip("Remove all completed downloads from the list")
                    .disabled(!any_completed)
                    .on_click(cx.listener(|this, _, _, cx| this.delete_completed(cx))),
            )
            .child(divider())
            .child(
                ToolbarButton::new("tb-options", "Options", Icon::Sliders)
                    .on_click(|_, _, cx| app::open_options(cx)),
            )
            .child(
                ToolbarButton::new("tb-scheduler", "Scheduler", Icon::Calendar)
                    .on_click(|_, _, cx| app::open_scheduler(cx)),
            )
            .child(divider())
            .child(
                ToolbarButton::new("tb-start-queue", "Start Queue", Icon::QueuePlay)
                    .disabled(self.queue_running)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.engine.start_queue();
                        this.refresh(cx);
                    })),
            )
            .child(
                ToolbarButton::new("tb-stop-queue", "Stop Queue", Icon::QueueStop)
                    .disabled(!self.queue_running)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.engine.stop_queue();
                        this.refresh(cx);
                    })),
            )
            .child(div().flex_1())
            .child(self.brand.clone())
    }

    #[allow(clippy::too_many_arguments)]
    fn sidebar_item(
        &self,
        key: &'static str,
        label: SharedString,
        item_icon: Icon,
        filter: Filter,
        count: usize,
        indent: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = self.filter == filter;
        let theme = *theme;
        // Windows 11 styles show categories as a tree under All Downloads: indented
        // past the guide line, a little shorter, their selection lighting the line.
        let child = indent && theme.fluent;
        div()
            .id(SharedString::from(format!("side-{key}")))
            .flex()
            .items_center()
            .gap(px(8.))
            .relative()
            .h(px(if child { 28. } else if theme.fluent { 32. } else { 28. }))
            .pl(px(if child { 10. } else if indent { 28. } else { 10. }))
            .pr(px(10.))
            .when(child, |this| this.ml(px(TREE_INDENT)).mr(px(8.)))
            .when(!child, |this| this.mx(px(8.)))
            .rounded(px(if theme.liquid { 16. } else if theme.fluent { theme.radius } else { 6. }))
            .text_size(px(13.))
            .cursor_pointer()
            .text_color(theme.text)
            .when(active && !theme.fluent, |this| this.bg(theme.accent_soft))
            // Windows 11: a quiet highlight and a short accent bar, as in Settings.
            // Records where this entry's accent bar sits, for the bar to slide from.
            .when(theme.fluent, |this| {
                let (left, top) = if child { (TREE_LINE_X - TREE_INDENT, 6.) } else { (0., 10.) };
                let slots = self.indicator_slots.clone();
                this.child(
                    canvas(
                        move |bounds, _, _| {
                            slots.borrow_mut().insert(key, bounds.origin + point(px(left), px(top)));
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
            })
            .when(active && theme.fluent, |this| {
                let (left, top) = if child { (TREE_LINE_X - TREE_INDENT, 6.) } else { (0., 10.) };
                let bar = div().absolute().left(px(left)).top(px(top)).w(px(3.)).h(px(if child { 16. } else { 12. })).rounded_full().bg(theme.accent);
                // Windows 11 Settings: the bar slides from the previous selection.
                let here = self.indicator_slots.borrow().get(key).copied();
                let slide = match (self.indicator_from, here) {
                    (Some(from), Some(here)) if crate::platform::animations_enabled() => Some(from - here),
                    _ => None,
                };
                this.bg(theme.hover).child(match slide {
                    Some(offset) => bar
                        .with_animation(
                            ("indicator", self.indicator_moves),
                            Animation::new(INDICATOR_MOTION).with_easing(gpui::ease_out_quint()),
                            move |bar, t| {
                                let rest = 1. - t;
                                bar.left(px(left) + offset.x * rest).top(px(top) + offset.y * rest)
                            },
                        )
                        .into_any_element(),
                    None => bar.into_any_element(),
                })
            })
            .when(!active, |this| this.hover(|s| s.bg(theme.hover)))
            .on_click(cx.listener(move |this, _, _, cx| this.set_filter(filter.clone(), cx)))
            // Windows 11 styles: icons in the accent colour, set apart from the text.
            .child(icon(item_icon, px(15.), if theme.fluent {
                theme.icon
            } else if active {
                theme.accent
            } else {
                theme.muted
            }))
            .child(div().flex_1().min_w_0().child(truncated(label)))
            .when(count > 0, |this| {
                this.child(div().text_size(px(11.)).text_color(theme.muted).child(count.to_string()))
            })
    }

    /// Briefly tints a download's row green as it finishes.
    fn flash(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        if !crate::platform::animations_enabled() {
            return;
        }
        let n = self.refreshes;
        self.flashes.insert(id, n);
        cx.spawn(async move |this, cx| {
            gpui::Timer::after(FLASH_MOTION).await;
            this.update(cx, |this, cx| {
                if this.flashes.get(&id) == Some(&n) {
                    this.flashes.remove(&id);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Folds or unfolds the categories, animated.
    pub(crate) fn toggle_categories(&mut self, cx: &mut Context<Self>) {
        self.tree_toggles += 1;
        if self.categories_open {
            self.categories_open = false;
            self.tree_closing = crate::platform::animations_enabled();
            let toggles = self.tree_toggles;
            cx.spawn(async move |this, cx| {
                gpui::Timer::after(TREE_MOTION).await;
                this.update(cx, |this, cx| {
                    // Unless it was unfolded again meanwhile.
                    if this.tree_toggles == toggles {
                        this.tree_closing = false;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        } else {
            self.categories_open = true;
            self.tree_closing = false;
        }
        cx.notify();
    }

    /// The categories' container, sliding open or shut (and fading) after a toggle.
    fn animate_tree(&self, tree: AnyElement) -> AnyElement {
        if self.tree_toggles == 0 || !crate::platform::animations_enabled() {
            return tree;
        }
        let opening = self.categories_open;
        let rows = self.engine.settings().categories.len() + 1;
        // Category rows are 28px apart by 2px gaps.
        let full = rows as f32 * 28. + (rows.saturating_sub(1)) as f32 * 2.;
        div()
            .overflow_hidden()
            .child(tree)
            .with_animation(
                ("tree-motion", self.tree_toggles),
                Animation::new(TREE_MOTION).with_easing(gpui::ease_out_quint()),
                move |el, t| {
                    let shown = if opening { t } else { 1. - t };
                    el.h(px(full * shown)).opacity(shown)
                },
            )
            .into_any_element()
    }

    /// The fold chevron, turning a quarter between pointing right and down.
    fn render_chevron(&self, theme: &Theme) -> AnyElement {
        let glyph = svg().path(Icon::ChevronRight.path()).size(px(10.)).text_color(theme.muted);
        let open = self.categories_open;
        if self.tree_toggles == 0 || !crate::platform::animations_enabled() {
            let angle = if open { std::f32::consts::FRAC_PI_2 } else { 0. };
            return glyph.with_transformation(Transformation::rotate(radians(angle))).into_any_element();
        }
        glyph
            .with_animation(
                ("tree-chevron", self.tree_toggles),
                Animation::new(TREE_MOTION).with_easing(gpui::ease_out_quint()),
                move |glyph, t| {
                    let turn = if open { t } else { 1. - t };
                    glyph.with_transformation(Transformation::rotate(radians(turn * std::f32::consts::FRAC_PI_2)))
                },
            )
            .into_any_element()
    }

    fn render_sidebar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.engine.settings();
        let mut categories: Vec<String> = settings.categories.iter().map(|c| c.name.clone()).collect();
        categories.push(GENERAL.to_string());
        let count = |f: &Filter| self.downloads.iter().filter(|d| f.matches(d)).count();

        let mut items: Vec<AnyElement> = Vec::new();
        items.push(
            div()
                .flex()
                .items_center()
                .child(
                    div()
                        .id("toggle-categories")
                        .pl(px(10.))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_categories(cx)))
                        .child(self.render_chevron(theme)),
                )
                .child(
                    div().flex_1().ml(px(-8.)).child(self.sidebar_item(
                        "all",
                        "All Downloads".into(),
                        Icon::Inbox,
                        Filter::All,
                        self.downloads.len(),
                        false,
                        theme,
                        cx,
                    )),
                )
                .into_any_element(),
        );
        if self.categories_open || self.tree_closing {
            let mut children: Vec<AnyElement> = Vec::new();
            for category in categories {
                let filter = Filter::Category(category.clone());
                let n = count(&filter);
                let key: &'static str = match category.as_str() {
                    "Compressed" => "cat-compressed",
                    "Documents" => "cat-documents",
                    "Music" => "cat-music",
                    "Programs" => "cat-programs",
                    "Video" => "cat-video",
                    GENERAL => "cat-general",
                    _ => "cat-custom",
                };
                children.push(
                    self.sidebar_item(key, category.clone().into(), Icon::for_category(&category), filter, n, true, theme, cx)
                        .into_any_element(),
                );
            }
            items.push(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    // The tree's guide line: a hairline down the middle of the 3px accent
                    // bars, from just under All Downloads' bar (which ends 10px above its
                    // 32px row's end, plus the 2px gap) to the bottom of the last category.
                    .when(theme.fluent, |this| {
                        this.child(
                            div()
                                .absolute()
                                .left(px(TREE_LINE_X + 1.))
                                .top(px(-9.))
                                .bottom_0()
                                .w(px(1.))
                                .rounded_full()
                                .bg(theme.accent.opacity(0.3)),
                        )
                    })
                    .children(children)
                    .into_any_element(),
            );
            let tree = items.pop().expect("just pushed");
            items.push(self.animate_tree(tree));
        }
        items.push(self.sidebar_item("unfinished", "Unfinished".into(), Icon::Clock, Filter::Unfinished, count(&Filter::Unfinished), false, theme, cx).into_any_element());
        items.push(self.sidebar_item("finished", "Finished".into(), Icon::CheckCircle, Filter::Finished, count(&Filter::Finished), false, theme, cx).into_any_element());

        div()
            .id("sidebar")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .bg(theme.surface)
            .when(!theme.fluent, |this| this.border_r_1().border_color(theme.border))
            .pt(px(10.))
            .gap(px(2.))
            .overflow_y_scroll()
            .child(div().px(px(18.)).pb(px(4.)).child(crate::widgets::section_title("Categories", theme)))
            .children(items)
            .child(div().h(px(12.)))
            .child(div().px(px(18.)).pb(px(4.)).child(crate::widgets::section_title("Queues", theme)))
            .child(
                self.sidebar_item(
                    "queue",
                    if self.queue_running { "Main Queue · running".into() } else { "Main Queue".into() },
                    Icon::List,
                    Filter::Queue,
                    count(&Filter::Queue),
                    false,
                    theme,
                    cx,
                ),
            )
    }

    fn render_header(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let (sorted, ascending) = self.sort;
        div()
            .flex()
            .flex_none()
            .h(px(28.))
            .px(px(8.))
            .items_center()
            .when(!theme.fluent, |this| this.bg(theme.chrome))
            .border_b_1()
            .border_color(theme.border)
            .text_size(px(12.))
            .text_color(theme.muted)
            .children(COLUMNS.iter().map(|&(column, title, width)| {
                let is_sorted = sorted == column;
                div()
                    .id(SharedString::from(format!("col-{title}")))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .h_full()
                    .px(px(6.))
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme.text))
                    .when_some(width, |this, w| this.w(px(w)).flex_none())
                    .when(width.is_none(), |this| this.flex_1().min_w(px(180.)))
                    .when(is_sorted, |this| this.text_color(theme.text).font_weight(FontWeight::MEDIUM))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_sort(column, cx)))
                    .child(truncated(title))
                    .when(is_sorted, |this| {
                        this.child(icon(
                            if ascending { Icon::ChevronUp } else { Icon::ChevronDown },
                            px(10.),
                            theme.muted,
                        ))
                    })
            }))
    }

    fn render_cell(&self, column: Column, d: &DownloadSnapshot, theme: &Theme) -> AnyElement {
        let muted = theme.muted;
        match column {
            Column::Name if theme.fluent => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .min_w_0()
                .child(if d.status == DownloadStatus::Failed {
                    icon(Icon::Alert, px(16.), theme.danger).into_any_element()
                } else {
                    file_icon_element(&d.path(), &d.category, px(16.), theme)
                })
                .child(truncated_name(d.file_name.clone()))
                .into_any_element(),
            Column::Name => {
                let (row_icon, color) = match d.status {
                    DownloadStatus::Failed => (Icon::Alert, theme.danger),
                    DownloadStatus::Completed => (Icon::for_category(&d.category), theme.success),
                    s if s.is_active() => (Icon::ArrowDown, theme.accent),
                    _ => (Icon::for_category(&d.category), muted),
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .min_w_0()
                    .child(icon(row_icon, px(15.), color))
                    .child(truncated_name(d.file_name.clone()))
                    .into_any_element()
            }
            Column::Queue => div()
                .when(d.in_queue, |this| this.child(icon(Icon::List, px(13.), muted)))
                .into_any_element(),
            Column::Size => truncated(format::size_cell(d)).into_any_element(),
            Column::Status => {
                let text = format::status_cell(d);
                let color = match d.status {
                    DownloadStatus::Completed => theme.success,
                    DownloadStatus::Failed => theme.danger,
                    _ => theme.text,
                };
                let show_bar = matches!(d.status, DownloadStatus::Downloading | DownloadStatus::Stopped)
                    && d.downloaded > 0
                    && d.progress().is_some();
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(div().w_full().text_color(color).child(truncated(text)))
                    .when(show_bar, |this| {
                        let bar_color = if d.status == DownloadStatus::Downloading { theme.accent } else { theme.faint };
                        let to = d.progress().unwrap_or(0.) as f32;
                        let from = self.last_progress.get(&d.id).copied().unwrap_or(to);
                        if crate::platform::animations_enabled() && (to - from).abs() > 0.0005 {
                            // Glides over the time until the next refresh.
                            this.child(gliding_progress_bar(
                                ("row-bar", (d.id as usize) << 20 | (self.refreshes & 0xfffff)),
                                from,
                                to,
                                REFRESH_INTERVAL,
                                px(3.),
                                bar_color,
                                theme,
                            ))
                        } else {
                            this.child(progress_bar(d.progress(), px(3.), bar_color, theme))
                        }
                    })
                    .into_any_element()
            }
            Column::TimeLeft => truncated(format::eta_cell(d)).into_any_element(),
            Column::Rate => {
                truncated(if d.status == DownloadStatus::Downloading { format::speed(d.speed) } else { String::new() })
                    .into_any_element()
            }
            Column::LastTry => div()
                .w_full()
                .text_color(muted)
                .child(truncated(format::date(d.last_try.or(d.completed_at).or(Some(d.added_at)))))
                .into_any_element(),
            Column::Description => div()
                .text_color(muted)
                .child(truncated(d.description.clone()))
                .into_any_element(),
        }
    }

    fn render_rows(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = Theme::of(window);
        range
            .filter_map(|ix| {
                let &row = self.rows.get(ix)?;
                let d = &self.downloads[row];
                let id = d.id;
                let selected = self.selected.contains(&id);
                let bg = if selected {
                    theme.accent_soft
                } else if ix % 2 == 1 {
                    theme.row_alt
                } else {
                    theme.bg
                };
                let flash = self.flashes.get(&id).copied();
                let row = div()
                        .id(("row", id as usize))
                        .flex()
                        .items_center()
                        .h(px(ROW_HEIGHT))
                        .w_full()
                        .px(px(8.))
                        .bg(bg)
                        .text_size(px(13.))
                        .text_color(theme.text)
                        .when(!selected, |this| this.hover(|s| s.bg(theme.hover)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                this.on_row_mouse_down(id, event, window, cx)
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                this.on_row_right_click(id, event, window, cx)
                            }),
                        )
                        .children(COLUMNS.iter().map(|&(column, _, width)| {
                            div()
                                .px(px(6.))
                                .overflow_hidden()
                                .when_some(width, |this, w| this.w(px(w)).flex_none())
                                .when(width.is_none(), |this| this.flex_1().min_w(px(180.)))
                                .child(self.render_cell(column, d, &theme))
                        }))
;
                Some(match flash {
                    // Just finished: a green wash that fades back to the row's colour.
                    Some(n) => {
                        let green = theme.success;
                        row.with_animation(
                            ("row-flash", (id as usize) << 20 | (n & 0xfffff)),
                            Animation::new(FLASH_MOTION).with_easing(gpui::ease_out_quint()),
                            move |row, t| row.bg(green.opacity(0.28 * (1. - t))),
                        )
                        .into_any_element()
                    }
                    None => row.into_any_element(),
                })
            })
            .collect()
    }

    fn render_empty(&self, theme: &Theme) -> impl IntoElement {
        let (title, hint) = match self.filter {
            Filter::All => (
                "No downloads yet",
                if cfg!(target_os = "macos") {
                    "Copy a link and press ⌘N, or click “Add URL”."
                } else {
                    "Copy a link and press Ctrl+N, or click “Add URL”."
                },
            ),
            Filter::Queue => ("The queue is empty", "Choose “Download Later” or right-click a download to queue it."),
            _ => ("Nothing here", "Downloads in this view will appear here."),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.))
            .child(icon(Icon::Inbox, px(40.), theme.faint))
            .child(div().text_size(px(15.)).font_weight(FontWeight::MEDIUM).text_color(theme.text).child(title))
            .child(div().text_size(px(13.)).text_color(theme.muted).child(hint))
            .when(self.filter == Filter::All, |this| {
                this.child(
                    div().pt(px(6.)).child(
                        Button::new("empty-add", "Add URL")
                            .primary()
                            .icon(Icon::Add)
                            .on_click(|_, _, cx| app::open_add_dialog(AddPrefill::FromClipboard, cx)),
                    ),
                )
            })
    }

    fn render_status_bar(&self, theme: &Theme) -> impl IntoElement {
        let active = self.downloads.iter().filter(|d| d.status.is_active()).count();
        let connections: usize = self.downloads.iter().map(|d| d.active_connections).sum();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(16.))
            .h(px(if theme.liquid { 20. } else { 26. }))
            .px(px(12.))
            .bg(theme.chrome)
            .when(!theme.liquid, |this| this.border_t_1().border_color(theme.border))
            .text_size(px(12.))
            .text_color(theme.muted)
            .child(format!(
                "{} download{} · {} active",
                self.downloads.len(),
                if self.downloads.len() == 1 { "" } else { "s" },
                active
            ))
            .when(connections > 0, |this| this.child(format!("{connections} connections")))
            .when(self.queue_running, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_color(theme.accent)
                        .child(icon(Icon::QueuePlay, px(12.), theme.accent))
                        .child("Queue running"),
                )
            })
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .when(self.total_speed > 0, |this| this.text_color(theme.text))
                    .child(icon(Icon::ArrowDown, px(12.), if self.total_speed > 0 { theme.accent } else { theme.faint }))
                    .child(if self.total_speed > 0 { format::speed(self.total_speed) } else { "Idle".into() }),
            )
    }

    fn render_context_menu(&self, menu: &ContextMenu, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let selection = self.selected_downloads();
        let single = selection.len() == 1;
        let completed = selection.iter().all(|d| d.status == DownloadStatus::Completed);
        let can_resume = selection.iter().any(|d| !d.status.is_active() && d.status != DownloadStatus::Completed);
        let can_stop = selection.iter().any(|d| d.status.is_active());
        let all_queued = selection.iter().all(|d| d.in_queue);
        let any_running = selection.iter().any(|d| d.status.is_active());
        let theme = *theme;

        type Handler = Box<dyn Fn(&mut MainWindow, &mut Window, &mut Context<MainWindow>)>;
        let mut entries: Vec<Option<(&'static str, Icon, bool, Handler)>> = Vec::new();
        entries.push(Some(("Open", Icon::External, single && completed, Box::new(|this, _, cx| {
            let ids: Vec<_> = this.selected.iter().copied().collect();
            ids.into_iter().for_each(|id| this.open(id, cx));
        }))));
        entries.push(Some((format::REVEAL_LABEL, Icon::Folder, single, Box::new(|this, _, cx| this.reveal(cx)))));
        entries.push(Some(("Progress Window", Icon::Info, single && !completed, Box::new(|this, _, cx| {
            if let Some(&id) = this.selected.iter().next() {
                app::open_progress_window(id, cx);
            }
        }))));
        entries.push(None);
        entries.push(Some(("Resume", Icon::Play, can_resume, Box::new(|this, _, cx| this.resume(cx)))));
        entries.push(Some(("Stop", Icon::Pause, can_stop, Box::new(|this, _, cx| this.stop(cx)))));
        entries.push(Some(("Redownload", Icon::Refresh, !any_running, Box::new(|this, _, cx| this.redownload(cx)))));
        entries.push(None);
        if all_queued {
            entries.push(Some(("Remove from Queue", Icon::List, true, Box::new(|this, _, cx| this.set_queued(false, cx)))));
            entries.push(Some(("Move Up in Queue", Icon::ChevronUp, single, Box::new(|this, _, cx| this.move_in_queue(true, cx)))));
            entries.push(Some(("Move Down in Queue", Icon::ChevronDown, single, Box::new(|this, _, cx| this.move_in_queue(false, cx)))));
        } else {
            entries.push(Some(("Add to Queue", Icon::List, !completed, Box::new(|this, _, cx| this.set_queued(true, cx)))));
        }
        entries.push(None);
        entries.push(Some(("Copy Address", Icon::Copy, true, Box::new(|this, _, cx| this.copy_address(cx)))));
        entries.push(Some(("Delete…", Icon::Trash, true, Box::new(|this, window, cx| this.delete(window, cx)))));

        let items = entries.into_iter().enumerate().map(|(ix, entry)| match entry {
            None => div().my(px(4.)).mx(px(8.)).child(separator(&theme)).into_any_element(),
            Some((label, item_icon, enabled, handler)) => div()
                .id(("ctx", ix))
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(26.))
                .px(px(10.))
                .mx(px(4.))
                .rounded(px(5.))
                .text_size(px(13.))
                .text_color(if enabled { theme.text } else { theme.faint })
                .when(enabled, |this| {
                    this.cursor_pointer()
                        .hover(|s| s.bg(theme.accent).text_color(theme.on_accent))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.context_menu = None;
                            handler(this, window, cx);
                            cx.notify();
                        }))
                })
                .child(icon(item_icon, px(14.), if enabled { theme.muted } else { theme.faint }))
                .child(label)
                .into_any_element(),
        });

        deferred(
            anchored().position(menu.position).snap_to_window_with_margin(px(8.)).child(
                div()
                    .id("context-menu")
                    .occlude()
                    .min_w(px(210.))
                    .py(px(4.))
                    .rounded(px(8.))
                    .bg(theme.chrome)
                    .border_1()
                    .border_color(theme.border)
                    .shadow_lg()
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.context_menu = None;
                        cx.notify();
                    }))
                    .children(items),
            ),
        )
        .with_priority(1)
    }
}

#[cfg(feature = "dev-snapshots")]
impl MainWindow {
    pub(crate) fn dev_select_first(&mut self, cx: &mut Context<Self>) {
        if let Some(&row) = self.rows.first() {
            self.selected = BTreeSet::from([self.downloads[row].id]);
            cx.notify();
        }
    }

    pub(crate) fn dev_filter(&mut self, name: &str, cx: &mut Context<Self>) {
        let filter = match name {
            "unfinished" => Filter::Unfinished,
            "finished" => Filter::Finished,
            "queue" => Filter::Queue,
            "all" => Filter::All,
            category => Filter::Category(category.to_string()),
        };
        self.set_filter(filter, cx);
    }
}

impl Focusable for MainWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        let list: AnyElement = if self.rows.is_empty() {
            self.render_empty(&theme).into_any_element()
        } else {
            uniform_list(
                "downloads",
                self.rows.len(),
                cx.processor(|this, range, window, cx| this.render_rows(range, window, cx)),
            )
            .size_full()
            .track_scroll(self.scroll.clone())
            .into_any_element()
        };

        // The title bar stays outside the focusable area: taking focus on a click marks
        // the click as handled, and Windows would then never get to move the window.
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.window)
            .text_color(theme.text)
            .font_family(".SystemUIFont")
            .child(title_bar("Tachyon Download Manager", true, window, &theme))
            .child(
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(|this, _: &ResumeSelected, _, cx| this.resume(cx)))
            .on_action(cx.listener(|this, _: &StopSelected, _, cx| this.stop(cx)))
            .on_action(cx.listener(|this, _: &DeleteSelected, window, cx| this.delete(window, cx)))
            .on_action(cx.listener(|this, _: &RevealSelected, _, cx| this.reveal(cx)))
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .child(self.render_toolbar(&theme, cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(&theme, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .when(theme.fluent, |this| {
                                this.bg(theme.layer)
                                    .rounded_tl(px(theme.radius_large))
                                    .border_l_1()
                                    .border_t_1()
                                    .border_color(theme.border)
                                    .overflow_hidden()
                            })
                            .children(crate::updater::banner(&theme, cx))
                            .child(self.render_header(&theme, cx))
                            .child(
                                div()
                                    .id("list-area")
                                    .flex_1()
                                    .min_h_0()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, window, _| {
                                            window.focus(&this.focus_handle);
                                        }),
                                    )
                                    .child(list),
                            )
                            // Windows 11 styles: the list's panel runs to the bottom corner,
                            // with the status line inside it.
                            .when(theme.fluent, |this| this.child(self.render_status_bar(&theme))),
                    ),
            )
            .when(!theme.fluent, |this| this.child(self.render_status_bar(&theme)))
            .when_some(self.context_menu.as_ref(), |this, menu| {
                this.child(self.render_context_menu(menu, &theme, cx))
            }),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use tachyon_engine::{DownloadSnapshot, DownloadStatus};

    use super::batch_progress;

    fn download(id: u64, status: DownloadStatus, downloaded: u64, size: u64) -> DownloadSnapshot {
        DownloadSnapshot { id, status, downloaded, size: Some(size), ..Default::default() }
    }

    #[test]
    fn finished_downloads_keep_counting_until_the_batch_ends() {
        let mut batch = BTreeSet::new();
        let running = [download(1, DownloadStatus::Downloading, 50, 100), download(2, DownloadStatus::Downloading, 0, 100)];
        assert_eq!(batch_progress(&running, false, &mut batch), Some(0.25));
        // The first finishes: the bolt must not drop back.
        let one_done = [download(1, DownloadStatus::Completed, 100, 100), download(2, DownloadStatus::Downloading, 50, 100)];
        assert_eq!(batch_progress(&one_done, false, &mut batch), Some(0.75));
        // Everything done: idle, and the next batch starts afresh.
        let all_done = [download(1, DownloadStatus::Completed, 100, 100), download(2, DownloadStatus::Completed, 100, 100)];
        assert_eq!(batch_progress(&all_done, false, &mut batch), None);
        assert!(batch.is_empty());
    }

    #[test]
    fn a_running_queue_counts_what_still_waits() {
        let mut batch = BTreeSet::new();
        let mut waiting = download(2, DownloadStatus::Stopped, 0, 300);
        waiting.in_queue = true;
        let items = [download(1, DownloadStatus::Downloading, 100, 100), waiting];
        assert_eq!(batch_progress(&items, true, &mut batch), Some(0.25));
    }
}
