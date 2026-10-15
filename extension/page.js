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
