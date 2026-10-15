//! Tachyon's own updates: the latest release on GitHub, and its installer downloaded
//! and checked against the SHA-256 published beside it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

const LATEST: &str = "https://api.github.com/repos/thesaikodelik/Tachyon-Download-Manager/releases/latest";
const INSTALLER: &str = "TachyonSetup.exe";

/// A release newer than this copy of Tachyon.
#[derive(Clone, Debug)]
pub struct Update {
    pub version: String,
    /// The release's page, for "What's new".
    pub page: String,
    pub installer: String,
    pub size: u64,
    /// The `TachyonSetup.exe.sha256` asset, when the release has one.
    pub checksum: Option<String>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

/// "v0.10.2" → [0, 10, 2]; anything after a '-' is ignored.
fn parse_version(text: &str) -> Vec<u64> {
    text.trim()
        .trim_start_matches(['v', 'V'])
        .split('-')
        .next()
        .unwrap_or("")
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// Whether `candidate` is a later version than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (mut a, mut b) = (parse_version(candidate), parse_version(current));
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a > b
}

/// The latest release, if it is newer than `current` and has an installer.
pub(crate) async fn check(client: &Client, url: Option<&str>, current: &str) -> Result<Option<Update>, String> {
    let response = client
        .get(url.unwrap_or(LATEST))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "Tachyon-Download-Manager")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!("GitHub answered {}", response.status()));
    }
    let body = response.bytes().await.map_err(|e| e.to_string())?;
    let release: Release = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    if release.draft || release.prerelease || !is_newer(&release.tag_name, current) {
        return Ok(None);
    }
    let Some(installer) = release.assets.iter().find(|a| a.name.eq_ignore_ascii_case(INSTALLER)) else {
        return Ok(None);
    };
    let checksum = release
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(&format!("{INSTALLER}.sha256")))
        .map(|a| a.browser_download_url.clone());
    Ok(Some(Update {
        version: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
        page: release.html_url,
        installer: installer.browser_download_url.clone(),
        size: installer.size,
        checksum,
    }))
}

/// Downloads the installer into `dir`, counting bytes in `progress`, and checks it.
pub(crate) async fn download(client: &Client, update: &Update, dir: &Path, progress: Arc<AtomicU64>) -> Result<PathBuf, String> {
    tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
    let expected = match &update.checksum {
        Some(url) => {
            let text = client.get(url).send().await.and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?;
            let text = text.text().await.map_err(|e| e.to_string())?;
            Some(text.split_whitespace().next().unwrap_or("").to_ascii_lowercase())
        }
        None => None,
    };
    let response = client
        .get(&update.installer)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let path = dir.join(format!("TachyonSetup-{}.exe", update.version));
    let part = path.with_extension("part");
    let mut file = tokio::fs::File::create(&part).await.map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        progress.fetch_add(chunk.len() as u64, Ordering::Relaxed);
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    if let Some(expected) = expected
        && hasher.hex() != expected
    {
        let _ = tokio::fs::remove_file(&part).await;
        return Err("the downloaded update doesn't match its published checksum".into());
    }
    tokio::fs::rename(&part, &path).await.map_err(|e| e.to_string())?;
    Ok(path)
}

/// A small SHA-256, so checking an update needs no extra library.
struct Sha256 {
    state: [u32; 8],
    buffer: Vec<u8>,
    length: u64,
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
    0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
    0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
    0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19],
            buffer: Vec::with_capacity(64),
            length: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.length += data.len() as u64;
        if !self.buffer.is_empty() {
            let take = (64 - self.buffer.len()).min(data.len());
            self.buffer.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buffer.len() == 64 {
                let block: [u8; 64] = self.buffer[..].try_into().unwrap();
                self.compress(&block);
                self.buffer.clear();
            }
        }
        while data.len() >= 64 {
            self.compress(data[..64].try_into().unwrap());
            data = &data[64..];
        }
        self.buffer.extend_from_slice(data);
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut h = self.state;
        for i in 0..64 {
            let s1 = h[4].rotate_right(6) ^ h[4].rotate_right(11) ^ h[4].rotate_right(25);
            let ch = (h[4] & h[5]) ^ (!h[4] & h[6]);
            let t1 = h[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = h[0].rotate_right(2) ^ h[0].rotate_right(13) ^ h[0].rotate_right(22);
            let maj = (h[0] & h[1]) ^ (h[0] & h[2]) ^ (h[1] & h[2]);
            let t2 = s0.wrapping_add(maj);
            h = [t1.wrapping_add(t2), h[0], h[1], h[2], h[3].wrapping_add(t1), h[4], h[5], h[6]];
        }
        for (s, v) in self.state.iter_mut().zip(h) {
            *s = s.wrapping_add(v);
        }
    }

    fn hex(mut self) -> String {
        let bits = self.length * 8;
        let mut tail = vec![0x80u8];
        while (self.buffer.len() + tail.len()) % 64 != 56 {
            tail.push(0);
        }
        tail.extend_from_slice(&bits.to_be_bytes());
        let length = self.length;
        self.update(&tail);
        self.length = length;
        self.state.iter().map(|v| format!("{v:08x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("v0.1.1", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.99.99"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0-beta", "0.1.0"));
    }

    #[test]
    fn sha256_matches_known_values() {
        let hash = |data: &[u8]| {
            let mut h = Sha256::new();
            // Fed in uneven pieces, as a download arrives.
            for piece in data.chunks(7) {
                h.update(piece);
            }
            h.hex()
        };
        assert_eq!(hash(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hash(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let million = vec![b'a'; 1_000_000];
        assert_eq!(hash(&million), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }
}
