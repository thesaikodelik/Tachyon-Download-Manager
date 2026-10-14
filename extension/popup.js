const DEFAULTS = {
  port: 7311,
  takeOver: true,
  videoButton: true,
  hiddenSites: [],
};

const $ = (id) => document.getElementById(id);

async function load() {
  const s = { ...DEFAULTS, ...(await chrome.storage.local.get(Object.keys(DEFAULTS))) };
  $("takeOver").checked = s.takeOver;
  $("port").value = s.port;
  $("videoButton").checked = s.videoButton;
  refreshStatus();
  refreshHidden();
}

/** The site of the active tab, if it is a web page. */
async function currentSite() {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  try {
    const url = new URL(tab?.url || "");
    return /^https?:$/.test(url.protocol) ? url.hostname : "";
  } catch {
    return "";
  }
}

// Brings back the video button where the user hid it.
async function refreshHidden() {
  const { hiddenSites = [] } = await chrome.storage.local.get("hiddenSites");
  const site = await currentSite();
  const hereHidden = site && hiddenSites.includes(site);
  $("siteRow").hidden = !hereHidden;
  $("siteText").textContent = `Hidden on ${site}`;
  const others = hiddenSites.length - (hereHidden ? 1 : 0);
  $("allRow").hidden = others === 0;
  $("allText").textContent = `Hidden on ${others} other site${others === 1 ? "" : "s"}`;
}

$("showSite").addEventListener("click", async () => {
  const site = await currentSite();
  const { hiddenSites = [] } = await chrome.storage.local.get("hiddenSites");
  await chrome.storage.local.set({ hiddenSites: hiddenSites.filter((s) => s !== site) });
  refreshHidden();
});
$("showAll").addEventListener("click", async () => {
  await chrome.storage.local.set({ hiddenSites: [] });
  refreshHidden();
});
$("videoButton").addEventListener("change", (e) => chrome.storage.local.set({ videoButton: e.target.checked }));

function refreshStatus() {
  chrome.runtime.sendMessage({ type: "status" }, (reply) => {
    const status = $("status");
    let text = "● Not running";
    let color = "var(--bad)";
    if (reply?.connected && reply.capture) {
      text = "● Connected";
      color = "var(--ok)";
    } else if (reply?.connected) {
      // Tachyon runs, but this browser is unticked in its Options.
      text = "● Off for this browser";
      color = "var(--muted)";
    }
    status.textContent = text;
    status.style.color = color;
  });
}

$("takeOver").addEventListener("change", (e) => chrome.storage.local.set({ takeOver: e.target.checked }));
$("port").addEventListener("change", (e) => {
  const port = Number(e.target.value);
  if (port >= 1024 && port <= 65535) {
    chrome.storage.local.set({ port }).then(refreshStatus);
  }
});

load();
