const DEFAULTS = {
  port: 7311,
  takeOver: true,
};

const $ = (id) => document.getElementById(id);

async function load() {
  const s = { ...DEFAULTS, ...(await chrome.storage.local.get(Object.keys(DEFAULTS))) };
  $("takeOver").checked = s.takeOver;
  $("port").value = s.port;
  refreshStatus();
}

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
