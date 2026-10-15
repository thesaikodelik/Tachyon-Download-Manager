// Hands links and browser downloads over to the Tachyon app via its local endpoint
// (127.0.0.1 only). The app shows its "Download File Info" dialog before anything
// is downloaded, unless the user opted out in Tachyon's Options.
//
// Which file types and sites are taken over, and whether this browser is taken over
// at all, is set in Tachyon (Options > Browser) and fetched from it.

const DEFAULTS = {
  port: 7311,
  takeOver: true,
};

const MENU_ID = "tachyon-download";
const CHECK_IN = "tachyon-check-in";
// URLs we handed back to the browser after Tachyon refused or was unreachable, or
// that the user Alt-clicked to keep in the browser.
const passThrough = new Set();

async function settings() {
  return { ...DEFAULTS, ...(await chrome.storage.local.get(Object.keys(DEFAULTS))) };
}

function endpoint(port, path) {
  return `http://127.0.0.1:${port}${path}`;
}

/** Tachyon's capture settings for this browser, or null when it isn't running. */
async function fetchConfig() {
  const { port } = await settings();
  try {
    const response = await fetch(endpoint(port, "/v1/config"), {
      headers: { "X-Tachyon-Client": "extension" },
      signal: AbortSignal.timeout(1500),
    });
    if (!response.ok) return null;
    const config = await response.json();
    // Remembered, to know which downloads are worth starting Tachyon for.
    chrome.storage.local.set({ lastConfig: config });
    return config;
  } catch {
    return null;
  }
}

// ---- Starting Tachyon when it isn't running ----
// Tachyon registers itself with the browser as a "native messaging" host that only
// this extension may call; calling it starts Tachyon in the notification area.

const NATIVE_HOST = "com.thesaikodelik.tachyon";
const START_WAIT_MS = 12000;
let starting = null;

/** Resolves to true once Tachyon answers, starting it first if needed. */
function ensureRunning() {
  starting ||= (async () => {
    if (await fetchConfig()) return true;
    try {
      await chrome.runtime.sendNativeMessage(NATIVE_HOST, { launch: true });
    } catch {
      // Not registered yet: Tachyon has never run on this computer.
      return false;
    }
    const until = Date.now() + START_WAIT_MS;
    while (Date.now() < until) {
      await new Promise((r) => setTimeout(r, 300));
      if (await fetchConfig()) return true;
    }
    return false;
  })().finally(() => {
    starting = null;
  });
  return starting;
}

/** The browser's cookies for a video page's site and its subdomains, so Tachyon
 *  reaches videos that need the user signed in, as the browser does. */
async function siteCookies(pageUrl) {
  try {
    const parts = new URL(pageUrl).hostname.split(".");
    // "youtube.com" from "www.youtube.com"; "bbc.co.uk" from "www.bbc.co.uk".
    const secondLevel = parts.length > 2 && parts.at(-1).length === 2 && parts.at(-2).length <= 3;
    const site = parts.slice(secondLevel ? -3 : -2).join(".");
    const cookies = await chrome.cookies.getAll({ domain: site });
    return cookies.map((c) => ({
      domain: c.domain,
      path: c.path,
      secure: c.secure,
      hostOnly: c.hostOnly,
      expirationDate: c.expirationDate ?? null,
      name: c.name,
      value: c.value,
    }));
  } catch {
    return [];
  }
}

async function cookieHeader(url) {
  try {
    const cookies = await chrome.cookies.getAll({ url });
    return cookies.map((c) => `${c.name}=${c.value}`).join("; ");
  } catch {
    return "";
  }
}

/** Sends one download to Tachyon, starting it if needed; true if it was accepted. */
async function sendToTachyon(download) {
  const sent = await postDownload(download);
  if (sent !== null) return sent;
  return (await ensureRunning()) && (await postDownload(download)) === true;
}

/** true or false for Tachyon's answer; null when it isn't running. */
async function postDownload({ url, referrer, filename, mediaFormat, size }) {
  if (!/^https?:/i.test(url)) return false;
  const { port } = await settings();
  const body = {
    url,
    referrer: referrer || "",
    filename: filename || "",
    cookies: await cookieHeader(url),
    userAgent: navigator.userAgent,
    mediaFormat: mediaFormat || "",
    size: size || null,
    cookieJar: mediaFormat ? await siteCookies(url) : [],
  };
  try {
    const response = await fetch(endpoint(port, "/v1/download"), {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Tachyon-Client": "extension" },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(3000),
    });
    return response.ok;
  } catch {
    return null;
  }
}

/** Asks Tachyon (through yt-dlp) which qualities a video page offers, starting it
 *  if needed. */
async function mediaFormats(url) {
  const result = await askFormats(url);
  if (!result.offline || !(await ensureRunning())) return result;
  return askFormats(url);
}

async function askFormats(url) {
  const { port } = await settings();
  try {
    const response = await fetch(endpoint(port, "/v1/media/formats"), {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Tachyon-Client": "extension" },
      body: JSON.stringify({ url, cookieJar: await siteCookies(url) }),
      signal: AbortSignal.timeout(55000),
    });
    if (response.status === 503) return { missing: true };
    const body = await response.json().catch(() => ({}));
    return response.ok ? body : { error: body.error || `HTTP ${response.status}` };
  } catch {
    return { offline: true };
  }
}

/** Matches Tachyon's site patterns: `*.example.com` means its subdomains. */
function siteMatches(pattern, host) {
  pattern = pattern.trim().toLowerCase();
  if (pattern.startsWith("*.")) {
    const base = pattern.slice(2);
    return host === base || host.endsWith("." + base);
  }
  return host === pattern;
}

function isExcluded(url, sites) {
  try {
    const host = new URL(url).hostname.toLowerCase();
    return (sites || []).some((p) => siteMatches(p, host));
  } catch {
    return false;
  }
}

// Tells Tachyon this browser's extension is installed, so Options can show it as
// connected; Tachyon adds a browser it hasn't met to its list.
function checkIn() {
  fetchConfig();
}

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: MENU_ID,
    title: "Download with Tachyon",
    contexts: ["link", "image", "video", "audio"],
  });
  chrome.alarms.create(CHECK_IN, { periodInMinutes: 5 });
  checkIn();
});
chrome.runtime.onStartup.addListener(checkIn);
chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === CHECK_IN) checkIn();
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId !== MENU_ID) return;
  const url = info.linkUrl || info.srcUrl;
  if (!url) return;
  const ok = await sendToTachyon({ url, referrer: info.pageUrl || tab?.url });
  if (!ok) {
    // Tachyon isn't running: fall back to the browser.
    passThrough.add(url);
    chrome.downloads.download({ url });
  }
});

function extensionOf(item) {
  const name = (item.filename || "").split(/[\\/]/).pop() || "";
  const fromName = name.includes(".") ? name.split(".").pop() : "";
  if (fromName) return fromName.toLowerCase();
  try {
    const path = new URL(item.finalUrl || item.url).pathname;
    const last = path.split("/").pop() || "";
    return last.includes(".") ? last.split(".").pop().toLowerCase() : "";
  } catch {
    return "";
  }
}

// Take over matching downloads, like IDM's browser integration.
chrome.downloads.onCreated.addListener(async (item) => {
  const url = item.finalUrl || item.url;
  if (!/^https?:/i.test(url) || item.byExtensionId === chrome.runtime.id) return;
  if (passThrough.delete(url) || passThrough.delete(item.url)) return;
  const { takeOver } = await settings();
  if (!takeOver) return;
  let config = await fetchConfig();
  if (!config) {
    // Tachyon isn't running: start it for the kinds of file it last took over.
    const { lastConfig } = await chrome.storage.local.get("lastConfig");
    const wanted = lastConfig?.capture && new Set(lastConfig.extensions).has(extensionOf(item));
    if (wanted && !isExcluded(url, lastConfig.excludedSites) && (await ensureRunning())) {
      config = await fetchConfig();
    }
  }
  if (!config?.capture) return;
  if (!new Set(config.extensions).has(extensionOf(item))) return;
  if (isExcluded(url, config.excludedSites)) return;

  await chrome.downloads.cancel(item.id).catch(() => {});
  await chrome.downloads.erase({ id: item.id }).catch(() => {});
  const filename = (item.filename || "").split(/[\\/]/).pop();
  const ok = await sendToTachyon({ url, referrer: item.referrer, filename });
  if (!ok) {
    passThrough.add(url);
    chrome.downloads.download({ url });
  }
});

// ---- Media files each tab played, for the video button ----
// Kept in session storage, since the service worker can be stopped at any time.

const MEDIA_EXTENSIONS = new Set(["mp4", "m4v", "webm", "mov", "mkv", "avi", "flv", "3gp", "mp3", "m4a", "ogg", "opus", "flac", "wav"]);
// Smaller complete files are clicks, chimes and ad pixels.
const MIN_MEDIA_BYTES = 512 * 1024;
const MAX_MEDIA_PER_TAB = 20;

function header(details, name) {
  return details.responseHeaders?.find((h) => h.name.toLowerCase() === name)?.value || "";
}

function mediaSize(details) {
  // "bytes 0-999/123456" for a ranged response.
  const range = header(details, "content-range").match(/\/(\d+)\s*$/);
  if (range) return Number(range[1]);
  const length = Number(header(details, "content-length"));
  return details.statusCode === 200 && length > 0 ? length : 0;
}

function isMedia(details) {
  const type = header(details, "content-type").toLowerCase();
  if (type.startsWith("video/") || type.startsWith("audio/")) return !type.includes("mpegurl");
  if (type && !type.startsWith("application/octet-stream") && !type.startsWith("binary/")) return false;
  try {
    const name = new URL(details.url).pathname.split("/").pop() || "";
    return MEDIA_EXTENSIONS.has((name.split(".").pop() || "").toLowerCase());
  } catch {
    return false;
  }
}

chrome.webRequest.onHeadersReceived.addListener(
  async (details) => {
    if (details.tabId < 0 || details.statusCode >= 300 || !isMedia(details)) return;
    const size = mediaSize(details);
    if (size && size < MIN_MEDIA_BYTES) return;
    const key = `media:${details.tabId}`;
    const list = (await chrome.storage.session.get(key))[key] || [];
    const known = list.find((m) => m.url === details.url);
    if (known) {
      known.size ||= size;
    } else {
      list.push({ url: details.url, size });
      if (list.length > MAX_MEDIA_PER_TAB) list.shift();
    }
    await chrome.storage.session.set({ [key]: list });
  },
  { urls: ["<all_urls>"], types: ["media", "xmlhttprequest", "other"] },
  ["responseHeaders"],
);

// A new page in the tab starts a new list.
chrome.tabs.onUpdated.addListener((tabId, change) => {
  if (change.url) chrome.storage.session.remove(`media:${tabId}`);
});
chrome.tabs.onRemoved.addListener((tabId) => chrome.storage.session.remove(`media:${tabId}`));

// Popup and video button API.
chrome.runtime.onMessage.addListener((message, sender, reply) => {
  if (message?.type === "leave-to-browser") {
    // An Alt-click: the browser's own download of this link stays the browser's.
    passThrough.add(message.url);
    setTimeout(() => passThrough.delete(message.url), 15000);
    reply(true);
    return false;
  }
  if (message?.type === "media-list") {
    const key = `media:${sender.tab?.id}`;
    chrome.storage.session.get(key).then((s) => reply(s[key] || []));
    return true;
  }
  if (message?.type === "download-video") {
    sendToTachyon(message).then(reply);
    return true;
  }
  if (message?.type === "media-formats") {
    // Asked ahead of a click: only if Tachyon already runs.
    (message.early ? askFormats(message.url) : mediaFormats(message.url)).then(reply);
    return true;
  }
  if (message?.type === "status") {
    fetchConfig().then((config) => reply({ connected: Boolean(config), capture: Boolean(config?.capture) }));
    return true;
  }
  return false;
});
