//! Video pages (YouTube and the many sites yt-dlp knows) are downloaded through
//! yt-dlp, which fetches picture and sound. Tachyon fetches yt-dlp and QuickJS
//! (which yt-dlp needs for YouTube) into its data folder the first time a video is
//! asked for and keeps them up to date, so they are simply part of it. Picture and
//! sound are joined by Tachyon itself (see `mux`); the system's ffmpeg is used only
//! for pairs it can't join.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

use crate::error::EngineError;
use crate::task::{TaskEnv, TaskInput, TaskOutcome, TaskShared};
use crate::types::{BrowserCookie, DownloadStatus, ProxyMode, Settings};

const YT_DLP: &str = if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" };
const FFMPEG: &str = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
const YT_DLP_URL: &str = if cfg!(windows) {
    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
} else if cfg!(target_os = "macos") {
    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos"
} else {
    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp"
};
/// Reading a page's qualities can take a while on slow connections.
const LIST_TIMEOUT: Duration = Duration::from_secs(45);
/// Marks our progress lines among yt-dlp's other output.
const PROGRESS_TAG: &str = "TACHYON";

/// One quality offered for a video page.
#[derive(Clone, Debug, Serialize)]
pub struct MediaChoice {
    /// Such as "1080p60" or "Audio only".
    pub label: String,
    /// yt-dlp's format selector.
    pub format: String,
    /// Extension of the finished file.
    pub ext: String,
    /// Approximate size of the finished file.
    pub size: Option<u64>,
}

/// What a video page offers.
#[derive(Clone, Debug, Serialize)]
pub struct MediaInfo {
    pub title: String,
    pub choices: Vec<MediaChoice>,
}

/// The helper programs video downloads need.
#[derive(Clone, Debug, Default)]
pub struct MediaTools {
    pub yt_dlp: Option<PathBuf>,
    /// QuickJS, which yt-dlp needs for YouTube.
    pub js: Option<PathBuf>,
    /// Optional: lets yt-dlp join pairs Tachyon can't (such as WebM).
    pub ffmpeg: Option<PathBuf>,
}

/// Finds `name` in Tachyon's tools folder or on the PATH.
fn find(tools_dir: &Path, name: &str) -> Option<PathBuf> {
    let own = tools_dir.join(name);
    if own.is_file() {
        return Some(own);
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

pub(crate) fn tools(tools_dir: &Path) -> MediaTools {
    MediaTools {
        yt_dlp: find(tools_dir, YT_DLP),
        js: Some(tools_dir.join(QJS)).filter(|p| p.is_file()),
        ffmpeg: find(tools_dir, FFMPEG),
    }
}

/// QuickJS, the small JavaScript engine yt-dlp needs for YouTube.
const QJS: &str = if cfg!(windows) { "qjs.exe" } else { "qjs" };
const QJS_URL: &str = if cfg!(windows) {
    "https://github.com/quickjs-ng/quickjs/releases/latest/download/qjs-windows-x86_64.exe"
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    "https://github.com/quickjs-ng/quickjs/releases/latest/download/qjs-darwin-arm64"
} else if cfg!(target_os = "macos") {
    "https://github.com/quickjs-ng/quickjs/releases/latest/download/qjs-darwin-x86_64"
} else {
    "https://github.com/quickjs-ng/quickjs/releases/latest/download/qjs-linux-x86_64"
};
/// QuickJS changes rarely; its latest release is fetched again after this.
const QJS_REFRESH_AFTER: Duration = Duration::from_secs(30 * 24 * 3600);
/// yt-dlp is updated when its copy is older than this, as sites change often.
const UPDATE_AFTER: Duration = Duration::from_secs(7 * 24 * 3600);

fn io(e: impl std::fmt::Display) -> EngineError {
    EngineError::Io(e.to_string())
}

/// Saves `url` to `target`, through a temporary file so a cut download leaves nothing.
async fn fetch(client: &Client, url: &str, target: &Path) -> Result<(), EngineError> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let response = client.get(url).send().await.map_err(|e| EngineError::Network(e.to_string()))?;
    if !response.status().is_success() {
        return Err(EngineError::Http {
            status: response.status().as_u16(),
            reason: response.status().canonical_reason().unwrap_or("").to_string(),
            retry_after: None,
        });
    }
    let temp = target.with_extension("download");
    let mut file = tokio::fs::File::create(&temp).await.map_err(io)?;
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|e| EngineError::Network(e.to_string()))?;
        file.write_all(&chunk).await.map_err(io)?;
    }
    file.flush().await.map_err(io)?;
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755));
    }
    tokio::fs::rename(&temp, target).await.map_err(io)
}

/// One fetch of each tool at a time, however many pages ask at once.
fn install_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// yt-dlp, fetched into the tools folder the first time it is needed and kept up
/// to date in the background. Part of Tachyon, so the user is never asked.
pub(crate) async fn ensure_yt_dlp(client: &Client, settings: &Settings, tools_dir: &Path) -> Result<PathBuf, EngineError> {
    let _guard = install_lock().lock().await;
    let own = tools_dir.join(YT_DLP);
    if own.is_file() {
        if age(&own).is_some_and(|age| age > UPDATE_AFTER) {
            // Touch it first, so a failed update isn't retried on every page.
            let _ = std::fs::File::options().append(true).open(&own).and_then(|f| f.set_modified(std::time::SystemTime::now()));
            let mut cmd = command(&own, settings, None);
            cmd.arg("-U");
            tokio::spawn(async move {
                match cmd.output().await {
                    Ok(out) => log::info!("yt-dlp update: {}", String::from_utf8_lossy(&out.stdout).trim()),
                    Err(e) => log::warn!("yt-dlp update failed: {e}"),
                }
            });
        }
        return Ok(own);
    }
    if let Some(found) = find(tools_dir, YT_DLP) {
        return Ok(found);
    }
    tokio::fs::create_dir_all(tools_dir).await.map_err(io)?;
    log::info!("fetching yt-dlp");
    fetch(client, YT_DLP_URL, &own).await?;
    Ok(own)
}

fn age(path: &Path) -> Option<Duration> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok())
}

/// QuickJS, fetched (or refreshed to its latest release) into the tools folder. If
/// that fails yt-dlp still works, with fewer YouTube qualities.
async fn ensure_qjs(client: &Client, tools_dir: &Path) -> Option<PathBuf> {
    let _guard = install_lock().lock().await;
    let own = tools_dir.join(QJS);
    let fresh = own.is_file() && age(&own).is_some_and(|age| age < QJS_REFRESH_AFTER);
    if !fresh {
        let _ = tokio::fs::create_dir_all(tools_dir).await;
        log::info!("fetching QuickJS");
        if let Err(e) = fetch(client, QJS_URL, &own).await {
            log::warn!("fetching QuickJS failed: {e}");
        }
    }
    own.is_file().then_some(own)
}

/// yt-dlp and QuickJS, ready to run.
async fn ready_tools(client: &Client, settings: &Settings, tools_dir: &Path) -> Result<(PathBuf, Option<PathBuf>), EngineError> {
    let yt_dlp = ensure_yt_dlp(client, settings, tools_dir).await?;
    Ok((yt_dlp, ensure_qjs(client, tools_dir).await))
}

/// A yt-dlp command with the options every call shares.
fn command(yt_dlp: &Path, settings: &Settings, js: Option<&Path>) -> Command {
    let mut cmd = Command::new(yt_dlp);
    // UTF-8 output: otherwise Windows' code page mangles titles such as "Jazz • Rain".
    cmd.args(["--no-playlist", "--no-colors", "--no-warnings", "--encoding", "utf-8"]);
    cmd.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1");
    if let Some(js) = js {
        cmd.arg("--js-runtimes").arg(format!("quickjs:{}", js.display()));
    }
    if settings.proxy_mode == ProxyMode::Manual && !settings.proxy.is_empty() {
        cmd.arg("--proxy").arg(&settings.proxy);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    {
        // No console window flashing up.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// The last error line yt-dlp printed, without its prefix.
fn last_error(stderr: &str) -> String {
    stderr
        .lines()
        .rev()
        .find(|l| l.starts_with("ERROR:"))
        .map(|l| l.trim_start_matches("ERROR:").trim().to_string())
        .unwrap_or_else(|| stderr.lines().last().unwrap_or("yt-dlp failed").trim().to_string())
}

/// The browser's cookies in the file format yt-dlp reads, removed when dropped.
struct CookieFile(PathBuf);

impl CookieFile {
    fn write(tools_dir: &Path, cookies: &[BrowserCookie]) -> Option<Self> {
        if cookies.is_empty() {
            return None;
        }
        let mut text = String::from("# Netscape HTTP Cookie File\n");
        for c in cookies {
            // A cookie for subdomains too is written with a leading dot.
            let domain = if c.host_only { c.domain.trim_start_matches('.').to_string() } else if c.domain.starts_with('.') { c.domain.clone() } else { format!(".{}", c.domain) };
            let flag = |b: bool| if b { "TRUE" } else { "FALSE" };
            let path = if c.path.is_empty() { "/" } else { c.path.as_str() };
            let expires = c.expiration_date.map(|t| t as i64).unwrap_or(0);
            text.push_str(&format!("{domain}\t{}\t{path}\t{}\t{expires}\t{}\t{}\n", flag(!c.host_only), flag(c.secure), c.name, c.value));
        }
        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let path = tools_dir.join(format!("cookies-{unique}.txt"));
        std::fs::create_dir_all(tools_dir).ok()?;
        std::fs::write(&path, text).ok()?;
        Some(Self(path))
    }

    fn add_to(&self, cmd: &mut Command) {
        cmd.arg("--cookies").arg(&self.0);
    }
}

impl Drop for CookieFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Asks yt-dlp what `url` offers, signed in with the browser's `cookies`.
pub(crate) async fn list(
    url: &str,
    cookies: &[BrowserCookie],
    settings: &Settings,
    client: &Client,
    tools_dir: &Path,
) -> Result<MediaInfo, String> {
    let started = std::time::Instant::now();
    let (yt_dlp, js) = ready_tools(client, settings, tools_dir).await.map_err(|e| format!("couldn't get yt-dlp: {e}"))?;
    let tools_ready = started.elapsed();
    let mut cmd = command(&yt_dlp, settings, js.as_deref());
    let jar = CookieFile::write(tools_dir, cookies);
    if let Some(jar) = &jar {
        jar.add_to(&mut cmd);
    }
    cmd.arg("-J").arg("--").arg(url);
    let output = tokio::time::timeout(LIST_TIMEOUT, cmd.output()).await;
    note_timing(tools_dir, url, cookies.len(), tools_ready, started.elapsed(), &output);
    let output = output.map_err(|_| "the site took too long to answer".to_string())?.map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(last_error(&String::from_utf8_lossy(&output.stderr)));
    }
    let json: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    save_page(tools_dir, url, &output.stdout);
    Ok(choices(&json, find(tools_dir, FFMPEG).is_some()))
}

/// Notes how long reading a page took, to find what makes the quality menu slow.
/// One line per read in logs\video-timing.log, which keeps its last 200 lines.
fn note_timing(
    tools_dir: &Path,
    url: &str,
    cookies: usize,
    tools_ready: Duration,
    total: Duration,
    output: &Result<std::io::Result<std::process::Output>, tokio::time::error::Elapsed>,
) {
    let Some(dir) = tools_dir.parent().map(|d| d.join("logs")) else { return };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("video-timing.log");
    let result = match output {
        Ok(Ok(out)) if out.status.success() => "ok".to_string(),
        Ok(Ok(out)) => format!("failed: {}", last_error(&String::from_utf8_lossy(&out.stderr))),
        Ok(Err(e)) => format!("failed: {e}"),
        Err(_) => "timed out".to_string(),
    };
    let line = format!(
        "{} tools {:.1}s, total {:.1}s, {cookies} cookies, {result}, {url}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        tools_ready.as_secs_f32(),
        total.as_secs_f32(),
    );
    log::info!("video page read: {line}");
    let mut lines: Vec<String> = std::fs::read_to_string(&path).unwrap_or_default().lines().map(str::to_string).collect();
    lines.push(line);
    let keep = lines.len().saturating_sub(200);
    let _ = std::fs::write(&path, lines[keep..].join("\n") + "\n");
}

/// What yt-dlp read from a page stays usable this long: the download then starts
/// from it instead of reading the page again. Sites' links last hours; this is well
/// inside that.
const PAGE_FRESH: Duration = Duration::from_secs(30 * 60);

fn saved_page_path(tools_dir: &Path, url: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    tools_dir.join("pages").join(format!("{:016x}.json", hasher.finish()))
}

/// Keeps what yt-dlp read from `url`, and drops pages read long ago.
fn save_page(tools_dir: &Path, url: &str, json: &[u8]) {
    let path = saved_page_path(tools_dir, url);
    let Some(dir) = path.parent() else { return };
    let _ = std::fs::create_dir_all(dir);
    for old in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if age(&old.path()).is_some_and(|a| a > PAGE_FRESH * 4) {
            let _ = std::fs::remove_file(old.path());
        }
    }
    let _ = std::fs::write(path, json);
}

/// The page as yt-dlp read it moments ago, if it's still fresh.
fn saved_page(tools_dir: &Path, url: &str) -> Option<PathBuf> {
    let path = saved_page_path(tools_dir, url);
    age(&path).is_some_and(|a| a < PAGE_FRESH).then_some(path)
}

/// A format's size, or an estimate from its bitrate (kbit/s) and the duration.
fn size_of(format: &Value, duration: f64) -> Option<u64> {
    format["filesize"]
        .as_u64()
        .or_else(|| format["filesize_approx"].as_u64())
        .or_else(|| format["tbr"].as_f64().filter(|_| duration > 0.).map(|tbr| (tbr * 125. * duration) as u64))
}

/// Direct downloads before playlists of pieces (m3u8), as yt-dlp itself prefers.
fn is_direct(format: &Value) -> bool {
    format["protocol"].as_str().is_some_and(|p| p.starts_with("http"))
}

/// Whether a format carries a picture. Some sites don't say which codecs they use;
/// yt-dlp then leaves them empty, and a format with a height is a video.
fn has_picture(format: &Value) -> bool {
    match format["vcodec"].as_str() {
        Some(codec) => codec != "none",
        None => format["height"].is_u64() || format["width"].is_u64(),
    }
}

/// Whether a format carries sound; unknown counts as yes, as for yt-dlp.
fn has_sound(format: &Value) -> bool {
    match format["acodec"].as_str() {
        Some(codec) => codec != "none",
        None => true,
    }
}

/// Skips storyboards and other formats that are neither picture nor sound.
fn is_media(format: &Value) -> bool {
    format["ext"].as_str() != Some("mhtml") && (has_picture(format) || format["acodec"].as_str().is_some_and(|c| c != "none"))
}

fn is_mp4(format: &Value) -> bool {
    format["ext"].as_str() == Some("mp4")
}

/// Turns yt-dlp's format list into a short menu: one entry per height, best first,
/// and one for the sound alone.
///
/// Where a site sends picture and sound apart, Tachyon joins MP4 picture with M4A
/// sound itself (selector "picture,sound"); other pairs need the system's ffmpeg
/// ("picture+sound"); otherwise only files that already have sound are offered.
fn choices(json: &Value, has_ffmpeg: bool) -> MediaInfo {
    let title = json["title"].as_str().unwrap_or("video").to_string();
    let duration = json["duration"].as_f64().unwrap_or(0.);
    let size_of = |f: &Value| size_of(f, duration);
    let empty = Vec::new();
    let formats = json["formats"].as_array().unwrap_or(&empty);
    let rank = |f: &&Value| {
        (
            f["vcodec"].as_str().is_some_and(|c| c.starts_with("avc1")),
            is_direct(f),
            f["tbr"].as_f64().unwrap_or(0.) as u64,
        )
    };
    let sound_only = |f: &&Value| is_media(f) && has_sound(f) && !has_picture(f);
    let picture_only = |f: &&Value| is_media(f) && has_picture(f) && !has_sound(f);
    let m4a = formats
        .iter()
        .filter(sound_only)
        .filter(|f| f["ext"].as_str() == Some("m4a"))
        .max_by_key(|f| (is_direct(f), f["tbr"].as_f64().unwrap_or(0.) as u64));
    let any_sound = formats.iter().filter(sound_only).max_by_key(|f| f["tbr"].as_f64().unwrap_or(0.) as u64);

    let mut heights: Vec<u64> =
        formats.iter().filter(|f| is_media(f) && has_picture(f)).filter_map(|f| f["height"].as_u64()).filter(|h| *h >= 144).collect();
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();

    let mut list = Vec::new();
    for height in heights {
        let at = |f: &&Value| f["height"].as_u64() == Some(height);
        let label = |f: &Value| {
            let fps = f["fps"].as_f64().unwrap_or(0.);
            if fps > 30. { format!("{height}p{}", fps.round() as u64) } else { format!("{height}p") }
        };
        let own_merge = m4a.and_then(|sound| {
            formats.iter().filter(picture_only).filter(at).filter(|f| is_mp4(f)).max_by_key(rank).map(|p| (p, sound))
        });
        let ffmpeg_merge = any_sound
            .filter(|_| has_ffmpeg)
            .and_then(|sound| formats.iter().filter(picture_only).filter(at).max_by_key(rank).map(|p| (p, sound)));
        let whole = formats
            .iter()
            .filter(|f| is_media(f) && has_picture(f) && has_sound(f))
            .filter(at)
            .max_by_key(|f| (is_mp4(f), rank(f)));
        let choice = if let Some((picture, sound)) = own_merge {
            MediaChoice {
                label: label(picture),
                format: format!(
                    "bv*[height={height}][ext=mp4][vcodec^=avc1][protocol^=http]/bv*[height={height}][ext=mp4][protocol^=http]/\
                     bv*[height={height}][ext=mp4],ba[ext=m4a][protocol^=http]/ba[ext=m4a]"
                ),
                ext: "mp4".into(),
                size: size_of(picture).map(|v| v + size_of(sound).unwrap_or(0)),
            }
        } else if let Some((picture, sound)) = ffmpeg_merge {
            MediaChoice {
                label: label(picture),
                format: format!("bv*[height={height}]+ba/b[height={height}]"),
                ext: "mp4".into(),
                size: size_of(picture).map(|v| v + size_of(sound).unwrap_or(0)),
            }
        } else if let Some(whole) = whole {
            MediaChoice {
                label: label(whole),
                format: whole["format_id"].as_str().unwrap_or("b").to_string(),
                ext: whole["ext"].as_str().unwrap_or("mp4").to_string(),
                size: size_of(whole),
            }
        } else {
            continue;
        };
        list.push(choice);
    }
    if let Some(sound) = m4a.or(any_sound) {
        list.push(MediaChoice {
            label: "Audio only".into(),
            format: "ba[ext=m4a]/ba".into(),
            ext: sound["ext"].as_str().unwrap_or("m4a").to_string(),
            size: size_of(sound),
        });
    }
    // Whatever the list says, never an empty menu: yt-dlp's own pick.
    if list.iter().all(|c| c.label == "Audio only") && formats.iter().any(is_media) {
        list.insert(0, MediaChoice { label: "Best available".into(), format: "b[ext=mp4]/b".into(), ext: "mp4".into(), size: None });
    }
    MediaInfo { title, choices: list }
}

/// Progress across the separate picture and sound downloads.
#[derive(Default)]
struct Progress {
    /// Bytes of the streams already finished.
    done: u64,
    /// The stream being downloaded now.
    current: u64,
    current_total: u64,
}

impl Progress {
    /// Reads one of our progress lines: "TACHYON downloaded total estimate speed".
    fn update(&mut self, line: &str) -> Option<u64> {
        let parts: Vec<&str> = line.strip_prefix(PROGRESS_TAG)?.split_whitespace().collect();
        // yt-dlp writes "NA" for what it doesn't know.
        let number = |i: usize| parts.get(i).and_then(|s| s.parse::<f64>().ok()).map(|v| v as u64);
        let downloaded = number(0)?;
        let total = number(1).or(number(2)).unwrap_or(0);
        let speed = number(3).unwrap_or(0);
        if downloaded < self.current {
            // The next stream started.
            self.done += self.current_total.max(self.current);
        }
        self.current = downloaded;
        self.current_total = total;
        Some(speed)
    }

    fn downloaded(&self) -> u64 {
        self.done + self.current
    }

    fn total(&self) -> u64 {
        self.done + self.current_total
    }
}

/// The picture (.mp4) and sound (.m4a) among downloaded parts, if both exist.
fn pick_parts(parts: &[PathBuf]) -> Option<(PathBuf, PathBuf)> {
    let with_ext = |ext: &str| {
        parts.iter().find(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))).cloned()
    };
    Some((with_ext("mp4")?, with_ext("m4a")?))
}

/// yt-dlp's finished parts for `stem` in `dir`: "<stem>.f<format>.<ext>".
fn parts_in(dir: &Path, stem: &str) -> Vec<PathBuf> {
    let prefix = format!("{stem}.f");
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&prefix)))
        .filter(|p| !p.extension().is_some_and(|e| e == "part" || e == "ytdl"))
        .collect()
}

/// The file a "[download] …" line names, if it names one.
fn downloaded_file(line: &str) -> Option<PathBuf> {
    let rest = line.strip_prefix("[download] ")?;
    if let Some(path) = rest.strip_prefix("Destination: ") {
        return Some(PathBuf::from(path.trim()));
    }
    rest.strip_suffix(" has already been downloaded").map(|p| PathBuf::from(p.trim()))
}

/// The picked quality first, then ever more forgiving ways to get a video of about
/// that quality, so a site's quirk or a vanished format doesn't end the download:
/// ffmpeg joining instead of Tachyon, any MP4 picture up to that height, any picture
/// at all, and finally whatever single file the site offers.
fn fallbacks(format: &str, has_ffmpeg: bool) -> Vec<String> {
    let mut list = vec![format.to_string()];
    if format.starts_with("ba") {
        list.push("ba/b".into());
        return list;
    }
    if has_ffmpeg && let Some((picture, sound)) = format.split_once(',') {
        list.push(format!("({picture})+({sound})"));
    }
    let height = format
        .split("[height=")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .and_then(|h| h.parse::<u32>().ok());
    match height {
        Some(h) => {
            list.push(format!("bv*[height<={h}][ext=mp4],ba[ext=m4a]"));
            if has_ffmpeg {
                list.push(format!("bv*[height<={h}]+ba"));
            }
            list.push(format!("b[height<={h}][ext=mp4]/b[height<={h}]/b"));
        }
        None => list.push("b[ext=mp4]/b".into()),
    }
    list.dedup();
    list
}

/// Updates Tachyon's own yt-dlp now; true if it ran. Sites change often, and a
/// fresh yt-dlp is the usual cure for a download that stopped working.
async fn update_now(yt_dlp: &Path, settings: &Settings, tools_dir: &Path) -> bool {
    if !yt_dlp.starts_with(tools_dir) {
        // Someone else's copy (on the PATH): theirs to update.
        return false;
    }
    let _guard = install_lock().lock().await;
    let mut cmd = command(yt_dlp, settings, None);
    cmd.arg("-U");
    match tokio::time::timeout(Duration::from_secs(90), cmd.output()).await {
        Ok(Ok(out)) => {
            log::info!("yt-dlp update: {}", String::from_utf8_lossy(&out.stdout).trim());
            out.status.success()
        }
        _ => false,
    }
}

/// Downloads a video page with yt-dlp, trying [`fallbacks`] in turn. Stopping kills
/// yt-dlp; starting again resumes from its partial files. If every way fails, what
/// yt-dlp said is kept in a report next to Tachyon's data.
pub(crate) async fn run(input: TaskInput, shared: Arc<TaskShared>, env: TaskEnv) -> TaskOutcome {
    let Some(media) = input.spec.media.clone() else {
        return TaskOutcome::Failed(EngineError::Config("not a video page".into()));
    };
    // Normally already there from reading the qualities.
    let (yt_dlp, js) = tokio::select! {
        _ = shared.cancel.cancelled() => return TaskOutcome::Paused,
        ready = ready_tools(&env.client, &env.settings, &env.tools_dir) => match ready {
            Ok(tools) => tools,
            Err(e) => return TaskOutcome::Failed(e),
        },
    };
    if let Err(e) = std::fs::create_dir_all(&input.directory) {
        return TaskOutcome::Failed(EngineError::Io(e.to_string()));
    }
    let ladder = fallbacks(&media.format, find(&env.tools_dir, FFMPEG).is_some());
    let mut failures: Vec<(String, String)> = Vec::new();
    let mut updated = false;
    let mut step = 0;
    while step < ladder.len() {
        let format = &ladder[step];
        // The first try starts from the page as read for the quality menu; any retry
        // reads it afresh, in case that went stale.
        let saved = if failures.is_empty() { saved_page(&env.tools_dir, &input.spec.url) } else { None };
        match attempt(&input, &shared, &env, &yt_dlp, js.as_deref(), &media, format, saved.as_deref()).await {
            Ok(outcome) => {
                if !failures.is_empty() {
                    log::info!("video {} downloaded on try {} ({format})", input.id, failures.len() + 1);
                }
                return outcome;
            }
            Err(error) => {
                log::warn!("video {} with {format}: {}", input.id, last_error(&error));
                failures.push((format.clone(), error));
                if shared.cancel.is_cancelled() {
                    return TaskOutcome::Paused;
                }
                // Once per download: a fresh yt-dlp, then the same way again.
                if !updated {
                    updated = true;
                    if update_now(&yt_dlp, &env.settings, &env.tools_dir).await {
                        continue;
                    }
                }
                step += 1;
            }
        }
    }
    let first = failures.first().map(|(_, e)| last_error(e)).unwrap_or_default();
    let report = write_report(&env.tools_dir, &input, &failures);
    let detail = report.map(|r| format!(" — details in {}", r.display())).unwrap_or_default();
    TaskOutcome::Failed(EngineError::Protocol(format!("{first}{detail}")))
}

/// Keeps what yt-dlp said on each try, for when a video can't be downloaded at all.
fn write_report(tools_dir: &Path, input: &TaskInput, failures: &[(String, String)]) -> Option<PathBuf> {
    let dir = tools_dir.parent()?.join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("video-{}.log", input.id));
    let mut text = format!("{}\n{}\n\n", input.spec.url, chrono::Local::now().to_rfc2822());
    for (format, error) in failures {
        text.push_str(&format!("--- {format}\n{}\n", error.trim()));
    }
    std::fs::write(&path, text).ok()?;
    Some(path)
}

/// One try at a video page in `format`. Ok with the outcome (finished or stopped),
/// or Err with what went wrong.
#[allow(clippy::too_many_arguments)]
async fn attempt(
    input: &TaskInput,
    shared: &TaskShared,
    env: &TaskEnv,
    yt_dlp: &Path,
    js: Option<&Path>,
    media: &crate::types::MediaSpec,
    format: &str,
    saved_page: Option<&Path>,
) -> Result<TaskOutcome, String> {
    let target = input.directory.join(&input.file_name);
    // "picture,sound": yt-dlp fetches both and Tachyon joins them.
    let own_merge = format.contains(',');
    let work_dir = input.part_dir.clone().unwrap_or_else(|| input.directory.clone());
    let stem = target.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "video".into());
    let mut cmd = command(yt_dlp, &env.settings, js);
    let jar = CookieFile::write(&env.tools_dir, &media.cookies);
    if let Some(jar) = &jar {
        jar.add_to(&mut cmd);
    }
    cmd.arg("-f").arg(format);
    if format.contains('+') {
        cmd.arg("--merge-output-format").arg("mp4");
        if let Some(ffmpeg) = find(&env.tools_dir, FFMPEG) {
            cmd.arg("--ffmpeg-location").arg(ffmpeg);
        }
    }
    if !own_merge && let Some(part_dir) = &input.part_dir {
        cmd.arg("-P").arg(format!("temp:{}", part_dir.display()));
    }
    let limit = env.settings.global_speed_limit;
    if limit > 0 {
        cmd.arg("--limit-rate").arg(limit.to_string());
    }
    // Networks drop and sites stall: yt-dlp retries before giving up.
    cmd.args(["--retries", "10", "--fragment-retries", "10", "--extractor-retries", "3"]);
    cmd.args(["--newline", "--continue", "--progress-template"]).arg(format!(
        "download:{PROGRESS_TAG} %(progress.downloaded_bytes)s %(progress.total_bytes)s \
         %(progress.total_bytes_estimate)s %(progress.speed)s"
    ));
    cmd.arg("-o");
    if own_merge {
        cmd.arg(work_dir.join(format!("{stem}.f%(format_id)s.%(ext)s")));
    } else {
        cmd.arg(&target);
    }
    match saved_page {
        Some(page) => cmd.arg("--load-info-json").arg(page),
        None => cmd.arg("--").arg(&input.spec.url),
    };

    let mut child = cmd.spawn().map_err(|e| format!("could not run yt-dlp: {e}"))?;
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    let errors = tokio::spawn(async move {
        let mut text = Vec::new();
        let _ = BufReader::new(stderr).read_to_end(&mut text).await;
        String::from_utf8_lossy(&text).into_owned()
    });

    shared.set_media_progress(0, media.size);
    let mut progress = Progress::default();
    let mut parts: Vec<PathBuf> = Vec::new();
    let mut reader = BufReader::new(stdout);
    let mut raw = Vec::new();
    loop {
        raw.clear();
        tokio::select! {
            _ = shared.cancel.cancelled() => {
                let _ = child.kill().await;
                return Ok(TaskOutcome::Paused);
            }
            // Byte lines, so a stray non-UTF-8 name can't end the reading.
            read = reader.read_until(b'\n', &mut raw) => match read {
                Ok(n) if n > 0 => {
                    let line = String::from_utf8_lossy(&raw);
                    let line = line.trim_end();
                    if let Some(speed) = progress.update(line) {
                        shared.set_phase_downloading();
                        shared.set_speed(speed);
                        // Before the sound starts, the known size beats yt-dlp's guess.
                        let total = media.size.unwrap_or(0).max(progress.total());
                        shared.set_media_progress(progress.downloaded(), (total > 0).then_some(total));
                    } else if let Some(file) = downloaded_file(line) {
                        if !parts.contains(&file) {
                            parts.push(file);
                        }
                    } else if line.starts_with("[Merger]") || line.starts_with("[FixupM") {
                        shared.set_speed(0);
                        shared.set_phase(DownloadStatus::Finalizing);
                    }
                }
                _ => break,
            },
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    let stderr = errors.await.unwrap_or_default();
    if !status.success() {
        return Err(if stderr.trim().is_empty() { format!("yt-dlp stopped ({status})") } else { stderr });
    }
    if own_merge {
        // Named by yt-dlp's output, or failing that found by name in the folder.
        let (picture, sound) = pick_parts(&parts)
            .or_else(|| pick_parts(&parts_in(&work_dir, &stem)))
            .ok_or_else(|| format!("expected picture and sound, got {} files", parts.len()))?;
        shared.set_speed(0);
        shared.set_phase(DownloadStatus::Finalizing);
        let out = target.clone();
        let (p, s) = (picture.clone(), sound.clone());
        match tokio::task::spawn_blocking(move || crate::mux::merge(&p, &s, &out)).await {
            Ok(Ok(())) => {
                let _ = std::fs::remove_file(&picture);
                let _ = std::fs::remove_file(&sound);
            }
            Ok(Err(e)) => return Err(format!("joining picture and sound failed: {e}")),
            Err(e) => return Err(e.to_string()),
        }
    }
    let meta = std::fs::metadata(&target).map_err(|e| format!("the finished video is missing: {e}"))?;
    Ok(TaskOutcome::Completed { file_name: input.file_name.clone(), size: meta.len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_without_codecs_are_whole_videos() {
        // archive.org names no codecs.
        let json = serde_json::json!({
            "title": "Popeye",
            "formats": [
                {"format_id": "0", "ext": "mp4", "height": 240, "width": 320, "filesize": 2000},
                {"format_id": "1", "ext": "ogv", "height": 304, "width": 400, "filesize": 1900}
            ]
        });
        let menu = choices(&json, false);
        let labels: Vec<&str> = menu.choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["304p", "240p"]);
        assert_eq!(menu.choices[1].format, "0");
        // Nothing usable by height: still something to pick.
        let bare = serde_json::json!({"title": "x", "formats": [{"format_id": "a", "ext": "mp4", "vcodec": "h264"}]});
        assert_eq!(choices(&bare, false).choices[0].label, "Best available");
    }

    #[test]
    fn fallbacks_get_ever_more_forgiving() {
        let picked = "bv*[height=720][ext=mp4][vcodec^=avc1]/bv*[height=720][ext=mp4],ba[ext=m4a]";
        assert_eq!(
            fallbacks(picked, false),
            [picked, "bv*[height<=720][ext=mp4],ba[ext=m4a]", "b[height<=720][ext=mp4]/b[height<=720]/b"]
        );
        let with_ffmpeg = fallbacks(picked, true);
        assert_eq!(with_ffmpeg[1], "(bv*[height=720][ext=mp4][vcodec^=avc1]/bv*[height=720][ext=mp4])+(ba[ext=m4a])");
        assert!(with_ffmpeg.contains(&"bv*[height<=720]+ba".to_string()));
        assert_eq!(fallbacks("ba[ext=m4a]/ba", true), ["ba[ext=m4a]/ba", "ba/b"]);
        assert_eq!(fallbacks("18", false), ["18", "b[ext=mp4]/b"]);
    }

    #[test]
    fn menu_has_one_entry_per_height_and_the_sound() {
        let json = serde_json::json!({
            "title": "Clip",
            "formats": [
                {"format_id": "140", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2", "tbr": 129.0, "filesize": 1000},
                {"format_id": "251", "ext": "webm", "vcodec": "none", "acodec": "opus", "tbr": 140.0, "filesize": 900},
                {"format_id": "137", "ext": "mp4", "vcodec": "avc1.640028", "acodec": "none", "height": 1080, "fps": 30, "tbr": 4000.0, "filesize": 50000},
                {"format_id": "248", "ext": "webm", "vcodec": "vp9", "acodec": "none", "height": 1080, "fps": 30, "tbr": 3000.0, "filesize": 40000},
                {"format_id": "18", "ext": "mp4", "vcodec": "avc1.42001E", "acodec": "mp4a.40.2", "height": 360, "fps": 30, "tbr": 500.0, "filesize": 8000},
                {"format_id": "sb0", "ext": "mhtml", "vcodec": "none", "acodec": "none", "height": 90}
            ]
        });
        // MP4 picture and M4A sound: Tachyon joins them, no ffmpeg needed.
        let menu = choices(&json, false);
        let labels: Vec<&str> = menu.choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["1080p", "360p", "Audio only"]);
        assert!(menu.choices[0].format.contains(','), "{}", menu.choices[0].format);
        assert_eq!(menu.choices[0].size, Some(51000), "H.264 picture plus m4a sound");
        assert_eq!(menu.choices[1].format, "18", "already has sound");
        assert_eq!(menu.choices[2].ext, "m4a");

        // Without M4A sound only ffmpeg could join; without it, files with sound only.
        let mut no_m4a = json.clone();
        no_m4a["formats"].as_array_mut().unwrap().remove(0);
        let labels: Vec<String> = choices(&no_m4a, false).choices.into_iter().map(|c| c.label).collect();
        assert_eq!(labels, ["360p", "Audio only"]);
        let with_ffmpeg = choices(&no_m4a, true);
        assert!(with_ffmpeg.choices[0].format.contains('+'));
    }

    #[test]
    fn progress_adds_up_picture_and_sound() {
        let mut p = Progress::default();
        p.update("TACHYON 500 1000 NA 100");
        p.update("TACHYON 1000 1000 NA 100");
        p.update("TACHYON 10 200 NA 50");
        assert_eq!(p.downloaded(), 1010);
        assert_eq!(p.total(), 1200);
        assert_eq!(p.update("[download] something"), None);
    }
}
