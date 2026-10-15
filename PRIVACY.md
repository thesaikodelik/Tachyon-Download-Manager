# Privacy policy

*Tachyon Download Manager and its browser extension. Last updated 15 October 2026.*

**Tachyon collects no personal data and sends nothing to its makers.** There are no accounts,
no analytics, no tracking and no ads.

## The browser extension

The extension exists to hand downloads from your browser to the Tachyon app on the same computer.
Everything it reads stays on your computer:

- **Links and downloads** you hand to Tachyon (by clicking, through its menus, or by downloads of
  the file types you chose) are sent to the Tachyon app at `127.0.0.1`, which only your own
  computer can reach.
- **Cookies** of the site a download comes from are sent with it to the Tachyon app, so files
  that need you to be signed in download as they would in the browser. They go nowhere else.
- **Network requests** of the open tab are looked at only to find video and audio files on the
  page, for the video button. Nothing is recorded beyond the current tab, and that list is
  forgotten when the tab changes or closes.
- **Settings** (the port, whether to take over downloads, where the video button is hidden) are
  stored in the browser's own extension storage.
- **Native messaging** is used only to start the Tachyon app when it isn't running.

The extension makes no connections of its own to the internet.

## The Tachyon app

- Your download list and settings are stored in `%APPDATA%\Tachyon` on your computer. Passwords
  for sites and proxies are encrypted with Windows' own data protection for your account.
- Files are downloaded from the addresses you give it, directly from those servers.
- To keep itself current, Tachyon asks GitHub once a day for the latest release of Tachyon
  (you can turn this off in *Options → General*), and fetches the video tools yt-dlp and QuickJS
  from their GitHub releases when they are first needed and to update them.
- To download videos, yt-dlp contacts the video's site, as your browser would.

These requests carry no information about you beyond what any web request does, such as your IP
address, which the servers involved (GitHub, the sites you download from) may log under their own
policies.

## Contact

Questions: open an issue at
[github.com/thesaikodelik/Tachyon-Download-Manager](https://github.com/thesaikodelik/Tachyon-Download-Manager/issues).
