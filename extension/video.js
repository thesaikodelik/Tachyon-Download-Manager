// A small, quiet download button on videos. It shows only while the pointer is over
// a video, fades after two idle seconds, and widens to show the size on hover.
// Right-click it to hide it for this video (until the page reloads) or for the whole
// site; the toolbar popup brings it back.

(() => {
  if (window.__tachyonVideoButton) return;
  window.__tachyonVideoButton = true;

  const MIN_WIDTH = 200;
  const MIN_HEIGHT = 120;
  const IDLE_MS = 2000;
  const site = location.hostname;

  let enabled = true;
  let hiddenHere = false;
  // Videos the user hid "just here".
  const dismissed = new WeakSet();

  function applySettings(s) {
    enabled = s.videoButton !== false;
    hiddenHere = (s.hiddenSites || []).includes(site);
    if (!enabled || hiddenHere) hide(true);
  }
  chrome.storage.local.get(["videoButton", "hiddenSites"]).then(applySettings);
  chrome.storage.onChanged.addListener((_changes, area) => {
    if (area === "local") chrome.storage.local.get(["videoButton", "hiddenSites"]).then(applySettings);
  });

  // ---- UI, in a closed shadow root so pages can't restyle it ----
  const host = document.createElement("tachyon-video-button");
  host.style.cssText = "all: initial; position: fixed; z-index: 2147483647; top: 0; left: 0; pointer-events: none;";
  const root = host.attachShadow({ mode: "closed" });
  root.innerHTML = `
    <style>
      :host { all: initial; }
      .wrap {
        position: fixed;
        display: flex;
        flex-direction: column;
        align-items: flex-end;
        gap: 0;
        font: 12px "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
        pointer-events: none;
      }
      .btn, .menu {
        --glass: rgba(255, 255, 255, 0.72);
        --edge: rgba(255, 255, 255, 0.55);
        --text: #1b1b1b;
        --bolt: #0067c0;
        --hover: rgba(0, 0, 0, 0.06);
        background: var(--glass);
        color: var(--text);
        border: 1px solid var(--edge);
        box-shadow: 0 2px 10px rgba(0, 0, 0, 0.18);
        backdrop-filter: blur(14px) saturate(1.6);
        -webkit-backdrop-filter: blur(14px) saturate(1.6);
      }
      @media (prefers-color-scheme: dark) {
        .btn, .menu {
          --glass: rgba(32, 32, 36, 0.62);
          --edge: rgba(255, 255, 255, 0.14);
          --text: #f2f2f5;
          --bolt: #4cc2ff;
          --hover: rgba(255, 255, 255, 0.08);
        }
      }
      .btn {
        display: flex;
        align-items: center;
        height: 28px;
        min-width: 28px;
        max-width: 28px;
        box-sizing: border-box;
        padding: 0 6px;
        border-radius: 14px;
        overflow: hidden;
        white-space: nowrap;
        cursor: pointer;
        opacity: 0;
        transform: scale(0.92);
        transition: opacity 0.18s ease, transform 0.18s ease, max-width 0.22s ease;
        pointer-events: auto;
      }
      .btn.shown { opacity: 0.88; transform: none; }
      .btn.open { opacity: 1; max-width: 280px; }
      .btn svg { flex: none; width: 15px; height: 15px; }
      .label { padding: 0 4px 0 6px; }
      .menu {
        display: none;
        flex-direction: column;
        min-width: 170px;
        max-width: 320px;
        padding: 4px;
        border-radius: 8px;
        pointer-events: auto;
      }
      /* Fills the space under the button, so moving down to the list never
         counts as leaving it. */
      .holder { display: none; padding-top: 6px; pointer-events: auto; }
      .holder.shown { display: block; }
      .holder .menu { display: flex; }
      .item {
        display: flex;
        gap: 16px;
        padding: 6px 10px;
        border-radius: 4px;
        cursor: pointer;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .item:hover { background: var(--hover); }
      .item .muted { opacity: 0.6; margin-left: auto; }
      @media (prefers-reduced-motion: reduce) {
        .btn { transition: none; }
      }
    </style>
    <div class="wrap">
      <div class="btn" title="Download with Tachyon">
        <svg viewBox="0 0 24 24"><path d="M13 2 4 14h7l-1 8 9-12h-7z" fill="currentColor" stroke="currentColor"
          stroke-width="1.8" stroke-linejoin="round" style="color: var(--bolt)"/></svg>
        <span class="label"></span>
      </div>
      <div class="holder"><div class="menu"></div></div>
    </div>`;
  const wrap = root.querySelector(".wrap");
  const button = root.querySelector(".btn");
  const label = root.querySelector(".label");
  const menu = root.querySelector(".menu");
  const holder = root.querySelector(".holder");
  (document.body || document.documentElement).appendChild(host);

  let video = null;
  let idleTimer = 0;
  let overUi = false;
  let candidates = [];

  // ---- Finding the video under the pointer ----
  function videoAt(x, y) {
    for (const v of document.querySelectorAll("video")) {
      if (dismissed.has(v)) continue;
      const r = v.getBoundingClientRect();
      if (r.width < MIN_WIDTH || r.height < MIN_HEIGHT) continue;
      if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return v;
    }
    return null;
  }

  function place() {
    if (!video) return;
    const r = video.getBoundingClientRect();
    wrap.style.top = `${Math.max(r.top, 0) + 10}px`;
    wrap.style.right = `${Math.max(window.innerWidth - r.right, 0) + 10}px`;
  }

  function show(v) {
    if (video !== v) {
      video = v;
      closeMenu();
      collapse();
    }
    place();
    button.classList.add("shown");
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => {
      if (!overUi) hide();
    }, IDLE_MS);
  }

  function hide(now) {
    clearTimeout(idleTimer);
    button.classList.remove("shown");
    closeMenu();
    collapse();
    if (now) video = null;
  }

  document.addEventListener(
    "mousemove",
    (e) => {
      if (!enabled || hiddenHere || document.fullscreenElement) return;
      if (overUi) return;
      const v = videoAt(e.clientX, e.clientY);
      if (v) show(v);
      else if (video) hide();
    },
    { passive: true, capture: true },
  );
  window.addEventListener("scroll", place, { passive: true, capture: true });
  window.addEventListener("resize", place, { passive: true });

  // ---- What can be downloaded ----
  function isHttp(url) {
    return /^https?:/i.test(url || "");
  }

  async function findCandidates() {
    const list = [];
    const src = video?.currentSrc || video?.src;
    if (isHttp(src)) list.push({ url: src });
    try {
      const seen = await chrome.runtime.sendMessage({ type: "media-list" });
      for (const m of seen || []) {
        const known = list.find((c) => c.url === m.url);
        if (known) known.size = m.size;
        else list.push(m);
      }
    } catch {}
    return list;
  }

  function formatSize(bytes) {
    if (!bytes) return "";
    const units = ["B", "KB", "MB", "GB"];
    let i = 0;
    while (bytes >= 1024 && i < units.length - 1) {
      bytes /= 1024;
      i++;
    }
    return `${bytes.toFixed(i >= 2 ? 1 : 0)} ${units[i]}`;
  }

  function nameOf(url) {
    try {
      return decodeURIComponent(new URL(url).pathname.split("/").pop()) || url;
    } catch {
      return url;
    }
  }

  function fileName(url) {
    const ext = (nameOf(url).match(/\.([a-z0-9]{2,5})$/i) || [, "mp4"])[1];
    const title = (document.title || "video").replace(/[\\/:*?"<>|]+/g, " ").trim().slice(0, 120);
    return `${title || "video"}.${ext}`;
  }

  // Qualities Tachyon (through yt-dlp) found for each page, so it reads a page once.
  const qualities = new Map();

  function pageQualities() {
    const page = location.href;
    if (!qualities.has(page)) {
      const asked = chrome.runtime.sendMessage({ type: "media-formats", url: page }).catch(() => ({ offline: true }));
      qualities.set(page, asked);
      // Ask again next time unless it worked.
      asked.then((r) => {
        if (!r?.choices?.length) qualities.delete(page);
      });
    }
    return qualities.get(page);
  }

  async function expand() {
    candidates = await findCandidates();
    if (candidates.length === 1) {
      label.textContent = formatSize(candidates[0].size) || "Download";
    } else if (candidates.length > 1) {
      label.textContent = `${candidates.length} files`;
    } else {
      // Picture and sound come in pieces (YouTube and the like): Tachyon reads the page.
      label.textContent = "Finding qualities…";
      button.classList.add("open");
      const result = await pageQualities();
      if (result?.choices?.length) {
        candidates = result.choices.map((c) => ({ ...c, page: true, title: result.title }));
        const best = candidates[0];
        label.textContent =
          candidates.length === 1 ? `${best.label} · ${formatSize(best.size)}` : `${best.label} · ${candidates.length} qualities`;
      } else if (result?.missing) {
        label.textContent = "Tachyon needs yt-dlp – see Tachyon";
      } else if (result?.offline) {
        label.textContent = "Tachyon isn't running";
      } else {
        label.textContent = result?.error ? "No video found here" : "Reload the page to find it";
      }
    }
    button.classList.add("open");
  }

  function collapse() {
    button.classList.remove("open");
  }

  function closeMenu() {
    holder.classList.remove("shown");
    menu.textContent = "";
  }

  function openMenu(items) {
    menu.textContent = "";
    for (const { text, note, run } of items) {
      const item = document.createElement("div");
      item.className = "item";
      item.textContent = text;
      if (note) {
        const muted = document.createElement("span");
        muted.className = "muted";
        muted.textContent = note;
        item.appendChild(muted);
      }
      item.addEventListener("click", (e) => {
        e.stopPropagation();
        closeMenu();
        run();
      });
      menu.appendChild(item);
    }
    holder.classList.add("shown");
  }

  function pageFileName(candidate) {
    const title = (candidate.title || document.title || "video").replace(/[\\/:*?"<>|]+/g, " ").trim().slice(0, 120);
    return `${title || "video"}.${candidate.ext || "mp4"}`;
  }

  async function download(candidate) {
    const message = candidate.page
      ? {
          type: "download-video",
          url: location.href,
          filename: pageFileName(candidate),
          referrer: location.href,
          mediaFormat: candidate.format,
          size: candidate.size,
        }
      : { type: "download-video", url: candidate.url, filename: fileName(candidate.url), referrer: location.href };
    const ok = await chrome.runtime.sendMessage(message);
    label.textContent = ok ? "Sent to Tachyon" : "Tachyon isn't running";
    button.classList.add("open");
    setTimeout(() => {
      if (!overUi) hide();
    }, 1400);
  }

  // ---- Pointer and clicks on the button ----
  // A pointer that slips off for a moment doesn't close the list.
  const LEAVE_GRACE_MS = 350;
  let leaveTimer = 0;
  wrap.addEventListener("mouseenter", () => {
    overUi = true;
    clearTimeout(idleTimer);
    clearTimeout(leaveTimer);
    if (!holder.classList.contains("shown")) expand();
  });
  wrap.addEventListener("mouseleave", () => {
    clearTimeout(leaveTimer);
    leaveTimer = setTimeout(() => {
      overUi = false;
      closeMenu();
      collapse();
      idleTimer = setTimeout(hide, IDLE_MS);
    }, LEAVE_GRACE_MS);
  });

  button.addEventListener("click", (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (candidates.length === 1) {
      download(candidates[0]);
    } else if (candidates.length > 1) {
      openMenu(
        candidates.map((c) => ({
          text: c.page ? c.label : nameOf(c.url),
          note: formatSize(c.size),
          run: () => download(c),
        })),
      );
    }
  });

  button.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    e.stopPropagation();
    openMenu([
      {
        text: "Hide here",
        run: () => {
          if (video) dismissed.add(video);
          overUi = false;
          hide(true);
        },
      },
      {
        text: `Hide on ${site}`,
        run: async () => {
          const { hiddenSites = [] } = await chrome.storage.local.get("hiddenSites");
          if (!hiddenSites.includes(site)) hiddenSites.push(site);
          await chrome.storage.local.set({ hiddenSites });
          overUi = false;
        },
      },
    ]);
  });
})();
