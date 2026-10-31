# Privacy model

This document describes what the **current code** stores, observes and sends. It is checked against the source, not against plans. Planned behaviour is marked **Planned**. Update this file in the same change that alters any data flow.

## Where data is stored

| Location | Contents | Written when |
|---|---|---|
| `~/.winisland/config.toml` | All settings (TOML), including SMTC app lists and lyrics preferences | Settings change; config migration |
| `~/.winisland/config.invalid-*.toml` | A copy of a config file that failed to parse | Config cannot be parsed |
| `~/.winisland/logs/winisland.log` | Application log | Continuously while running |
| `~/.winisland/.crash_flag`, `.crash_plugin` | Crash markers used to disable a crashing plugin | Startup and after a crash |
| `%APPDATA%\WinIsland\plugins\` | Installed native plugins, their manifests and per-plugin key-value stores (`<plugin>/store/`) | Plugin install; plugins writing their store |
| `%LOCALAPPDATA%\WinIsland\PluginMarketplace\` | Marketplace catalog and icon cache | Only if the marketplace is enabled (it is disabled in this build) |
| `~/.winisland/focus.json` | Focus timer kind, phase, elapsed time and save time | Each timer state change |
| `~/.winisland/screen_time.json` | Per-day active time by app executable (titles only if enabled), category rules | Every 5 minutes while tracking, on settings change and on exit |
| `~/.winisland/clipboard.bin` | Clipboard history (text, links, file paths, source app), JSON encrypted with DPAPI for the current Windows user | About 10 seconds after a change while history is on, and on exit |
| `~/.winisland/weather.json` | Last forecast for the chosen place (rounded coordinates, temperatures, condition) | After each successful refresh |

Clipboard and screen-time data are collected only after the user turns those features on. No notes, location or credential data is collected by the current code.

## What the app observes

| Source | Data | Default | Leaves the device |
|---|---|---|---|
| Windows media sessions (SMTC) | Track title, artist, album art, playback state and source app ID | On (`smtc_enabled`), limited to allowed apps | Only if online lyrics are enabled (title, artist, duration) |
| Audio loopback (WASAPI) | Audio samples from the playing app, converted to a spectrum in memory | Off by default (`audio_visualizer_enabled`). When enabled, active only while music plays; analysis pauses while the island is hidden | Never; samples are not stored |
| Windows notifications (`UserNotificationListener`) | Notification content shown in the island | Off (`notification_display` is false by default) and requires Windows permission | Never |
| Foreground and fullscreen state | Whether a fullscreen app is active | On, for auto-hide | Never |
| Screen time (`EVENT_SYSTEM_FOREGROUND` hook, input idle time) | Foreground app executable and active duration per day; window titles only if `screen_time_titles` is on | Off by default (`screen_time_enabled`); stored in `screen_time.json` for 7 days | Never |
| Clipboard (`WM_CLIPBOARDUPDATE`) | Copied text (up to 1 MiB) and file paths, skipped entirely when password-manager exclusion markers are present | Off by default (`clipboard_history_enabled`); unpinned items kept 7 days, 200 items, 64 MB | Never |
| System metrics | CPU, memory, disk and network rates for widgets | Only while those widgets are shown | Never |

## Network access

| Destination | Purpose | State in this build |
|---|---|---|
| GitHub (upstream WinIsland releases) | Update checks and installer download | **Disabled** by `UPDATE_ENDPOINTS_ENABLED = false` in `src/utils/updater.rs`. Update checks are also off by default in config. |
| GitHub (upstream PluginMarketplace) | Plugin catalog, icons and packages | **Disabled** by `MARKETPLACE_ENABLED = false` in `crates/winisland-plugin-package/src/marketplace.rs` |
| LRCLIB, AMLL, NetEase, QQ Music, Kugou | Lyrics lookup (sends title, artist and duration) | **Opt-in.** `lyrics_mode` defaults to `lrc` (local files only). Online lookup happens only when the user selects online mode. |
| Open-Meteo (`geocoding-api.open-meteo.com`, `api.open-meteo.com`) | Place search (the typed city name and UI language) and forecasts (coordinates rounded to about 1 km) | **Opt-in.** `weather_enabled` defaults to off. Search runs only when the user presses Enter in the location field; forecasts refresh every 30 minutes while enabled and a place is set. |
| Native plugins | Anything | Plugins run as native code with the user's privileges and are not network-restricted |

No telemetry, analytics or crash reports are sent.

## Known gaps

- **Plugins are unrestricted.** A native plugin can read files and use the network. Capability prompts and a network policy are planned (workflow §23).
- **Logs** may contain media titles and app IDs. Add redaction before stable release.
- **Paths are split** between `~/.winisland` and `%APPDATA%\WinIsland`. Both move to `%LOCALAPPDATA%/<Product>` at rebrand, with a one-time migration (workflow Phase 3).
- **Data controls are partial.** Settings, General, "Data on this PC" shows the size of screen time, clipboard, weather and Focus data and clears each immediately. Export, backups and a full privacy dashboard with network history are still planned (workflow §22).
