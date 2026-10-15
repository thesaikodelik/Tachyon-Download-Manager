//! Options, laid out like Windows 11 Settings: sections on the left, their options
//! on the right. Covers what IDM's Options has, minus dial-up networking.

use std::sync::Arc;

use gpui::{
    AnyElement, AppContext, Context, Entity, FontWeight, PathPromptOptions, SharedString, Window, div,
    prelude::*, px,
};
use tachyon_engine::{
    BrowserRule, CategoryDef, DEFAULT_CAPTURE_EXTENSIONS, DuplicateAction, Engine, ProxyMode, ServerLimit, Settings, SiteLogin, ThemeMode, WindowStyle,
    default_categories,
};

use crate::app;
use crate::assets::Icon;
use crate::format;
use crate::theme::Theme;
use crate::widgets::text_input::TextInput;
use crate::widgets::{
    Button, Checkbox, icon, radio_group, radio_row, section_title, separator, stepper, title_bar, truncated,
};

const SECTIONS: [(&str, Icon); 8] = [
    ("General", Icon::Sliders),
    ("Downloads", Icon::ArrowDown),
    ("Connection", Icon::Bolt),
    ("Proxy", Icon::Globe),
    ("Site Logins", Icon::Info),
    ("Save To", Icon::Folder),
    ("File Types", Icon::Document),
    ("Browser", Icon::External),
];
/// Adds the browsers installed on this computer that the list doesn't have yet, and
/// fills in where listed ones are.
fn merge_installed_browsers(list: &mut Vec<BrowserRule>) {
    for (name, path) in crate::platform::installed_browsers() {
        let Some(exe) = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()) else { continue };
        match list.iter_mut().find(|b| b.exe.eq_ignore_ascii_case(&exe)) {
            Some(known) => {
                // Names the engine guessed from the file name give way to the real ones.
                if known.path.is_none() || known.name.eq_ignore_ascii_case(exe.trim_end_matches(".exe")) {
                    known.name = name;
                }
                known.path.get_or_insert(path);
            }
            None => list.push(BrowserRule { name, exe, path: Some(path), enabled: true }),
        }
    }
    list.sort_by_key(|b| b.name.to_lowercase());
}

const CONNECTION_PRESETS: [u32; 7] = [1, 2, 4, 8, 16, 24, 32];
const NAV_WIDTH: f32 = 176.;

struct LoginRow {
    site: Entity<TextInput>,
    username: Entity<TextInput>,
    password: Entity<TextInput>,
}

struct LimitRow {
    server: Entity<TextInput>,
    connections: u32,
}

pub struct OptionsWindow {
    engine: Arc<Engine>,
    draft: Settings,
    pub(crate) tab: usize,
    download_dir: Entity<TextInput>,
    temp_dir: Entity<TextInput>,
    proxy: Entity<TextInput>,
    proxy_username: Entity<TextInput>,
    proxy_password: Entity<TextInput>,
    user_agent: Entity<TextInput>,
    speed_limit: Entity<TextInput>,
    min_split: Entity<TextInput>,
    retry_delay: Entity<TextInput>,
    connect_timeout: Entity<TextInput>,
    read_timeout: Entity<TextInput>,
    quota_megabytes: Entity<TextInput>,
    quota_hours: Entity<TextInput>,
    port: Entity<TextInput>,
    virus_scanner: Entity<TextInput>,
    excluded_sites: Entity<TextInput>,
    capture_types: Entity<TextInput>,
    /// The highlighted row in the browser list, for Remove.
    selected_browser: Option<usize>,
    extensions: Vec<(String, Entity<TextInput>)>,
    /// Folder override per category; empty uses the default.
    category_folders: Vec<(String, Entity<TextInput>)>,
    logins: Vec<LoginRow>,
    limits: Vec<LimitRow>,
    error: Option<String>,
}

fn input(cx: &mut Context<OptionsWindow>, placeholder: &str, text: impl Into<SharedString>) -> Entity<TextInput> {
    let placeholder = placeholder.to_string();
    let text = text.into();
    cx.new(move |cx| TextInput::new(cx, placeholder).with_text(text))
}

fn number(cx: &mut Context<OptionsWindow>, value: u64) -> Entity<TextInput> {
    cx.new(move |cx| TextInput::new(cx, "0").with_text(value.to_string()).numeric())
}

fn secret(cx: &mut Context<OptionsWindow>, placeholder: &str, text: String) -> Entity<TextInput> {
    let placeholder = placeholder.to_string();
    cx.new(move |cx| TextInput::new(cx, placeholder).masked().with_text(text))
}

impl OptionsWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let engine = app::engine(cx);
        let mut draft = engine.settings();
        merge_installed_browsers(&mut draft.browsers);
        cx.observe_window_appearance(window, |_, _, cx| cx.notify()).detach();
        let extensions = draft
            .categories
            .iter()
            .map(|c| (c.name.clone(), input(cx, "ext1 ext2 …", c.extensions.join(" "))))
            .collect();
        let category_folders = draft
            .categories
            .iter()
            .map(|c| {
                let folder = c.folder.as_deref().map(format::display_path).unwrap_or_default();
                (c.name.clone(), input(cx, "Default", folder))
            })
            .collect();
        let logins = draft
            .site_logins
            .iter()
            .map(|l| LoginRow {
                site: input(cx, "example.com", l.site.clone()),
                username: input(cx, "User name", l.username.clone()),
                password: secret(cx, "Password", l.password.clone()),
            })
            .collect();
        let limits = draft
            .server_limits
            .iter()
            .map(|l| LimitRow { server: input(cx, "example.com", l.server.clone()), connections: l.max_connections })
            .collect();
        Self {
            download_dir: input(cx, "Folder", format::display_path(&draft.download_dir)),
            temp_dir: input(
                cx,
                "Next to the downloaded file",
                draft.temp_dir.as_deref().map(format::display_path).unwrap_or_default(),
            ),
            proxy: input(cx, "http://host:8080 or socks5://host:1080", draft.proxy.clone()),
            proxy_username: input(cx, "Optional", draft.proxy_username.clone()),
            proxy_password: secret(cx, "Optional", draft.proxy_password.clone()),
            user_agent: input(cx, "User agent", draft.user_agent.clone()),
            speed_limit: number(cx, draft.global_speed_limit / 1024),
            min_split: number(cx, draft.min_split_size / 1024),
            retry_delay: number(cx, draft.retry_delay_secs),
            connect_timeout: number(cx, draft.connect_timeout_secs),
            read_timeout: number(cx, draft.read_timeout_secs),
            quota_megabytes: number(cx, draft.quota.megabytes),
            quota_hours: number(cx, draft.quota.hours),
            port: number(cx, u64::from(draft.integration_port)),
            virus_scanner: input(cx, "Microsoft Defender", draft.virus_scanner.clone()),
            excluded_sites: input(cx, "*.example.com other.site", draft.excluded_sites.join(" ")),
            capture_types: input(cx, "zip exe mp4 …", draft.capture_extensions.join(" ")),
            selected_browser: None,
            extensions,
            category_folders,
            logins,
            limits,
            engine,
            draft,
            tab: 0,
            error: None,
        }
    }

    fn parse_number(&self, entity: &Entity<TextInput>, what: &str, cx: &Context<Self>) -> Result<u64, String> {
        let text = entity.read(cx).text();
        text.trim().parse::<u64>().map_err(|_| format!("{what} must be a whole number."))
    }

    fn collect(&self, cx: &Context<Self>) -> Result<Settings, String> {
        let mut s = self.draft.clone();
        let dir = self.download_dir.read(cx).text();
        if dir.trim().is_empty() {
            return Err("Choose a download folder.".into());
        }
        s.download_dir = format::expand_home(dir.trim());
        let temp = self.temp_dir.read(cx).text();
        s.temp_dir = (!temp.trim().is_empty()).then(|| format::expand_home(temp.trim()));
        s.proxy = self.proxy.read(cx).text().trim().to_string();
        if s.proxy_mode == ProxyMode::Manual {
            if s.proxy.is_empty() {
                return Err("Enter the proxy address, or choose another proxy option.".into());
            }
            if url::Url::parse(&s.proxy).is_err() {
                return Err("The proxy must be a URL such as http://host:8080 or socks5://host:1080.".into());
            }
        }
        s.proxy_username = self.proxy_username.read(cx).text().trim().to_string();
        s.proxy_password = self.proxy_password.read(cx).text();
        let ua = self.user_agent.read(cx).text();
        if !ua.trim().is_empty() {
            s.user_agent = ua.trim().to_string();
        }
        s.global_speed_limit = self.parse_number(&self.speed_limit, "The speed limit", cx)? * 1024;
        s.min_split_size = self.parse_number(&self.min_split, "The minimum segment size", cx)?.max(256) * 1024;
        s.retry_delay_secs = self.parse_number(&self.retry_delay, "The retry delay", cx)?;
        s.connect_timeout_secs = self.parse_number(&self.connect_timeout, "The connection timeout", cx)?;
        s.read_timeout_secs = self.parse_number(&self.read_timeout, "The read timeout", cx)?;
        s.quota.megabytes = self.parse_number(&self.quota_megabytes, "The download limit", cx)?;
        s.quota.hours = self.parse_number(&self.quota_hours, "The download limit period", cx)?;
        let port = self.parse_number(&self.port, "The port", cx)?;
        s.integration_port = u16::try_from(port)
            .ok()
            .filter(|p| *p >= 1024)
            .ok_or("The port must be between 1024 and 65535.")?;
        s.virus_scanner = self.virus_scanner.read(cx).text().trim().to_string();
        s.excluded_sites = self
            .excluded_sites
            .read(cx)
            .text()
            .split([' ', ',', ';'])
            .map(|site| site.trim().to_string())
            .filter(|site| !site.is_empty())
            .collect();
        s.capture_extensions = self
            .capture_types
            .read(cx)
            .text()
            .split([' ', ',', ';'])
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        s.categories = self
            .extensions
            .iter()
            .map(|(name, entity)| {
                let folder = self
                    .category_folders
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, e)| e.read(cx).text().trim().to_string())
                    .filter(|f| !f.is_empty())
                    .map(|f| format::expand_home(&f));
                CategoryDef {
                    name: name.clone(),
                    extensions: entity
                        .read(cx)
                        .text()
                        .split([' ', ',', ';'])
                        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
                        .filter(|e| !e.is_empty())
                        .collect(),
                    folder,
                }
            })
            .collect();
        s.site_logins = self
            .logins
            .iter()
            .map(|row| SiteLogin {
                site: row.site.read(cx).text().trim().to_string(),
                username: row.username.read(cx).text().trim().to_string(),
                password: row.password.read(cx).text(),
            })
            .filter(|l| !l.site.is_empty())
            .collect();
        s.server_limits = self
            .limits
            .iter()
            .map(|row| ServerLimit { server: row.server.read(cx).text().trim().to_string(), max_connections: row.connections })
            .filter(|l| !l.server.is_empty())
            .collect();
        Ok(s)
    }

    fn apply(&mut self, cx: &mut Context<Self>) -> bool {
        let result = self.collect(cx).and_then(|settings| {
            self.engine.update_settings(settings.clone()).map_err(|e| e.to_string())?;
            Ok(settings)
        });
        match result {
            Ok(settings) => {
                crate::app::apply_system_settings(&settings, cx);
                self.draft = settings;
                self.error = None;
                cx.notify();
                true
            }
            Err(e) => {
                self.error = Some(e);
                cx.notify();
                false
            }
        }
    }

    /// Lets the user pick a folder and puts it in `target`.
    fn browse_into(&mut self, target: Entity<TextInput>, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |_, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                cx.update(|_, cx| target.update(cx, |i, cx| i.set_text(format::display_path(&path), cx))).ok();
            }
        })
        .detach();
    }

    fn row(label: &'static str, field: impl IntoElement, theme: &Theme) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .min_h(px(30.))
            .child(div().w(px(170.)).flex_none().text_color(theme.muted).child(label))
            .child(div().flex_1().min_w_0().flex().items_center().gap(px(8.)).child(field))
    }

    fn hint(text: impl Into<SharedString>, theme: &Theme) -> impl IntoElement {
        div().text_size(px(12.)).text_color(theme.muted).child(text.into())
    }

    fn unit(field: impl IntoElement, unit: &'static str) -> impl IntoElement {
        div().flex().items_center().gap(px(8.)).child(div().w(px(90.)).child(field)).child(unit)
    }

    /// Window style and notification-area behaviour, which only Windows has.
    fn render_windows_section(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let d = &self.draft;
        let style_note = if crate::platform::supports_backdrops() {
            "Liquid Glass is light and Liquid Glass Smoke dark, whatever the theme; both show what is behind \
             the window. Classic is Tachyon's original look."
        } else {
            "Liquid Glass needs Windows 10 1809 or later; Solid is used instead."
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(section_title("Appearance", theme))
            .child(radio_group(
                "window-style",
                WindowStyle::ALL.iter().map(|s| (*s, SharedString::from(s.label()))).collect(),
                d.window_style,
                theme,
                cx.listener(|this, style: &WindowStyle, _, cx| {
                    this.draft.window_style = *style;
                    cx.notify();
                }),
            ))
            .child(Self::hint(style_note, theme))
            .child(Self::row(
                "Theme",
                radio_row(
                    "theme-mode",
                    ThemeMode::ALL.iter().map(|m| (*m, SharedString::from(m.label()))).collect(),
                    d.theme_mode,
                    theme,
                    cx.listener(|this, mode: &ThemeMode, _, cx| {
                        this.draft.theme_mode = *mode;
                        cx.notify();
                    }),
                ),
                theme,
            ))
            .child(div().h(px(6.)))
            .child(section_title("Notification area", theme))
            .child(
                Checkbox::new("close-to-tray", "Keep running in the notification area when the window is closed", d.close_to_tray)
                    .on_toggle(cx.listener(|this, v: &bool, _, cx| {
                        this.draft.close_to_tray = *v;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("start-with-system", "Start Tachyon when I sign in to Windows", d.start_with_system).on_toggle(
                    cx.listener(|this, v: &bool, _, cx| {
                        this.draft.start_with_system = *v;
                        cx.notify();
                    }),
                ),
            )
            .child(div().h(px(6.)))
    }

    fn render_general(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let d = &self.draft;
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .when(cfg!(windows), |this| this.child(self.render_windows_section(theme, cx)))
            .child(section_title("Dialogs", theme))
            .child(Checkbox::new("show-progress", "Show the progress window when a download starts", d.show_progress_window).on_toggle(
                cx.listener(|this, v: &bool, _, cx| {
                    this.draft.show_progress_window = *v;
                    cx.notify();
                }),
            ))
            .child(Checkbox::new("show-complete", "Show the download complete dialog", d.show_complete_dialog).on_toggle(
                cx.listener(|this, v: &bool, _, cx| {
                    this.draft.show_complete_dialog = *v;
                    cx.notify();
                }),
            ))
            .child(Checkbox::new("clipboard", "Offer to download links copied to the clipboard", d.monitor_clipboard).on_toggle(
                cx.listener(|this, v: &bool, _, cx| {
                    this.draft.monitor_clipboard = *v;
                    cx.notify();
                }),
            ))
            .child(div().h(px(6.)))
            .child(section_title("Updates", theme))
            .child(Checkbox::new("check-updates", "Check for new versions of Tachyon every day", d.check_for_updates).on_toggle(
                cx.listener(|this, v: &bool, _, cx| {
                    this.draft.check_for_updates = *v;
                    cx.notify();
                }),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(Button::new("check-now", "Check Now").on_click(|_, _, cx| crate::updater::check(true, cx)))
                    .child(div().text_color(theme.muted).child(crate::updater::status_line(cx))),
            )
            .child(div().h(px(6.)))
            .child(section_title("Sounds", theme))
            .child(Checkbox::new("sounds", "Play a sound when a download finishes or fails", d.play_sounds).on_toggle(
                cx.listener(|this, v: &bool, _, cx| {
                    this.draft.play_sounds = *v;
                    cx.notify();
                }),
            ))
            .into_any_element()
    }

    fn render_downloads(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let d = &self.draft;
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(section_title("If a link is already in the list", theme))
            .child(radio_group(
                "duplicates",
                DuplicateAction::ALL.iter().map(|a| (*a, SharedString::from(a.label()))).collect(),
                d.duplicate_action,
                theme,
                cx.listener(|this, action: &DuplicateAction, _, cx| {
                    this.draft.duplicate_action = *action;
                    cx.notify();
                }),
            ))
            .child(div().h(px(6.)))
            .child(section_title("Files", theme))
            .child(
                Checkbox::new("server-time", "Give finished files the date reported by the server", d.use_server_file_time)
                    .on_toggle(cx.listener(|this, v: &bool, _, cx| {
                        this.draft.use_server_file_time = *v;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new(
                    "ignore-modified",
                    "Resume even if the file's modification date changed on the server",
                    d.ignore_modified_on_resume,
                )
                .on_toggle(cx.listener(|this, v: &bool, _, cx| {
                    this.draft.ignore_modified_on_resume = *v;
                    cx.notify();
                })),
            )
            .child(Self::hint(
                "Downloads still restart from scratch when the server says the file itself changed (ETag).",
                theme,
            ))
            .child(div().h(px(6.)))
            .child(section_title("Virus scanning", theme))
            .child(Checkbox::new("virus-scan", "Scan finished downloads for viruses", d.virus_scan).on_toggle(cx.listener(
                |this, v: &bool, _, cx| {
                    this.draft.virus_scan = *v;
                    cx.notify();
                },
            )))
            .child(div().when(!d.virus_scan, |this| this.opacity(0.5)).child(Self::row("Scanner", self.virus_scanner.clone(), theme)))
            .child(Self::hint(
                "Leave empty to use Microsoft Defender, which quarantines anything it finds. For another scanner, \
                 enter its program and arguments; {file} stands for the downloaded file.",
                theme,
            ))
            .child(div().h(px(6.)))
            .child(section_title("Identification", theme))
            .child(Self::row("User agent", self.user_agent.clone(), theme))
            .into_any_element()
    }

    fn render_connection(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let d = &self.draft;
        let current = d.max_connections;
        let theme_copy = *theme;
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(section_title("Acceleration", theme))
            .child(Self::row(
                "Connections per download",
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(stepper("max-conn", current, 1, Settings::MAX_CONNECTIONS_LIMIT, theme, cx.listener(
                        |this, v: &u32, _, cx| {
                            this.draft.max_connections = *v;
                            cx.notify();
                        },
                    )))
                    .children(CONNECTION_PRESETS.iter().map(|&n| {
                        let active = n == current;
                        div()
                            .id(("preset", n as usize))
                            .px(px(7.))
                            .h(px(22.))
                            .flex()
                            .items_center()
                            .rounded(px(theme_copy.radius))
                            .text_size(px(12.))
                            .cursor_pointer()
                            .text_color(if active { theme_copy.on_accent } else { theme_copy.muted })
                            .when(active, |this| this.bg(theme_copy.accent))
                            .when(!active, |this| this.hover(|s| s.bg(theme_copy.hover)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.draft.max_connections = n;
                                cx.notify();
                            }))
                            .child(n.to_string())
                    })),
                theme,
            ))
            .child(Self::hint(
                "Each download is split dynamically: when a connection finishes, it takes over half of the largest remaining part.",
                theme,
            ))
            .child(Self::row("Minimum segment size", Self::unit(self.min_split.clone(), "KB"), theme))
            .child(Self::row("Speed limit (all downloads)", Self::unit(self.speed_limit.clone(), "KB/s  (0 = unlimited)"), theme))
            .child(div().h(px(6.)))
            .child(section_title("Exceptions by server", theme))
            .child(Self::hint("Use a different number of connections for some servers, e.g. ones that limit them.", theme))
            .children(self.limits.iter().enumerate().map(|(ix, row)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().child(row.server.clone()))
                    .child(stepper(
                        "limit-conn",
                        row.connections,
                        1,
                        Settings::MAX_CONNECTIONS_LIMIT,
                        theme,
                        cx.listener(move |this, v: &u32, _, cx| {
                            if let Some(row) = this.limits.get_mut(ix) {
                                row.connections = *v;
                            }
                            cx.notify();
                        }),
                    ))
                    .child(Button::new(("remove-limit", ix), "Remove").compact().on_click(cx.listener(
                        move |this, _, _, cx| {
                            if ix < this.limits.len() {
                                this.limits.remove(ix);
                            }
                            cx.notify();
                        },
                    )))
            }))
            .child(div().child(Button::new("add-limit", "Add server").icon(Icon::Add).compact().on_click(cx.listener(
                |this, _, _, cx| {
                    let server = input(cx, "example.com", "");
                    this.limits.push(LimitRow { server, connections: 2 });
                    cx.notify();
                },
            ))))
            .child(div().h(px(6.)))
            .child(section_title("Download limit", theme))
            .child(Checkbox::new("quota", "Stop downloading after a volume of data per period", d.quota.enabled).on_toggle(
                cx.listener(|this, v: &bool, _, cx| {
                    this.draft.quota.enabled = *v;
                    cx.notify();
                }),
            ))
            .child(
                div()
                    .when(!d.quota.enabled, |this| this.opacity(0.5))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child("No more than")
                    .child(div().w(px(80.)).child(self.quota_megabytes.clone()))
                    .child("MB every")
                    .child(div().w(px(64.)).child(self.quota_hours.clone()))
                    .child("hours"),
            )
            .child(div().h(px(6.)))
            .child(section_title("Reliability", theme))
            .child(Self::row(
                "Retries per connection",
                stepper("retries", d.max_retries, 0, 99, theme, cx.listener(|this, v: &u32, _, cx| {
                    this.draft.max_retries = *v;
                    cx.notify();
                })),
                theme,
            ))
            .child(Self::row("Delay between retries", Self::unit(self.retry_delay.clone(), "seconds"), theme))
            .child(Self::row("Connection timeout", Self::unit(self.connect_timeout.clone(), "seconds"), theme))
            .child(Self::row("Read timeout", Self::unit(self.read_timeout.clone(), "seconds"), theme))
            .into_any_element()
    }

    fn render_proxy(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let d = &self.draft;
        let manual = d.proxy_mode == ProxyMode::Manual;
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(section_title("Proxy", theme))
            .child(radio_group(
                "proxy-mode",
                vec![
                    (ProxyMode::System, "Use the system proxy settings".into()),
                    (ProxyMode::None, "Don't use a proxy".into()),
                    (ProxyMode::Manual, "Use this proxy:".into()),
                ],
                d.proxy_mode,
                theme,
                cx.listener(|this, mode: &ProxyMode, _, cx| {
                    this.draft.proxy_mode = *mode;
                    cx.notify();
                }),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .pl(px(24.))
                    .when(!manual, |this| this.opacity(0.5))
                    .child(Self::row("Address", self.proxy.clone(), theme))
                    .child(Self::row("User name", self.proxy_username.clone(), theme))
                    .child(Self::row("Password", self.proxy_password.clone(), theme))
                    .child(Self::hint(
                        "HTTP, HTTPS and SOCKS5 proxies are supported. The password is stored encrypted for your Windows account.",
                        theme,
                    )),
            )
            .into_any_element()
    }

    fn render_logins(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(section_title("User names and passwords for sites", theme))
            .child(Self::hint(
                "Sent automatically to matching sites. example.com also covers its subdomains; add a path \
                 (example.com/members) to limit it. Passwords are stored encrypted for your Windows account.",
                theme,
            ))
            .when(!self.logins.is_empty(), |this| {
                this.child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .text_color(theme.muted)
                        .child(div().flex_1().child("Site"))
                        .child(div().w(px(140.)).child("User name"))
                        .child(div().w(px(140.)).child("Password"))
                        .child(div().w(px(72.))),
                )
            })
            .children(self.logins.iter().enumerate().map(|(ix, row)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().child(row.site.clone()))
                    .child(div().w(px(140.)).child(row.username.clone()))
                    .child(div().w(px(140.)).child(row.password.clone()))
                    .child(div().w(px(72.)).child(Button::new(("remove-login", ix), "Remove").compact().on_click(
                        cx.listener(move |this, _, _, cx| {
                            if ix < this.logins.len() {
                                this.logins.remove(ix);
                            }
                            cx.notify();
                        }),
                    )))
            }))
            .child(div().child(Button::new("add-login", "Add login").icon(Icon::Add).compact().on_click(cx.listener(
                |this, _, _, cx| {
                    let row = LoginRow {
                        site: input(cx, "example.com", ""),
                        username: input(cx, "User name", ""),
                        password: secret(cx, "Password", String::new()),
                    };
                    this.logins.push(row);
                    cx.notify();
                },
            ))))
            .into_any_element()
    }

    fn render_save_to(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let d = &self.draft;
        let base = format::expand_home(self.download_dir.read(cx).text().trim());
        let download_dir = self.download_dir.clone();
        let temp_dir = self.temp_dir.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(section_title("Location", theme))
            .child(Self::row(
                "Download folder",
                div()
                    .flex()
                    .flex_1()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().child(self.download_dir.clone()))
                    .child(Button::new("browse", "Browse…").on_click(cx.listener(move |this, _, window, cx| {
                        this.browse_into(download_dir.clone(), window, cx)
                    }))),
                theme,
            ))
            .child(Checkbox::new("subfolders", "Save each category in its own subfolder", d.use_category_folders).on_toggle(
                cx.listener(|this, v: &bool, _, cx| {
                    this.draft.use_category_folders = *v;
                    cx.notify();
                }),
            ))
            .child(div().h(px(6.)))
            .child(section_title("Where each category goes", theme))
            .child(Self::hint("Leave a folder empty to use the default shown.", theme))
            .children(self.category_folders.iter().enumerate().map(|(ix, (name, entity))| {
                let default = if d.use_category_folders { base.join(name) } else { base.clone() };
                let target = entity.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(icon(Icon::for_category(name), px(14.), theme.muted))
                            .child(div().w(px(92.)).flex_none().child(name.clone()))
                            .child(div().flex_1().min_w_0().child(entity.clone()))
                            .child(Button::new(("browse-cat", ix), "Browse…").on_click(cx.listener(move |this, _, window, cx| {
                                this.browse_into(target.clone(), window, cx)
                            }))),
                    )
                    .child(
                        div()
                            .pl(px(114.))
                            .text_size(px(11.))
                            .text_color(theme.faint)
                            .child(truncated(format!("Default: {}", format::display_path(&default)))),
                    )
            }))
            .child(div().h(px(6.)))
            .child(section_title("Temporary folder", theme))
            .child(Self::row(
                "Unfinished downloads",
                div()
                    .flex()
                    .flex_1()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().child(self.temp_dir.clone()))
                    .child(Button::new("browse-temp", "Browse…").on_click(cx.listener(move |this, _, window, cx| {
                        this.browse_into(temp_dir.clone(), window, cx)
                    }))),
                theme,
            ))
            .child(Self::hint(
                "Leave empty to keep unfinished files next to where they will be saved. A folder on another drive \
                 can be faster, as IDM suggests; finished files are then moved into place.",
                theme,
            ))
            .into_any_element()
    }

    fn render_file_types(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(section_title("Extensions by category", theme))
            .child(Self::hint(
                "New downloads are sorted into a category by extension. Separate extensions with spaces.",
                theme,
            ))
            .children(self.extensions.iter().map(|(name, entity)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(icon(Icon::for_category(name), px(14.), theme.muted))
                    .child(div().w(px(96.)).flex_none().child(name.clone()))
                    .child(div().flex_1().min_w_0().child(entity.clone()))
            }))
            .child(div().pt(px(4.)).child(Button::new("reset-types", "Restore Defaults").on_click(cx.listener(
                |this, _, _, cx| {
                    let defaults = default_categories();
                    for (name, entity) in &this.extensions {
                        if let Some(def) = defaults.iter().find(|d| &d.name == name) {
                            let text = def.extensions.join(" ");
                            entity.update(cx, |i, cx| i.set_text(text, cx));
                        }
                    }
                },
            ))))
            .into_any_element()
    }

    /// Lets the user pick any program and adds it to the browser list.
    fn add_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Add".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
                && let Some(exe) = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase())
            {
                this.update(cx, |this, cx| {
                    let list = &mut this.draft.browsers;
                    match list.iter().position(|b| b.exe.eq_ignore_ascii_case(&exe)) {
                        Some(ix) => list[ix].enabled = true,
                        None => {
                            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(exe.clone());
                            list.push(BrowserRule { name, exe, path: Some(path), enabled: true });
                            list.sort_by_key(|b| b.name.to_lowercase());
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// IDM's "Capture downloads from the following browsers", with whether each
    /// browser's Tachyon extension is in touch.
    fn render_browser_list(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = self.engine.connected_browsers();
        let theme = *theme;
        let rows = self.draft.browsers.iter().enumerate().map(|(ix, browser)| {
            let selected = self.selected_browser == Some(ix);
            let online = connected.iter().any(|exe| exe.eq_ignore_ascii_case(&browser.exe));
            let program_icon = browser
                .path
                .as_deref()
                .and_then(|p| crate::file_icons::file_icon(p, true))
                .map(|image| gpui::img(image).size(px(16.)).flex_none().into_any_element())
                .unwrap_or_else(|| icon(Icon::Globe, px(16.), theme.icon).into_any_element());
            div()
                .id(("browser-row", ix))
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(30.))
                .px(px(8.))
                .rounded(px(theme.radius))
                .when(selected, |this| this.bg(theme.accent_soft))
                .when(!selected, |this| this.hover(move |s| s.bg(theme.hover)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected_browser = Some(ix);
                    cx.notify();
                }))
                .child(Checkbox::new(("browser", ix), "", browser.enabled).on_toggle(cx.listener(
                    move |this, v: &bool, _, cx| {
                        if let Some(b) = this.draft.browsers.get_mut(ix) {
                            b.enabled = *v;
                        }
                        cx.notify();
                    },
                )))
                .child(program_icon)
                .child(div().flex_1().min_w_0().child(truncated(browser.name.clone())))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(if online { theme.text } else { theme.muted })
                        .child(div().size(px(7.)).rounded_full().bg(if online { theme.success } else { theme.border_strong }))
                        .child(if online { "Extension connected" } else { "No extension yet" }),
                )
        });
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .id("browser-list")
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .p(px(4.))
                    .max_h(px(200.))
                    .overflow_y_scroll()
                    .rounded(px(theme.radius))
                    .bg(theme.input)
                    .border_1()
                    .border_color(theme.border)
                    .children(rows)
                    .when(self.draft.browsers.is_empty(), |this| {
                        this.child(div().p(px(8.)).text_color(theme.muted).child("No browsers found. Use Add… to pick one."))
                    }),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(Button::new("add-browser", "Add…").on_click(cx.listener(|this, _, window, cx| {
                        this.add_browser(window, cx)
                    })))
                    .child(
                        Button::new("remove-browser", "Remove")
                            .disabled(self.selected_browser.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(ix) = this.selected_browser.take()
                                    && ix < this.draft.browsers.len()
                                {
                                    this.draft.browsers.remove(ix);
                                }
                                cx.notify();
                            })),
                    ),
            )
    }

    fn render_browser(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let d = &self.draft;
        let extension = app::extension_dir();
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(section_title("Capture downloads from these browsers", theme))
            .child(self.render_browser_list(theme, cx))
            .child(Self::hint(
                "Installed browsers are listed automatically, and a browser whose Tachyon extension connects is added \
                 by itself. Add… takes any other program.",
                theme,
            ))
            .child(div().h(px(6.)))
            .child(section_title("Take over these file types", theme))
            .child(div().flex().child(div().flex_1().min_w_0().child(self.capture_types.clone())))
            .child(div().flex().child(Button::new("reset-capture-types", "Restore Defaults").on_click(cx.listener(
                |this, _, _, cx| {
                    this.capture_types.update(cx, |i, cx| i.set_text(DEFAULT_CAPTURE_EXTENSIONS.split_whitespace().collect::<Vec<_>>().join(" "), cx));
                },
            ))))
            .child(div().h(px(6.)))
            .child(section_title("Browser integration", theme))
            .child(Checkbox::new("integration", "Accept downloads from the Tachyon browser extension", d.browser_integration).on_toggle(
                cx.listener(|this, v: &bool, _, cx| {
                    this.draft.browser_integration = *v;
                    cx.notify();
                }),
            ))
            .child(Self::row(
                "Local port",
                div().flex().items_center().gap(px(8.)).child(div().w(px(90.)).child(self.port.clone())).child(
                    div().text_color(theme.muted).child("on 127.0.0.1 only"),
                ),
                theme,
            ))
            .child(Checkbox::new("auto-start", "Start captured downloads without asking", d.start_captured_immediately).on_toggle(
                cx.listener(|this, v: &bool, _, cx| {
                    this.draft.start_captured_immediately = *v;
                    cx.notify();
                }),
            ))
            .child(div().h(px(6.)))
            .child(section_title("Leave these sites to the browser", theme))
            .child(self.excluded_sites.clone())
            .child(Self::hint(
                "Downloads from these sites are never taken over. Separate them with spaces; *.example.com means its subdomains.",
                theme,
            ))
            .child(div().h(px(6.)))
            .child(section_title("Install the extension", theme))
            .child(Self::hint(
                "Chrome, Edge, Brave, Opera, Vivaldi and other Chromium browsers: open the extensions page \
                 (chrome://extensions), turn on Developer mode, click “Load unpacked” and choose the extension folder.",
                theme,
            ))
            .child(Self::hint(
                "Firefox: open about:debugging, choose This Firefox › Load Temporary Add-on and pick manifest.json \
                 in the Firefox version's folder. It stays until Firefox restarts.",
                theme,
            ))
            .child(Self::hint("Then right-click any link and choose “Download with Tachyon”, or let it take over downloads.", theme))
            .child(div().pt(px(4.)).child(match extension {
                Some(path) => {
                    let data_dir = self.engine.data_dir().to_path_buf();
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            Button::new("show-ext", "Show Extension Folder")
                                .icon(Icon::Folder)
                                .on_click(move |_, _, cx| cx.reveal_path(&path)),
                        )
                        .child(Button::new("show-ext-firefox", "Show Firefox Version").icon(Icon::Folder).on_click(
                            cx.listener(move |this, _, _, cx| match app::firefox_extension_dir(&data_dir) {
                                Ok(dir) => cx.reveal_path(&dir.join("manifest.json")),
                                Err(e) => {
                                    this.error = Some(format!("Couldn't prepare the Firefox version: {e}"));
                                    cx.notify();
                                }
                            }),
                        ))
                        .into_any_element()
                }
                None => Self::hint("The extension folder wasn't found next to the app.", theme).into_any_element(),
            }))
            .into_any_element()
    }

    /// Section list on the left, as in Windows 11 Settings.
    fn render_nav(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *theme;
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(NAV_WIDTH))
            .h_full()
            .gap(px(2.))
            .px(px(8.))
            .py(px(12.))
            .bg(theme.surface)
            .when(!theme.fluent, |this| this.border_r_1().border_color(theme.border))
            .children(SECTIONS.iter().enumerate().map(|(ix, (label, section_icon))| {
                let active = ix == self.tab;
                div()
                    .id(("section", ix))
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(34.))
                    .px(px(12.))
                    .rounded(px(theme.radius))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .when(active, |this| {
                        this.bg(if theme.fluent { theme.hover } else { theme.accent_soft }).child(
                            div().absolute().left_0().top(px(11.)).w(px(3.)).h(px(12.)).rounded_full().bg(theme.accent),
                        )
                    })
                    .when(!active, |this| this.hover(move |s| s.bg(theme.hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = ix;
                        cx.notify();
                    }))
                    .child(icon(*section_icon, px(15.), if theme.fluent {
                        theme.icon
                    } else if active {
                        theme.accent
                    } else {
                        theme.muted
                    }))
                    .child(*label)
            }))
    }
}

impl Render for OptionsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        let body = match self.tab {
            0 => self.render_general(&theme, cx),
            1 => self.render_downloads(&theme, cx),
            2 => self.render_connection(&theme, cx),
            3 => self.render_proxy(&theme, cx),
            4 => self.render_logins(&theme, cx),
            5 => self.render_save_to(&theme, cx),
            6 => self.render_file_types(&theme, cx),
            _ => self.render_browser(&theme, cx),
        };
        let title = SECTIONS.get(self.tab).map(|(label, _)| *label).unwrap_or_default();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.window)
            .text_color(theme.text)
            .text_size(px(13.))
            .child(title_bar("Options", false, window, &theme))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_nav(&theme, cx))
                    .child(
                        // As in the main window: the page sits on a panel that runs to the
                        // bottom corner, with its buttons inside it.
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
                            .child(
                                div()
                                    .id("options-body")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .px(px(24.))
                                    .py(px(18.))
                                    .child(
                                        div()
                                            .pb(px(12.))
                                            .text_size(px(20.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(title),
                                    )
                                    .child(body),
                            )
                            .when(!theme.fluent, |this| this.child(separator(&theme)))
                            .child(self.render_footer(&theme, cx)),
                    ),
            )
    }
}

impl OptionsWindow {
    fn render_footer(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *theme;
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .px(px(20.))
                    .py(px(10.))
                    .when(!theme.fluent, |this| this.bg(theme.chrome))
                    .when(theme.fluent, |this| this.border_t_1().border_color(theme.border))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .when_some(self.error.clone(), |this, e| this.text_color(theme.danger).child(e)),
                    )
                    .child(Button::new("ok", "OK").primary().on_click(cx.listener(|this, _, window, cx| {
                        if this.apply(cx) {
                            window.remove_window();
                        }
                    })))
                    .child(Button::new("cancel", "Cancel").on_click(|_, window, _| window.remove_window()))
                    .child(Button::new("apply", "Apply").on_click(cx.listener(|this, _, _, cx| {
                        this.apply(cx);
                    })))
    }
}
