use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::categories::default_categories;

pub type DownloadId = u64;

/// Persisted lifecycle state of a download.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadStatus {
    /// Not running and not finished (never started, paused or interrupted).
    #[default]
    Stopped,
    /// Resolving the address and waiting for the server to answer.
    Connecting,
    /// At least one connection is receiving data.
    Downloading,
    /// All bytes received; flushing and moving the file into place.
    Finalizing,
    Completed,
    Failed,
}

impl DownloadStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Connecting | Self::Downloading | Self::Finalizing)
    }
}

/// Byte range owned by one connection: `[start, end)`, with `pos` the next byte to write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentState {
    pub start: u64,
    pub pos: u64,
    pub end: u64,
}

impl SegmentState {
    pub fn is_complete(&self) -> bool {
        self.pos >= self.end
    }

    pub fn remaining(&self) -> u64 {
        self.end.saturating_sub(self.pos)
    }

    pub fn downloaded(&self) -> u64 {
        self.pos.saturating_sub(self.start)
    }
}

/// What to do after a download or a queue finishes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PostAction {
    #[default]
    Nothing,
    QuitApp,
    Sleep,
    ShutDown,
}

impl PostAction {
    pub const ALL: [PostAction; 4] = [Self::Nothing, Self::QuitApp, Self::Sleep, Self::ShutDown];

    pub fn label(self) -> &'static str {
        match self {
            Self::Nothing => "Do nothing",
            Self::QuitApp => "Quit Tachyon",
            Self::Sleep => "Put the Mac to sleep",
            Self::ShutDown => "Shut down the Mac",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompletionOptions {
    pub show_dialog: bool,
    pub open_file: bool,
    pub post_action: PostAction,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// Everything needed to (re)issue requests for a download.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RequestSpec {
    pub url: String,
    pub referer: Option<String>,
    pub cookies: Option<String>,
    pub user_agent: Option<String>,
    pub credentials: Option<Credentials>,
}

impl RequestSpec {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Default::default()
        }
    }
}

/// A request to add a new download.
#[derive(Clone, Debug, Default)]
pub struct AddRequest {
    pub request: RequestSpec,
    /// Overrides the file name derived from the server/URL.
    pub file_name: Option<String>,
    /// Overrides the category/default folder.
    pub directory: Option<PathBuf>,
    pub category: Option<String>,
    pub description: String,
    /// Start immediately; otherwise the download waits (in the queue if `queue` is set).
    pub start: bool,
    pub queue: bool,
    /// Size/resume info from an earlier probe, shown in the list before the download starts.
    pub probed: Option<ProbeInfo>,
}

/// Metadata learned from the server before downloading.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeInfo {
    pub final_url: String,
    pub total: Option<u64>,
    pub resumable: bool,
    pub file_name: String,
    pub content_type: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// Per-connection state displayed in the progress window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Receiving,
    Retrying { attempt: u32, reason: String },
    Idle,
    Finished,
    /// Closed to reduce load after the server throttled us; its work was handed back.
    Released(String),
    Failed(String),
}

impl ConnectionState {
    pub fn label(&self) -> String {
        match self {
            Self::Connecting => "Connecting…".into(),
            Self::Receiving => "Receiving data…".into(),
            Self::Retrying { attempt, reason } => format!("Retry #{attempt}: {reason}"),
            Self::Idle => "Idle".into(),
            Self::Finished => "Finished".into(),
            Self::Released(reason) => format!("Closed ({reason}); using fewer connections"),
            Self::Failed(e) => format!("Error: {e}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConnectionSnapshot {
    pub index: usize,
    pub downloaded: u64,
    pub state: ConnectionState,
}

/// A point-in-time view of a download for the UI.
#[derive(Clone, Debug)]
pub struct DownloadSnapshot {
    pub id: DownloadId,
    pub url: String,
    pub file_name: String,
    pub directory: PathBuf,
    pub category: String,
    pub description: String,
    pub size: Option<u64>,
    pub downloaded: u64,
    pub status: DownloadStatus,
    /// Bytes per second over the last few seconds.
    pub speed: u64,
    pub eta_secs: Option<u64>,
    /// `None` until the server has been contacted.
    pub resumable: Option<bool>,
    pub error: Option<String>,
    pub added_at: i64,
    pub last_try: Option<i64>,
    pub completed_at: Option<i64>,
    pub in_queue: bool,
    pub queue_order: u64,
    pub speed_limit: u64,
    pub active_connections: usize,
    pub completion: CompletionOptions,
}

impl DownloadSnapshot {
    pub fn path(&self) -> PathBuf {
        self.directory.join(&self.file_name)
    }

    pub fn progress(&self) -> Option<f64> {
        match self.size {
            Some(0) => Some(1.0),
            Some(total) => Some((self.downloaded as f64 / total as f64).clamp(0.0, 1.0)),
            None => None,
        }
    }
}

/// Segment map and per-connection state for the progress window.
#[derive(Clone, Debug, Default)]
pub struct DownloadDetails {
    pub segments: Vec<SegmentState>,
    pub connections: Vec<ConnectionSnapshot>,
}

/// Notifications for the UI, drained with [`crate::Engine::drain_events`].
#[derive(Clone, Debug)]
pub enum EngineEvent {
    Completed(DownloadId),
    Failed { id: DownloadId, error: String },
    /// The queue ran out of work after being started.
    QueueFinished { post_action: PostAction },
    /// A download was handed over by the browser extension and awaits confirmation.
    Captured(Box<AddRequest>),
    /// A download asked for a system action once it finished.
    PostAction(PostAction),
    /// The download limit (Options → Connection) was reached; downloads were stopped
    /// and can start again at `resumes_at` (Unix time).
    QuotaReached { resumes_at: i64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryDef {
    pub name: String,
    pub extensions: Vec<String>,
    /// Folder override; defaults to `<download_dir>/<name>`.
    pub folder: Option<PathBuf>,
}

/// Time of day as minutes after midnight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayTime {
    pub hour: u8,
    pub minute: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScheduleSettings {
    pub start_enabled: bool,
    pub start_at: DayTime,
    pub stop_enabled: bool,
    pub stop_at: DayTime,
    /// Monday = bit 0 … Sunday = bit 6.
    pub days: u8,
}

impl Default for ScheduleSettings {
    fn default() -> Self {
        Self {
            start_enabled: false,
            start_at: DayTime { hour: 2, minute: 0 },
            stop_enabled: false,
            stop_at: DayTime { hour: 8, minute: 0 },
            days: 0b111_1111,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct QueueSettings {
    pub max_concurrent: u32,
    /// How many times a failed file in the queue is retried before it is skipped.
    pub retries: u32,
    pub post_action: PostAction,
    pub schedule: ScheduleSettings,
}

impl Default for QueueSettings {
    fn default() -> Self {
        Self {
            max_concurrent: 3,
            retries: 3,
            post_action: PostAction::Nothing,
            schedule: ScheduleSettings::default(),
        }
    }
}

/// How windows look. The first three follow Windows 11; `Classic` is Tachyon's
/// original design and the only style on other systems.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowStyle {
    /// The standard Windows 11 look: a subtle wallpaper tint (Mica).
    Mica,
    /// Pale smoky glass over a light blur, with outlined capsule controls.
    LiquidGlass,
    /// The dark version of Liquid Glass.
    LiquidSmoke,
    /// Windows 11 colours on opaque windows, for older systems or when transparency
    /// is distracting.
    Solid,
    /// Tachyon's original look.
    Classic,
}

impl Default for WindowStyle {
    fn default() -> Self {
        if cfg!(windows) { WindowStyle::Mica } else { WindowStyle::Classic }
    }
}

impl WindowStyle {
    /// Classic stays last: it is the fallback index.
    pub const ALL: [WindowStyle; 5] = [
        WindowStyle::Mica,
        WindowStyle::LiquidGlass,
        WindowStyle::LiquidSmoke,
        WindowStyle::Solid,
        WindowStyle::Classic,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WindowStyle::Mica => "Mica",
            WindowStyle::LiquidGlass => "Liquid Glass",
            WindowStyle::LiquidSmoke => "Liquid Glass Smoke",
            WindowStyle::Solid => "Solid",
            WindowStyle::Classic => "Classic",
        }
    }

    /// Whether the style draws over a translucent system backdrop.
    pub fn is_glass(self) -> bool {
        matches!(self, WindowStyle::Mica | WindowStyle::LiquidGlass | WindowStyle::LiquidSmoke)
    }
}

/// Light or dark, or whatever the system uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 3] = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark];

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::System => "Same as Windows",
            ThemeMode::Light => "Light",
            ThemeMode::Dark => "Dark",
        }
    }
}

/// What to do when a link already in the list is added again (IDM's Downloads tab).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateAction {
    /// Ask each time.
    #[default]
    Ask,
    /// Add it again under a numbered name ("file (1).zip").
    AddNumbered,
    /// Download it again over the existing entry and file.
    Overwrite,
    /// Don't add it; show the existing download instead.
    ShowExisting,
}

impl DuplicateAction {
    pub const ALL: [DuplicateAction; 4] =
        [DuplicateAction::Ask, DuplicateAction::AddNumbered, DuplicateAction::Overwrite, DuplicateAction::ShowExisting];

    pub fn label(self) -> &'static str {
        match self {
            DuplicateAction::Ask => "Ask what to do",
            DuplicateAction::AddNumbered => "Add it with a numbered name",
            DuplicateAction::Overwrite => "Download it again, replacing the old file",
            DuplicateAction::ShowExisting => "Show the existing download",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    /// Use the proxy configured in Windows (or the system elsewhere).
    #[default]
    System,
    /// Connect directly.
    None,
    /// Use `Settings::proxy`.
    Manual,
}

/// User name and password sent automatically to a site (IDM's "Sites Logins").
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SiteLogin {
    /// `example.com`, `*.example.com` or `example.com/members`.
    pub site: String,
    pub username: String,
    #[serde(with = "crate::secret::password")]
    pub password: String,
}

/// A different number of connections for one server (IDM's "Exceptions").
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerLimit {
    /// `example.com` or `*.example.com`.
    pub server: String,
    pub max_connections: u32,
}

/// Stop downloading after a volume of data per period (IDM's "Download limits").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadQuota {
    pub enabled: bool,
    pub megabytes: u64,
    pub hours: u64,
}

impl Default for DownloadQuota {
    fn default() -> Self {
        Self { enabled: false, megabytes: 200, hours: 5 }
    }
}

/// Whether `host` matches `pattern`: `example.com` matches itself and its
/// subdomains, `*.example.com` only subdomains. Case-insensitive.
pub fn host_matches(pattern: &str, host: &str) -> bool {
    let pattern = pattern.trim().trim_end_matches('.').to_ascii_lowercase();
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if pattern.is_empty() {
        return false;
    }
    match pattern.strip_prefix("*.") {
        Some(base) => host.ends_with(&format!(".{base}")),
        None => host == pattern || host.ends_with(&format!(".{pattern}")),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Maximum connections per download (IDM's "Max. connections number").
    pub max_connections: u32,
    /// A segment is only split if both halves would be at least this big.
    pub min_split_size: u64,
    pub max_retries: u32,
    pub retry_delay_secs: u64,
    pub connect_timeout_secs: u64,
    pub read_timeout_secs: u64,
    /// Bytes per second across all downloads; 0 means unlimited.
    pub global_speed_limit: u64,
    pub download_dir: PathBuf,
    pub use_category_folders: bool,
    pub user_agent: String,
    pub proxy_mode: ProxyMode,
    /// `http://`, `https://` or `socks5://` proxy URL, used when `proxy_mode` is Manual.
    pub proxy: String,
    pub proxy_username: String,
    #[serde(with = "crate::secret::password")]
    pub proxy_password: String,
    pub show_progress_window: bool,
    pub show_complete_dialog: bool,
    pub monitor_clipboard: bool,
    pub browser_integration: bool,
    pub integration_port: u16,
    /// Start browser-captured downloads without asking first.
    pub start_captured_immediately: bool,
    pub categories: Vec<CategoryDef>,
    pub queue: QueueSettings,
    pub window_style: WindowStyle,
    pub theme_mode: ThemeMode,
    /// Closing the main window keeps Tachyon running in the notification area.
    pub close_to_tray: bool,
    /// Start Tachyon in the notification area when the user signs in.
    pub start_with_system: bool,
    pub duplicate_action: DuplicateAction,
    pub site_logins: Vec<SiteLogin>,
    pub server_limits: Vec<ServerLimit>,
    pub quota: DownloadQuota,
    /// Give finished files the modification time the server reports.
    pub use_server_file_time: bool,
    /// Resume even if the server's Last-Modified changed (an ETag change still restarts).
    pub ignore_modified_on_resume: bool,
    /// Sites whose downloads the browser extension leaves to the browser.
    pub excluded_sites: Vec<String>,
    pub play_sounds: bool,
    /// Scan finished files for viruses.
    pub virus_scan: bool,
    /// Scanner program and arguments; `{file}` is replaced by the path. Empty uses
    /// Microsoft Defender on Windows.
    pub virus_scanner: String,
    /// Where unfinished downloads are kept; empty keeps them next to the final file.
    pub temp_dir: Option<PathBuf>,
}

/// The platform part of the default User-Agent, as browsers on this system send it.
const PLATFORM_UA: &str = if cfg!(windows) {
    "Windows NT 10.0; Win64; x64"
} else if cfg!(target_os = "macos") {
    "Macintosh; Intel Mac OS X 10_15_7"
} else {
    "X11; Linux x86_64"
};

impl Default for Settings {
    fn default() -> Self {
        let download_dir = dirs::download_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            max_connections: 8,
            min_split_size: 1024 * 1024,
            max_retries: 10,
            retry_delay_secs: 3,
            connect_timeout_secs: 20,
            read_timeout_secs: 30,
            global_speed_limit: 0,
            download_dir,
            use_category_folders: true,
            user_agent: format!("Mozilla/5.0 ({PLATFORM_UA}) Tachyon/{}", env!("CARGO_PKG_VERSION")),
            proxy: String::new(),
            show_progress_window: true,
            show_complete_dialog: true,
            monitor_clipboard: false,
            browser_integration: true,
            integration_port: 7311,
            start_captured_immediately: false,
            categories: default_categories(),
            queue: QueueSettings::default(),
            window_style: WindowStyle::default(),
            theme_mode: ThemeMode::default(),
            close_to_tray: true,
            start_with_system: false,
            proxy_mode: ProxyMode::default(),
            proxy_username: String::new(),
            proxy_password: String::new(),
            duplicate_action: DuplicateAction::default(),
            site_logins: Vec::new(),
            server_limits: Vec::new(),
            quota: DownloadQuota::default(),
            use_server_file_time: false,
            ignore_modified_on_resume: false,
            // As IDM: never take over Windows Update downloads.
            excluded_sites: vec![
                "*.update.microsoft.com".into(),
                "download.windowsupdate.com".into(),
                "*.download.windowsupdate.com".into(),
            ],
            play_sounds: false,
            virus_scan: false,
            virus_scanner: String::new(),
            temp_dir: None,
        }
    }
}

impl Settings {
    pub const MAX_CONNECTIONS_LIMIT: u32 = 32;

    /// Connections allowed for `host`: its exception if one matches, else the default.
    pub fn connections_for(&self, host: &str) -> u32 {
        self.server_limits
            .iter()
            .find(|l| host_matches(&l.server, host))
            .map_or(self.max_connections, |l| l.max_connections)
    }

    /// The saved login for `url`, if a site pattern (optionally with a path) matches.
    pub fn login_for(&self, url: &str) -> Option<Credentials> {
        let url = url::Url::parse(url).ok()?;
        let host = url.host_str()?;
        self.site_logins
            .iter()
            .find(|login| {
                let (site, path) = login.site.trim().split_once('/').unwrap_or((login.site.trim(), ""));
                host_matches(site, host) && url.path().trim_start_matches('/').starts_with(path)
            })
            .map(|login| Credentials { username: login.username.clone(), password: login.password.clone() })
    }

    /// Whether the browser extension should leave a download from `url` to the browser.
    pub fn is_excluded_site(&self, url: &str) -> bool {
        url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .is_some_and(|host| self.excluded_sites.iter().any(|p| host_matches(p, &host)))
    }

    /// Clamps values that would break the engine.
    pub fn sanitized(mut self) -> Self {
        self.max_connections = self.max_connections.clamp(1, Self::MAX_CONNECTIONS_LIMIT);
        self.min_split_size = self.min_split_size.max(256 * 1024);
        self.connect_timeout_secs = self.connect_timeout_secs.clamp(1, 600);
        self.read_timeout_secs = self.read_timeout_secs.clamp(1, 600);
        self.retry_delay_secs = self.retry_delay_secs.min(600);
        self.queue.max_concurrent = self.queue.max_concurrent.clamp(1, 32);
        for limit in &mut self.server_limits {
            limit.max_connections = limit.max_connections.clamp(1, Self::MAX_CONNECTIONS_LIMIT);
        }
        self.server_limits.retain(|l| !l.server.trim().is_empty());
        self.site_logins.retain(|l| !l.site.trim().is_empty());
        self.excluded_sites.retain(|s| !s.trim().is_empty());
        self.quota.megabytes = self.quota.megabytes.max(1);
        self.quota.hours = self.quota.hours.clamp(1, 24 * 31);
        // Builds before Windows support saved the macOS default User-Agent everywhere.
        if !cfg!(target_os = "macos") && self.user_agent.contains("Macintosh") && self.user_agent.contains(" Tachyon/") {
            self.user_agent = format!("Mozilla/5.0 ({PLATFORM_UA}) Tachyon/{}", env!("CARGO_PKG_VERSION"));
        }
        self
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn host_patterns() {
        assert!(host_matches("example.com", "example.com"));
        assert!(host_matches("example.com", "dl.example.com"));
        assert!(host_matches("*.example.com", "dl.example.com"));
        assert!(!host_matches("*.example.com", "example.com"));
        assert!(!host_matches("example.com", "badexample.com"));
        assert!(host_matches("EXAMPLE.com", "Example.COM"));
        assert!(!host_matches("", "example.com"));
    }

    #[test]
    fn logins_match_site_and_path() {
        let mut s = Settings::default();
        s.site_logins = vec![
            SiteLogin { site: "example.com/members".into(), username: "m".into(), password: "1".into() },
            SiteLogin { site: "*.files.net".into(), username: "f".into(), password: "2".into() },
        ];
        assert_eq!(s.login_for("https://example.com/members/a.zip").map(|c| c.username), Some("m".into()));
        assert!(s.login_for("https://example.com/public/a.zip").is_none());
        assert_eq!(s.login_for("https://cdn.files.net/x").map(|c| c.username), Some("f".into()));
        assert!(s.login_for("not a url").is_none());
    }

    #[test]
    fn excluded_sites_default_to_windows_update() {
        let s = Settings::default();
        assert!(s.is_excluded_site("http://au.download.windowsupdate.com/x.cab"));
        assert!(!s.is_excluded_site("https://example.com/x.zip"));
    }

    #[test]
    fn server_exceptions_override_connections() {
        let mut s = Settings::default();
        s.server_limits = vec![ServerLimit { server: "slow.example".into(), max_connections: 2 }];
        assert_eq!(s.connections_for("slow.example"), 2);
        assert_eq!(s.connections_for("fast.example"), s.max_connections);
    }

    #[test]
    fn passwords_survive_a_save() {
        let mut s = Settings::default();
        s.proxy_password = "secret".into();
        s.site_logins = vec![SiteLogin { site: "a.b".into(), username: "u".into(), password: "p@ss".into() }];
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.proxy_password, "secret");
        assert_eq!(back.site_logins[0].password, "p@ss");
        if cfg!(windows) {
            assert!(!json.contains("secret") && !json.contains("p@ss"));
        }
    }
}
