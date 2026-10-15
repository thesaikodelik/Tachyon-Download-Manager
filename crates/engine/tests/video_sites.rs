//! Downloads a short video from several real sites the way the browser extension
//! does: asks for the qualities, picks the smallest, downloads it and checks the
//! file. Needs the network, so it only runs when asked:
//!
//!     cargo test -p tachyon-engine --test video_sites -- --ignored --nocapture
//!
//! TACHYON_TEST_TOOLS may name a folder with yt-dlp and QuickJS to copy instead of
//! fetching them.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use common::open_engine;
use tachyon_engine::DownloadStatus;

const PAGES: [&str; 4] = [
    "https://www.youtube.com/watch?v=jNQXAC9IVRw",
    "https://www.youtube.com/shorts/tPEE9ZwTmy0",
    "https://www.dailymotion.com/video/x7tgad0",
    "https://archive.org/details/Popeye_forPresident",
];

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn post(port: u16, path: &str, body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(120))).unwrap();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Tachyon-Client: test\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status = response.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let body = response.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    (status, body)
}

#[test]
#[ignore = "downloads from the internet"]
fn videos_download_from_real_sites() {
    let port = free_port();
    let dir = tempfile::tempdir().unwrap();
    let engine = open_engine(dir.path(), |s| {
        s.browser_integration = true;
        s.integration_port = port;
        s.start_captured_immediately = true;
    });
    if let Some(tools) = std::env::var_os("TACHYON_TEST_TOOLS") {
        let target = engine.data_dir().join("tools");
        std::fs::create_dir_all(&target).unwrap();
        for entry in std::fs::read_dir(tools).unwrap().flatten() {
            std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
        }
    }
    for _ in 0..50 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let mut failed = Vec::new();
    for page in PAGES {
        let (status, body) = post(port, "/v1/media/formats", &serde_json::json!({ "url": page }).to_string());
        if status != 200 {
            failed.push(format!("{page}: qualities {status} {body}"));
            continue;
        }
        let info: serde_json::Value = serde_json::from_str(&body).unwrap();
        let choices = info["choices"].as_array().unwrap();
        // The smallest video, to keep the test quick.
        let Some(choice) = choices.iter().filter(|c| c["label"] != "Audio only").last() else {
            failed.push(format!("{page}: no video qualities in {body}"));
            continue;
        };
        let name = format!("{}.{}", info["title"].as_str().unwrap_or("video"), choice["ext"].as_str().unwrap_or("mp4"));
        let download = serde_json::json!({
            "url": page,
            "filename": name,
            "mediaFormat": choice["format"],
            "size": choice["size"],
        });
        let (status, body) = post(port, "/v1/download", &download.to_string());
        assert_eq!(status, 200, "{page}: {body}");

        let started = Instant::now();
        let finished = loop {
            let snapshot = engine.snapshot().into_iter().find(|d| d.url == page).expect("added");
            if matches!(snapshot.status, DownloadStatus::Completed | DownloadStatus::Failed) {
                break snapshot;
            }
            assert!(started.elapsed() < Duration::from_secs(300), "{page}: still {:?}", snapshot.status);
            std::thread::sleep(Duration::from_millis(500));
        };
        let path = finished.path();
        let head = std::fs::read(&path).map(|b| b.get(4..8).map(<[u8]>::to_vec).unwrap_or_default()).unwrap_or_default();
        println!(
            "{page}: {:?} {} ({} bytes) in {:.0?} {}",
            finished.status,
            finished.file_name,
            finished.downloaded,
            started.elapsed(),
            finished.error.clone().unwrap_or_default()
        );
        if finished.status != DownloadStatus::Completed || (choice["ext"] == "mp4" && head != b"ftyp") {
            failed.push(format!("{page}: {:?} {}", finished.status, finished.error.unwrap_or_default()));
        }
    }
    assert!(failed.is_empty(), "failed:\n{}", failed.join("\n"));

    // A quality that has vanished (as when a site changes between listing and
    // downloading): the fallbacks still deliver a video.
    let page = "https://www.youtube.com/watch?v=jNQXAC9IVRw&vanished";
    let download = serde_json::json!({
        "url": page,
        "filename": "vanished quality.mp4",
        "mediaFormat": "bv*[height=4321][ext=mp4],ba[ext=m4a]",
    });
    assert_eq!(post(port, "/v1/download", &download.to_string()).0, 200);
    let started = Instant::now();
    let finished = loop {
        let snapshot = engine.snapshot().into_iter().find(|d| d.url == page).expect("added");
        if matches!(snapshot.status, DownloadStatus::Completed | DownloadStatus::Failed) {
            break snapshot;
        }
        assert!(started.elapsed() < Duration::from_secs(300));
        std::thread::sleep(Duration::from_millis(500));
    };
    println!(
        "vanished quality: {:?} ({} bytes) in {:.0?} {}",
        finished.status,
        finished.downloaded,
        started.elapsed(),
        finished.error.clone().unwrap_or_default()
    );
    assert_eq!(finished.status, DownloadStatus::Completed);
}
