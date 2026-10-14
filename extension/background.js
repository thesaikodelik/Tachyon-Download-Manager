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
// URLs we handed back to the browser after Tachyon refused or was unreachable.
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
    return await response.json();
  } catch {
    return null;
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

/** Sends one download to Tachyon; resolves to true if the app accepted it. */
async function sendToTachyon({ url, referrer, filename }) {
  if (!/^https?:/i.test(url)) return false;
  const { port } = await settings();
  const body = {
    url,
    referrer: referrer || "",
    filename: filename || "",
    cookies: await cookieHeader(url),
    userAgent: navigator.userAgent,
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
    return false;
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
  if (passThrough.delete(url)) return;
  const { takeOver } = await settings();
  if (!takeOver) return;
  const config = await fetchConfig();
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
  if (message?.type === "media-list") {
    const key = `media:${sender.tab?.id}`;
    chrome.storage.session.get(key).then((s) => reply(s[key] || []));
    return true;
  }
  if (message?.type === "download-video") {
    sendToTachyon({ url: message.url, referrer: message.referrer, filename: message.filename }).then(reply);
    return true;
  }
  if (message?.type === "status") {
    fetchConfig().then((config) => reply({ connected: Boolean(config), capture: Boolean(config?.capture) }));
    return true;
  }
  return false;
});
