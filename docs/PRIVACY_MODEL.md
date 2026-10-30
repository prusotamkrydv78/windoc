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
| Focus timer state | Not stored | **Planned:** `~/.winisland/focus.json` snapshot of timer kind, phase and elapsed time |

No clipboard, notes, screen-time, location or credential data is collected by the current code.

## What the app observes

| Source | Data | Default | Leaves the device |
|---|---|---|---|
| Windows media sessions (SMTC) | Track title, artist, album art, playback state and source app ID | On (`smtc_enabled`), limited to allowed apps | Only if online lyrics are enabled (title, artist, duration) |
| Audio loopback (WASAPI) | Audio samples from the playing app, converted to a spectrum in memory | Active only while music plays and the island is visible | Never; samples are not stored |
| Windows notifications (`UserNotificationListener`) | Notification content shown in the island | Off (`notification_display` is false by default) and requires Windows permission | Never |
| Foreground and fullscreen state | Whether a fullscreen app is active | On, for auto-hide | Never |
| System metrics | CPU, memory, disk and network rates for widgets | Only while those widgets are shown | Never |

## Network access

| Destination | Purpose | State in this build |
|---|---|---|
| GitHub (upstream WinIsland releases) | Update checks and installer download | **Disabled** by `UPDATE_ENDPOINTS_ENABLED = false` in `src/utils/updater.rs`. Update checks are also off by default in config. |
| GitHub (upstream PluginMarketplace) | Plugin catalog, icons and packages | **Disabled** by `MARKETPLACE_ENABLED = false` in `crates/winisland-plugin-package/src/marketplace.rs` |
| LRCLIB, AMLL, NetEase, QQ Music, Kugou | Lyrics lookup (sends title, artist and duration) | **Opt-in.** `lyrics_mode` defaults to `lrc` (local files only). Online lookup happens only when the user selects online mode. |
| Native plugins | Anything | Plugins run as native code with the user's privileges and are not network-restricted |

No telemetry, analytics or crash reports are sent.

## Known gaps

- **Audio visualisation is not opt-in.** The design guide requires an explicit opt-in. Today capture starts automatically while music plays and the island is visible. Add a setting and default it to off.
- **Plugins are unrestricted.** A native plugin can read files and use the network. Capability prompts and a network policy are planned (workflow §23).
- **Logs** may contain media titles and app IDs. Add redaction before stable release.
- **Paths are split** between `~/.winisland` and `%APPDATA%\WinIsland`. Both move to `%LOCALAPPDATA%/<Product>` at rebrand, with a one-time migration (workflow Phase 3).
- **No data controls yet.** Export, clear and retention controls are planned with the privacy dashboard (workflow §22).
