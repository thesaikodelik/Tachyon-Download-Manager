//! Video pages (YouTube and the many sites yt-dlp knows) are downloaded through
//! yt-dlp, which fetches picture and sound and merges them with ffmpeg. Tachyon
//! keeps its own copy of yt-dlp in its data folder and uses ffmpeg from the system.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::error::EngineError;
use crate::task::{TaskEnv, TaskInput, TaskOutcome, TaskShared};
use crate::types::{DownloadStatus, ProxyMode, Settings};

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
    /// Without ffmpeg only qualities that come as one file are offered.
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
    MediaTools { yt_dlp: find(tools_dir, YT_DLP), ffmpeg: find(tools_dir, FFMPEG) }
}

/// Downloads the latest yt-dlp into the tools folder.
pub(crate) async fn install_yt_dlp(client: &Client, tools_dir: &Path) -> Result<PathBuf, EngineError> {
    tokio::fs::create_dir_all(tools_dir).await.map_err(|e| EngineError::Io(e.to_string()))?;
    let response = client.get(YT_DLP_URL).send().await.map_err(|e| EngineError::Network(e.to_string()))?;
    if !response.status().is_success() {
        return Err(EngineError::Http {
            status: response.status().as_u16(),
            reason: response.status().canonical_reason().unwrap_or("").to_string(),
            retry_after: None,
        });
    }
    let bytes = response.bytes().await.map_err(|e| EngineError::Network(e.to_string()))?;
    let target = tools_dir.join(YT_DLP);
    let temp = tools_dir.join(format!("{YT_DLP}.download"));
    tokio::fs::write(&temp, &bytes).await.map_err(|e| EngineError::Io(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755));
    }
    tokio::fs::rename(&temp, &target).await.map_err(|e| EngineError::Io(e.to_string()))?;
    Ok(target)
}

/// A yt-dlp command with the options every call shares.
fn command(yt_dlp: &Path, settings: &Settings) -> Command {
    let mut cmd = Command::new(yt_dlp);
    cmd.args(["--no-playlist", "--no-colors", "--no-warnings"]);
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

/// Asks yt-dlp what `url` offers.
pub(crate) async fn list(url: &str, settings: &Settings, tools: &MediaTools) -> Result<MediaInfo, String> {
    let yt_dlp = tools.yt_dlp.as_deref().ok_or("yt-dlp missing")?;
    let mut cmd = command(yt_dlp, settings);
    cmd.arg("-J").arg("--").arg(url);
    let output = tokio::time::timeout(LIST_TIMEOUT, cmd.output())
        .await
        .map_err(|_| "the site took too long to answer".to_string())?
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(last_error(&String::from_utf8_lossy(&output.stderr)));
    }
    let json: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    Ok(choices(&json, tools.ffmpeg.is_some()))
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

fn has(format: &Value, codec: &str) -> bool {
    format[codec].as_str().is_some_and(|c| c != "none")
}

/// Turns yt-dlp's format list into a short menu: one entry per height, best first,
/// and one for the sound alone.
fn choices(json: &Value, can_merge: bool) -> MediaInfo {
    let title = json["title"].as_str().unwrap_or("video").to_string();
    let duration = json["duration"].as_f64().unwrap_or(0.);
    let size_of = |f: &Value| size_of(f, duration);
    let empty = Vec::new();
    let formats = json["formats"].as_array().unwrap_or(&empty);
    let audio = formats
        .iter()
        .filter(|f| has(f, "acodec") && !has(f, "vcodec"))
        .max_by_key(|f| (f["ext"].as_str() == Some("m4a"), is_direct(f), f["tbr"].as_f64().unwrap_or(0.) as u64));
    let mut heights: Vec<u64> = formats
        .iter()
        .filter(|f| has(f, "vcodec") && (can_merge || has(f, "acodec")))
        .filter_map(|f| f["height"].as_u64())
        .filter(|h| *h >= 144)
        .collect();
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();

    let mut list = Vec::new();
    for height in heights {
        let at_height = formats
            .iter()
            .filter(|f| has(f, "vcodec") && f["height"].as_u64() == Some(height))
            .filter(|f| can_merge || has(f, "acodec"));
        // As the selector below: H.264 first, as it plays everywhere.
        let Some(best) = at_height.max_by_key(|f| {
            (
                f["vcodec"].as_str().is_some_and(|c| c.starts_with("avc1")),
                is_direct(f),
                f["tbr"].as_f64().unwrap_or(0.) as u64,
            )
        }) else {
            continue;
        };
        let fps = best["fps"].as_f64().unwrap_or(0.);
        let label = if fps > 30. { format!("{height}p{}", fps.round() as u64) } else { format!("{height}p") };
        let choice = if can_merge && !has(best, "acodec") {
            MediaChoice {
                label,
                format: format!(
                    "bv*[height={height}][vcodec^=avc1]+ba[ext=m4a]/bv*[height={height}]+ba/b[height={height}]"
                ),
                ext: "mp4".into(),
                size: size_of(best).map(|v| v + audio.and_then(size_of).unwrap_or(0)),
            }
        } else {
            MediaChoice {
                label,
                format: best["format_id"].as_str().unwrap_or("b").to_string(),
                ext: best["ext"].as_str().unwrap_or("mp4").to_string(),
                size: size_of(best),
            }
        };
        list.push(choice);
    }
    if let Some(audio) = audio {
        list.push(MediaChoice {
            label: "Audio only".into(),
            format: "ba[ext=m4a]/ba".into(),
            ext: audio["ext"].as_str().unwrap_or("m4a").to_string(),
            size: size_of(audio),
        });
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

/// Downloads a video page with yt-dlp. Stopping kills yt-dlp; starting again resumes
/// from its partial files.
pub(crate) async fn run(input: TaskInput, shared: Arc<TaskShared>, env: TaskEnv) -> TaskOutcome {
    let Some(media) = input.spec.media.clone() else {
        return TaskOutcome::Failed(EngineError::Config("not a video page".into()));
    };
    let tools = tools(&env.tools_dir);
    let Some(yt_dlp) = tools.yt_dlp.clone() else {
        return TaskOutcome::Failed(EngineError::Config("yt-dlp is needed to download videos from this site".into()));
    };
    if let Err(e) = std::fs::create_dir_all(&input.directory) {
        return TaskOutcome::Failed(EngineError::Io(e.to_string()));
    }
    let target = input.directory.join(&input.file_name);
    let mut cmd = command(&yt_dlp, &env.settings);
    cmd.arg("-f").arg(&media.format);
    if media.format.contains('+') {
        cmd.arg("--merge-output-format").arg("mp4");
    }
    if let Some(ffmpeg) = &tools.ffmpeg {
        cmd.arg("--ffmpeg-location").arg(ffmpeg);
    }
    if let Some(part_dir) = &input.part_dir {
        cmd.arg("-P").arg(format!("temp:{}", part_dir.display()));
    }
    let limit = env.settings.global_speed_limit;
    if limit > 0 {
        cmd.arg("--limit-rate").arg(limit.to_string());
    }
    cmd.args(["--newline", "--continue", "--progress-template"])
        .arg(format!(
            "download:{PROGRESS_TAG} %(progress.downloaded_bytes)s %(progress.total_bytes)s \
             %(progress.total_bytes_estimate)s %(progress.speed)s"
        ))
        .arg("-o")
        .arg(&target)
        .arg("--")
        .arg(&input.spec.url);

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return TaskOutcome::Failed(EngineError::Config(format!("could not run yt-dlp: {e}"))),
    };
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    let errors = tokio::spawn(async move {
        let mut text = String::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            text.push_str(&line);
            text.push('\n');
        }
        text
    });

    shared.set_media_progress(0, media.size);
    let mut progress = Progress::default();
    let mut lines = BufReader::new(stdout).lines();
    loop {
        tokio::select! {
            _ = shared.cancel.cancelled() => {
                let _ = child.kill().await;
                return TaskOutcome::Paused;
            }
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    if let Some(speed) = progress.update(&line) {
                        shared.set_phase_downloading();
                        shared.set_speed(speed);
                        // Before the sound starts, the known size beats yt-dlp's guess.
                        let total = media.size.unwrap_or(0).max(progress.total());
                        shared.set_media_progress(progress.downloaded(), (total > 0).then_some(total));
                    } else if line.starts_with("[Merger]") || line.starts_with("[FixupM") {
                        shared.set_speed(0);
                        shared.set_phase(DownloadStatus::Finalizing);
                    }
                }
                _ => break,
            },
        }
    }
    let status = child.wait().await;
    let stderr = errors.await.unwrap_or_default();
    match status {
        Ok(status) if status.success() => match std::fs::metadata(&target) {
            Ok(meta) => TaskOutcome::Completed { file_name: input.file_name, size: meta.len() },
            Err(e) => TaskOutcome::Failed(EngineError::Io(format!("the finished video is missing: {e}"))),
        },
        Ok(_) => TaskOutcome::Failed(EngineError::Protocol(last_error(&stderr))),
        Err(e) => TaskOutcome::Failed(EngineError::Io(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let merged = choices(&json, true);
        let labels: Vec<&str> = merged.choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["1080p", "360p", "Audio only"]);
        assert_eq!(merged.choices[0].size, Some(51000), "H.264 picture plus m4a sound");
        assert_eq!(merged.choices[2].ext, "m4a");

        // Without ffmpeg only files that already have sound.
        let single = choices(&json, false);
        let labels: Vec<&str> = single.choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["360p", "Audio only"]);
        assert_eq!(single.choices[0].format, "18");
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
