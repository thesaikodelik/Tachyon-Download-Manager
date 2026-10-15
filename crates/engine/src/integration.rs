//! Local endpoint the browser extension hands downloads to.
//!
//! Listens on 127.0.0.1 only. A request is accepted only if:
//! - its Host header names the loopback address, which defeats DNS rebinding;
//! - it has no Origin, or an extension Origin, so web pages are refused;
//! - it carries the `X-Tachyon-Client` header. Pages can't send that without a CORS
//!   preflight, which this server never approves.
//!
//! Captured downloads are not started silently: by default the app asks the user first.
//! Each request is traced to the program that sent it, so only the browsers the user
//! picked are taken over.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::types::{AddRequest, RequestSpec};

const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 256 * 1024;
const EXTENSION_ORIGINS: [&str; 3] = ["chrome-extension://", "moz-extension://", "safari-web-extension://"];

/// What the endpoint asks of the engine.
pub(crate) trait Host: Send + Sync {
    /// The program sending a request made contact (on every request).
    fn seen(&self, program: Option<&Path>);
    /// Takes a captured download; returns false to leave it to the browser.
    fn capture(&self, request: AddRequest, program: Option<&Path>) -> bool;
    /// What the extension in `program` should take over.
    fn config(&self, program: Option<&Path>) -> ClientConfig;
    /// Links collected from a page, for the user to pick from.
    fn links(&self, batch: crate::types::LinkBatch);
    /// The qualities a video page offers, from yt-dlp.
    fn media_formats(
        &self,
        url: String,
        cookies: Vec<crate::types::BrowserCookie>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<crate::media::MediaInfo, String>> + Send>>;
}

/// Tells an extension which downloads to hand over, so it never cancels one in the
/// browser that Tachyon would refuse.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientConfig {
    /// Downloads from this browser are taken over at all.
    pub capture: bool,
    pub extensions: Vec<String>,
    pub excluded_sites: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureBody {
    url: String,
    #[serde(default)]
    referrer: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    cookies: Option<String>,
    #[serde(default)]
    user_agent: Option<String>,
    #[serde(default)]
    description: Option<String>,
    /// For a video page: the quality picked, as a yt-dlp format selector.
    #[serde(default)]
    media_format: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    /// The site's cookies, for a video page.
    #[serde(default)]
    cookie_jar: Vec<crate::types::BrowserCookie>,
}

#[derive(Deserialize)]
struct LinksBody {
    #[serde(default)]
    page: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    links: Vec<crate::types::PageLink>,
}

/// More links than this on one page are not worth listing.
const MAX_LINKS: usize = 5000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaQuery {
    url: String,
    #[serde(default)]
    cookie_jar: Vec<crate::types::BrowserCookie>,
}

pub(crate) async fn serve(port: u16, cancel: CancellationToken, host: Arc<dyn Host>) {
    let listener = match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => listener,
        Err(e) => {
            log::error!("browser integration: cannot listen on 127.0.0.1:{port}: {e}");
            return;
        }
    };
    log::info!("browser integration listening on 127.0.0.1:{port}");
    loop {
        let stream = tokio::select! {
            _ = cancel.cancelled() => return,
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(_) => continue,
            },
        };
        let host = host.clone();
        tokio::spawn(async move {
            let (stream, peer) = stream;
            // Long enough for yt-dlp to read a video page.
            let _ = tokio::time::timeout(Duration::from_secs(60), handle(stream, peer, port, host)).await;
        });
    }
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

async fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        let n = stream.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEAD {
            return None;
        }
    };
    let head = std::str::from_utf8(&buf[..head_end]).ok()?;
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split_whitespace();
    let method = request_line.next()?.to_string();
    let path = request_line.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    if length > MAX_BODY {
        return None;
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(length);
    Some(Request {
        method,
        path,
        headers,
        body,
    })
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn is_allowed(request: &Request, port: u16) -> bool {
    let host_ok = request.header("host").is_some_and(|h| {
        h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}")
    });
    let origin_ok = match request.header("origin") {
        None => true,
        Some(origin) => EXTENSION_ORIGINS.iter().any(|p| origin.starts_with(p)),
    };
    host_ok && origin_ok && request.header("x-tachyon-client").is_some()
}

async fn handle(mut stream: TcpStream, peer: SocketAddr, port: u16, host: Arc<dyn Host>) {
    let Some(request) = read_request(&mut stream).await else {
        respond(&mut stream, "400 Bad Request", r#"{"error":"bad request"}"#).await;
        return;
    };
    if !is_allowed(&request, port) {
        respond(&mut stream, "403 Forbidden", r#"{"error":"forbidden"}"#).await;
        return;
    }
    let program: Option<PathBuf> = tokio::task::spawn_blocking(move || crate::sender::program_of(peer, port))
        .await
        .ok()
        .flatten();
    let program = program.as_deref();
    host.seen(program);
    match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/v1/links") => {
            let Ok(body) = serde_json::from_slice::<LinksBody>(&request.body) else {
                respond(&mut stream, "400 Bad Request", r#"{"error":"invalid json"}"#).await;
                return;
            };
            let mut seen = std::collections::HashSet::new();
            let links: Vec<_> = body
                .links
                .into_iter()
                .filter(|l| url::Url::parse(&l.url).is_ok_and(|u| matches!(u.scheme(), "http" | "https")))
                .filter(|l| seen.insert(l.url.clone()))
                .take(MAX_LINKS)
                .collect();
            if links.is_empty() {
                respond(&mut stream, "422 Unprocessable Entity", r#"{"error":"no links"}"#).await;
                return;
            }
            host.links(crate::types::LinkBatch { page: body.page, title: body.title, links });
            respond(&mut stream, "200 OK", r#"{"ok":true}"#).await;
        }
        ("POST", "/v1/media/formats") => {
            let Ok(query) = serde_json::from_slice::<MediaQuery>(&request.body) else {
                respond(&mut stream, "400 Bad Request", r#"{"error":"invalid json"}"#).await;
                return;
            };
            if !url::Url::parse(&query.url).is_ok_and(|u| matches!(u.scheme(), "http" | "https")) {
                respond(&mut stream, "422 Unprocessable Entity", r#"{"error":"unsupported url"}"#).await;
                return;
            }
            match host.media_formats(query.url, query.cookie_jar).await {
                Ok(info) => respond(&mut stream, "200 OK", &serde_json::to_string(&info).unwrap_or_default()).await,
                Err(e) if e == "yt-dlp missing" => {
                    respond(&mut stream, "503 Service Unavailable", r#"{"error":"yt-dlp missing"}"#).await
                }
                Err(e) => {
                    let body = serde_json::json!({ "error": e }).to_string();
                    respond(&mut stream, "422 Unprocessable Entity", &body).await
                }
            }
        }
        ("GET", "/v1/config") => {
            let body = serde_json::to_string(&host.config(program)).unwrap_or_default();
            respond(&mut stream, "200 OK", &body).await;
        }
        ("GET", "/v1/ping") => {
            let body = format!(r#"{{"app":"Tachyon","version":"{}"}}"#, env!("CARGO_PKG_VERSION"));
            respond(&mut stream, "200 OK", &body).await;
        }
        ("POST", "/v1/download") => {
            let Ok(capture) = serde_json::from_slice::<CaptureBody>(&request.body) else {
                respond(&mut stream, "400 Bad Request", r#"{"error":"invalid json"}"#).await;
                return;
            };
            let valid = url::Url::parse(&capture.url)
                .map(|u| matches!(u.scheme(), "http" | "https"))
                .unwrap_or(false);
            if !valid {
                respond(&mut stream, "422 Unprocessable Entity", r#"{"error":"unsupported url"}"#).await;
                return;
            }
            let accepted = host.capture(AddRequest {
                request: RequestSpec {
                    url: capture.url,
                    referer: capture.referrer.filter(|r| !r.is_empty()),
                    cookies: capture.cookies.filter(|c| !c.is_empty()),
                    user_agent: capture.user_agent.filter(|u| !u.is_empty()),
                    credentials: None,
                    media: capture
                        .media_format
                        .filter(|f| !f.is_empty())
                        .map(|format| crate::types::MediaSpec { format, size: capture.size, cookies: capture.cookie_jar }),
                },
                file_name: capture.filename.filter(|f| !f.is_empty()),
                description: capture.description.unwrap_or_default(),
                start: true,
                ..Default::default()
            }, program);
            if accepted {
                respond(&mut stream, "200 OK", r#"{"ok":true}"#).await;
            } else {
                // The extension then lets the browser download it.
                respond(&mut stream, "409 Conflict", r#"{"error":"not taken over"}"#).await;
            }
        }
        _ => respond(&mut stream, "404 Not Found", r#"{"error":"not found"}"#).await,
    }
}
