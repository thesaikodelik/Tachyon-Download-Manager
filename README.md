<p align="center">
  <img src="docs/screenshots/windows/logo-banner.jpg" alt="Tachyon Download Manager" width="100%">
</p>

<p align="center">
  <a href="https://github.com/thesaikodelik/Tachyon-Download-Manager/releases"><img src="https://img.shields.io/badge/download-TachyonSetup.exe-0067c0?style=for-the-badge&logo=windows11&logoColor=white" alt="Download for Windows"></a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Windows-11-0078d4?logo=windows11&logoColor=white" alt="Windows 11">
  <img src="https://img.shields.io/badge/license-MIT-2ea44f" alt="MIT license">
  <img src="https://img.shields.io/badge/made%20with-Rust-b7410e?logo=rust&logoColor=white" alt="Made with Rust">
  <img src="https://img.shields.io/badge/size-23%20MB-6e40c9" alt="23 MB">
  <a href="https://github.com/sponsors/thesaikodelik"><img src="https://img.shields.io/badge/buy%20me%20a%20coffee-%E2%98%95-b7793e" alt="Buy me a coffee"></a>
</p>

<p align="center">
  <b>English</b> · <a href="README.fa.md">فارسی</a>
</p>

<p align="center">
  <img src="docs/screenshots/windows/banner.png" alt="Tachyon's main window on Windows 11" width="100%">
</p>

**Tachyon** is a free, open-source **download manager for Windows**, in the spirit of
**Internet Download Manager (IDM)**. It splits every download across many connections, takes
over downloads from your browser, saves **videos from YouTube and hundreds of other sites**,
and wraps it all in a Windows 11 *Liquid Glass* look. One 23 MB program, no ads, no tracking.

<table>
  <tr>
    <td align="center" width="25%">⚡<br><b>Fast</b><br><sub>up to 32 connections per file</sub></td>
    <td align="center" width="25%">🎬<br><b>Videos</b><br><sub>YouTube and more, up to 4K</sub></td>
    <td align="center" width="25%">🧩<br><b>Every browser</b><br><sub>Chrome, Edge, Firefox, Brave…</sub></td>
    <td align="center" width="25%">🔒<br><b>Private</b><br><sub>everything stays on your PC</sub></td>
  </tr>
</table>

---

## ⚡ Downloads that use your whole connection

<img src="docs/screenshots/windows/progress.png" align="right" width="46%" alt="Progress window with every connection and the live segment map">

Tachyon splits a download *while it runs*, like IDM: every free connection takes over half of
the largest part still missing, up to 32 at a time. Connections are reused, so nothing waits
to reconnect.

- **Resume anywhere**: after a pause, a crash or a reboot, only the missing bytes are fetched.
- **Kind to servers**: it ramps up gently and honours their limits.
- **Speed limiter**, a **queue** and a **scheduler** that can sleep or shut down the PC when done.
- **Categories** that sort files into their own folders, site logins and proxies.

<br clear="right">

## 🎬 Videos, one click away

<img src="docs/screenshots/windows/video-button.png" width="75%" alt="Tachyon's Download button next to a video on YouTube">

A small **Download** button appears beside videos you point at. Click it and Tachyon shows every
quality, up to 4K, with its size:

<p align="center">
  <img src="docs/screenshots/windows/add-video.png" width="70%" alt="Picking a video's quality in Tachyon">
</p>

- **YouTube and hundreds of sites**, through [yt-dlp](https://github.com/yt-dlp/yt-dlp), which
  Tachyon fetches and keeps up to date by itself.
- Picture and sound are joined into **one MP4** by Tachyon itself; no ffmpeg needed.
- If a site changes, Tachyon updates yt-dlp and tries other ways before giving up.
- Or just **drop a video page's address** on Tachyon.

## 🧩 Works with your browser

<img src="docs/screenshots/windows/links.png" align="right" width="52%" alt="Download all links of a page">

- **Takes over downloads** of the file types you choose, from the browsers you tick.
- **Download all links** of a page, or just the ones you selected, filtered by type.
- Hold **Alt** while clicking to keep a download in the browser.
- Not running? The extension **starts Tachyon** for you.
- **Drop links** on Tachyon's window.

<br clear="right">

## 🎨 Make it yours

<img src="docs/screenshots/windows/themes.png" alt="Tachyon in four themes: Liquid Glass, Smoke, Solid and Classic">

Four themes: **Liquid Glass** (front), **Liquid Glass Smoke**, **Solid** and **Classic**; light,
dark or following Windows. Animations follow Windows' own setting.

<table>
  <tr>
    <td width="50%"><img src="docs/screenshots/windows/options.png" alt="Options"></td>
    <td width="50%"><img src="docs/screenshots/windows/complete.png" alt="Download complete"><br><br><img src="docs/screenshots/windows/about.png" width="60%" alt="About"></td>
  </tr>
  <tr>
    <td align="center"><sub>Options, laid out like Windows 11 Settings</sub></td>
    <td align="center"><sub>Drag a finished file straight to any folder</sub></td>
  </tr>
</table>

## 📦 Install

<img src="docs/screenshots/windows/setup.png" align="right" width="44%" alt="Tachyon Setup">

1. Download **TachyonSetup.exe** from [Releases](https://github.com/thesaikodelik/Tachyon-Download-Manager/releases).
2. Run it. It installs for your account only, **no administrator rights needed**, and can fetch
   the video tools right away.
3. Add the [browser extension](#-browser-extension).

> New programs may get *"Windows protected your PC"* until they're widely downloaded: choose
> **More info → Run anyway**. Tachyon is open source; read every line, or build it yourself.

Uninstall from **Settings → Apps**. Your downloaded files are never touched.

<br clear="right">

## 🧩 Browser extension

| Browser | How |
| --- | --- |
| **Chrome, Brave, Opera, Vivaldi** and other Chromium browsers | Get `tachyon-extension.zip` from [Releases](https://github.com/thesaikodelik/Tachyon-Download-Manager/releases) and unzip it, or use the `extension` folder of your installation (*Options → Browser → Show Extension Folder*). Open `chrome://extensions`, turn on **Developer mode**, click **Load unpacked** and choose the folder. |
| **Microsoft Edge** | From Edge Add-ons (coming soon). Meanwhile, load the same folder at `edge://extensions`. |
| **Firefox** | From Firefox Add-ons (coming soon). |

A green dot next to the browser in *Options → Browser* means it's connected.

<details>
<summary><b>🛠 Build from source, architecture and how IDM was studied</b></summary>

### Build

Stable Rust with the MSVC toolchain and Visual Studio Build Tools (*Desktop development with C++*).

```powershell
winget install Rustlang.Rustup
cargo build --release -p tachyon          # target\release\tachyon.exe
cargo test --workspace
```

`tachyon.exe` is its own installer: copied as `TachyonSetup.exe` (or run with `--setup`) it installs
itself. State lives in `%APPDATA%\Tachyon`; `TACHYON_DATA_DIR` selects another profile.
`cargo test -p tachyon-engine --test video_sites -- --ignored` downloads from real video sites.

### Architecture

```
crates/
  engine/   the download core, independent of the interface (tokio, reqwest/rustls)
    segment.rs      dynamic segmentation
    task.rs         one download: probe → part file → connections → finish
    engine.rs       public API, queue, scheduler, persistence, events
    media.rs        video pages through yt-dlp, with fallbacks
    mux.rs          joins picture and sound into one MP4
    integration.rs  the 127.0.0.1 endpoint for the browser extension
  app/      the GPUI app, and its own installer (setup.rs)
extension/  Manifest V3 extension for Chromium browsers and Firefox
```

The browser endpoint listens on 127.0.0.1 only, requires a loopback `Host` (no DNS rebinding), an
extension or absent `Origin`, and a header web pages can't send without an approved CORS
preflight. Requests are traced to the program that sent them, so only the browsers you tick are
taken over, and only Tachyon's own extension may start it.

### How IDM was studied

IDM is closed source and its licence forbids disassembly. Tachyon is a **clean-room** work: only
IDM's public documentation and observable behaviour were used, never its code or binaries.

| IDM behaviour (source) | Tachyon |
| --- | --- |
| "Dynamic file segmentation": a free connection halves the largest remaining part ([features][f]). | `SegmentMap::split_largest` in `segment.rs` |
| Connections are reused without reconnecting ([f]). | The probe response becomes segment 1; workers loop on `next_work()`. |
| Resume after lost connections, shutdowns or power cuts ([f]). | Segment checkpoints, validator checks, the part file. |
| Main window, progress dialog with a per-segment chart, queues, scheduler, categories, browser takeover ([help][h], [scheduler][q]). | `main_window.rs`, `progress_window.rs`, `scheduler_window.rs`, `categories.rs`, `extension/` |

[f]: https://www.internetdownloadmanager.com/features2.html
[h]: https://www.internetdownloadmanager.com/support/using_idm/using_idm.html
[q]: https://www.internetdownloadmanager.com/support/idm-scheduler/idm_queues.html

</details>

## ❤️ Credits and support

Tachyon was created for macOS by **[sonatina-dream](https://github.com/sonatina-dream/Tachyon-Download-Manager)**,
including the download engine at its heart. The Windows edition is by **[thesaikodelik](https://github.com/thesaikodelik)**.

If it's useful to you, you can **[buy me a coffee](https://github.com/sponsors/thesaikodelik)** ☕.
Entirely optional; Tachyon stays free.

> **Please note:** download only what you have the right to. Many sites, YouTube among them,
> allow downloading only of your own content or content offered for download.

<sub>[MIT License](LICENSE). Tachyon is independent and not affiliated with Tonec Inc. or Internet Download Manager.</sub>
