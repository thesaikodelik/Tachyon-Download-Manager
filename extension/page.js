// Holding Alt while clicking a link leaves that download to the browser, as in IDM.
// (Chrome and Edge download an Alt-clicked link themselves; Tachyon then stays out.)

(() => {
  if (window.__tachyonPage) return;
  window.__tachyonPage = true;

  function connected() {
    try {
      return Boolean(chrome.runtime?.id);
    } catch {
      return false;
    }
  }

  // ---- "Download all links" / "Download selected links" ----
  // The page's links (or only those in the selection), with their media sources.

  function inSelection(node) {
    const selection = window.getSelection();
    if (!selection || selection.isCollapsed) return false;
    for (let i = 0; i < selection.rangeCount; i++) {
      if (selection.getRangeAt(i).intersectsNode(node)) return true;
    }
    return false;
  }

  function collectLinks(selectedOnly) {
    const links = [];
    const add = (url, text) => {
      if (/^https?:/i.test(url || "")) links.push({ url, text: (text || "").trim().slice(0, 200) });
    };
    for (const a of document.querySelectorAll("a[href]")) {
      if (!selectedOnly || inSelection(a)) add(a.href, a.textContent);
    }
    for (const media of document.querySelectorAll("video[src], audio[src], source[src]")) {
      if (!selectedOnly || inSelection(media)) add(media.src, "");
    }
    return links;
  }

  try {
    chrome.runtime.onMessage.addListener((message, _sender, reply) => {
      if (message?.type !== "collect-links") return false;
      reply({ page: location.href, title: document.title, links: collectLinks(Boolean(message.selected)) });
      return false;
    });
  } catch {}

  document.addEventListener(
    "click",
    (e) => {
      if (!e.altKey || !connected()) return;
      const link = e.target instanceof Element ? e.target.closest("a[href]") : null;
      if (!link || !/^https?:/i.test(link.href)) return;
      try {
        chrome.runtime.sendMessage({ type: "leave-to-browser", url: link.href }).catch(() => {});
      } catch {}
    },
    { capture: true },
  );
})();
