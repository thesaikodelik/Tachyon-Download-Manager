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

// Popup API.
chrome.runtime.onMessage.addListener((message, _sender, reply) => {
  if (message?.type === "status") {
    fetchConfig().then((config) => reply({ connected: Boolean(config), capture: Boolean(config?.capture) }));
    return true;
  }
  return false;
});
