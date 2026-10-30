# Product and Engineering Workflow for `winisland-next`

This guide defines how to evolve the active `winisland-next` repository into a reliable Windows productivity platform with a compact top-edge surface, a full modular workspace, local-first data handling and optional connected services. The objective is feature parity with the useful parts of Vayli while producing a more coherent architecture, safer defaults, clearer onboarding, stronger accessibility and an original product identity.

The product should feel calm and immediate: common actions must take one gesture, private information must remain understandable and controllable, and modules must never compete for attention.

## Current project baseline

The active repository is `C:\Users\Purshotam\Desktop\WinIsland-Product-Blueprint\winisland-next`. It has fresh Git history and contains only the native Rust application baseline:

- Application shell and routing under `src/`
- Domain, platform, Windows, renderer and plugin crates under `crates/`
- Skia/D3D12 and DirectComposition rendering paths
- SMTC media, WASAPI audio, Windows notification and system integrations
- Native resources and packaging scripts
- GPL-3.0 licence and an upstream snapshot notice

The BuddyDesk Python application and the React/TypeScript `Page/` website are intentionally outside this product. Do not reintroduce Python, Node.js, React, TypeScript, WebView or Tauri into the desktop runtime without an explicit architecture decision. Non-code assets such as images, fonts, language files and shaders remain acceptable.

Baseline status (2026-10-30):

- Rust stable (MSVC host) with rustfmt and clippy is installed.
- Still missing: the Visual Studio “Desktop development with C++” workload, the Windows SDK and LLVM/Clang (`LIBCLANG_PATH` for `skia-safe`).
- Roughly 20 GB of free space is required on the build drive for those tools, the Skia binaries and `target/`.
- The baseline has therefore not yet been compiled. A clean baseline build remains the first blocking engineering task.

Changes already made before the baseline build, under the safety exception in Phase 1:

- The inherited updater and plugin marketplace are disabled in code because they point at upstream WinIsland infrastructure and trust its signing key.
- Online lyrics default to off.
- All file-based BuddyDesk IPC is inert. The BuddyDesk modules are physically removed once the baseline compiles.

# Product boundaries

## Required principles

1. **Original identity.** Do not copy the Vayli name, logo, text, screenshots, illustrations, sounds, or exact layout. Use Vayli only as a functional benchmark.
2. **GPL compliance.** A distributed WinIsland fork must remain GPL-3.0 and include the complete corresponding source, build instructions, licence notices, and modification history.
3. **Local first.** Clipboard content, notes, screen-time records, files, settings, logs, and integration secrets stay on the PC unless the user performs an explicit network action.
4. **Modular by default.** Every major capability is independently enableable. Disabled modules do not poll, subscribe to system events, allocate render surfaces, or make network requests.
5. **Progressive complexity.** The first-run experience exposes only Home, Media, Clipboard, Shelf, Focus (timer), and Settings. Advanced modules are discoverable later.
6. **No silent authority.** Features that inspect window titles, clipboard contents, location, microphone state, local logs, directories, or network services explain the access before enabling it.
7. **Fast recovery.** A broken module or plugin must not make the product unusable. Safe mode, crash attribution, reset, export, and repair are first-class features.

## Product success targets

These are engineering budgets, not marketing claims:

| Measure | Target |
|---|---:|
| Idle CPU after stabilization | Below 0.3% of one modern desktop core |
| Idle working set with six core modules | Below 120 MB |
| Compact-to-open response | First visual response within 50 ms |
| Full panel interactive time | Below 150 ms |
| Background disk writes | No periodic writes when state is unchanged |
| Cold start to visible compact surface | Below 1.5 seconds on a typical SSD |
| Crash-free sessions | At least 99.5% before stable release |
| Keyboard-only completion | Every primary flow |
| Minimum supported resolution | 1280 × 720 at 100–150% scaling; 1920 × 1080 at up to 200% |
| Compact size class | Mandatory when the work area is narrower than 800 logical pixels |

# Current architecture and target boundaries

The imported `winisland-next` workspace currently contains these crates:

```text
crates/
├── winisland-core
├── winisland-render
├── winisland-platform
├── winisland-platform-windows
├── winisland-plugin-api
├── winisland-plugin-host
├── winisland-plugin-package
├── settings-schema
├── settings-schema-derive
└── pollkit
```

Preserve these ownership boundaries:

| Layer | Responsibility | Must not contain |
|---|---|---|
| `winisland-core` | Domain models, commands, module state, orchestration | Win32 calls, GPU code, HTTP clients |
| `winisland-platform` | OS-neutral traits and capability contracts | Windows implementation details or `unsafe` |
| `winisland-platform-windows` | Win32, WinRT, SMTC, WMI, Credential Manager and system events | Product layout and business rules |
| `winisland-render` | Composition tree, Skia/GPU rendering, animation, text and images | Storage, network, secrets or module business rules |
| `winisland-plugin-api` | Versioned C ABI, capabilities and stable shared types | Product-specific mutable state |
| `winisland-plugin-host` | Loading, lifecycle, capability gates, crash attribution | UI-specific feature implementations |
| `winisland-plugin-package` | Package validation, signatures, hashes and extraction | Runtime plugin logic |
| Application `src/` | Composition root, routing, startup and shutdown | Reusable platform or rendering internals |

Add new crates only when the boundary is durable:

```text
crates/
├── winisland-storage          # migrations, repositories, encryption
├── winisland-network          # HTTP policy, retries, caching, offline state
├── winisland-module-sdk       # in-tree module lifecycle and view models
├── winisland-transfer         # local-network pairing and file transfer
├── winisland-search           # indexing and command palette search
└── winisland-diagnostics      # structured local diagnostics and redaction
```

Do not create one crate per tiny feature. Group code by stable architectural responsibility.

# Development workflow

## Phase 0: finish the legal, toolchain and product baseline

- Treat `winisland-next` as a working repository name until the final brand is selected.
- Preserve `LICENSE`, upstream copyright notices and `UPSTREAM_NOTICE.md`.
- Add `THIRD_PARTY_NOTICES.md` and a machine-readable dependency licence report.
- Record which assets are original and which dependencies supply fonts, icons or sounds.
- Install the stable Rust MSVC toolchain, Visual Studio C++ Build Tools, Windows SDK and LLVM/Clang.
- Replace upstream product identifiers incrementally; do not conceal the source lineage.
- Create `docs/PRIVACY_MODEL.md`, `SECURITY.md`, and a vulnerability-reporting process.

Exit criteria:

- The fork builds from a clean checkout.
- Licence obligations are documented.
- No copied Vayli asset or text is present.

## Phase 1: compile and audit the imported baseline

Run the following sequence from the `winisland-next` repository before feature work:

```powershell
rustup toolchain install stable-x86_64-pc-windows-msvc
rustup default stable-x86_64-pc-windows-msvc
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo build --release
cargo test --workspace
cargo audit
```

Create an audit report covering:

- Startup path and single-instance handling
- Window creation, topmost behavior and focus rules
- GPU surfaces, device-loss recovery and frame scheduling
- SMTC/media subscriptions
- Audio capture and FFT lifecycle
- Settings schema and migrations
- Plugin ABI and native DLL trust boundary
- Marketplace signature and package extraction
- Network endpoints and update flow
- Every `unsafe` block and its safety invariant

Do not start product changes until the imported snapshot builds, launches, renders on the current GPU and has a written architectural audit. If an inherited feature fails, record it as a baseline defect before changing behavior.

Safety exception: changes that only remove risk may land before the baseline build. Examples are disabling network endpoints the fork does not control, or making third-party IPC inert. Each must be small, keep existing function signatures where possible, and say in its commit message that it has not been compiled.

## Phase 2: build the module framework

Every built-in module implements a shared lifecycle:

```rust
pub trait ProductModule {
    fn id(&self) -> ModuleId;
    fn metadata(&self) -> ModuleMetadata;
    fn required_capabilities(&self) -> CapabilitySet;
    fn enable(&mut self, context: &ModuleContext) -> Result<()>;
    fn suspend(&mut self);
    fn resume(&mut self);
    fn disable(&mut self);
    fn snapshot(&self) -> ModuleSnapshot;
    fn handle(&mut self, action: ModuleAction) -> Result<Vec<ModuleEffect>>;
}
```

Required framework behavior:

- Lazy initialization
- Explicit capability declaration
- Cancellation tokens for background work
- Bounded channels and backpressure
- Per-module error boundary
- Per-module storage namespace
- Per-module network allowlist
- Per-module data-clear and export operation
- Visibility-aware refresh frequency
- Battery-saver and fullscreen suspension
- Reduced-motion awareness

## Phase 3: storage and secrets

Use SQLite for structured data and a content-addressed directory for images and files.

```text
%LOCALAPPDATA%/<Product>/
├── app.db
├── blobs/
├── shelf/
├── cache/
├── logs/
├── backups/
└── plugins/
```

Rules:

- Encrypt clipboard bodies, private notes, calendar URLs and sensitive metadata with Windows DPAPI.
- Store OAuth tokens, API keys and licence material in Windows Credential Manager or DPAPI-protected storage.
- Never place secrets in ordinary settings files (TOML or JSON), logs, crash reports, backups or plugin configuration.
- Version every schema and run transactional migrations.
- Keep blobs reference-counted so deleted records do not leave private orphan files.
- Provide retention limits by age, count and total disk size.
- Encrypt backups when they contain private data; require a user-chosen passphrase for portable backups.

Migration from the inherited layout:

- The imported code stores TOML configuration, logs and plugin state under `~/.winisland/`. The marketplace cache lives under `%LOCALAPPDATA%\WinIsland`.
- On first start of the renamed product, copy recognised settings into the new layout once, then record the migration version.
- Leave the old directory untouched so a downgrade still works.
- Never import anything from `~/.buddydesk/`.

## Phase 4: render and interaction foundation

Implement these shell states before feature modules:

1. Hidden
2. Resting indicator
3. Live activity
4. Peek card
5. Expanded workspace
6. Pinned workspace
7. Fullscreen-suppressed state
8. Safe mode and recovery state

The renderer must support multiple monitors, per-monitor DPI, taskbar relocation, remote desktop, display hot-plugging, HDR, device loss and reduced motion.

Two foundations must be designed here, not retrofitted later:

- **Accessibility tree.** A Skia/D3D12 surface is opaque to assistive technology. Implement a Windows UI Automation provider (`IRawElementProviderSimple` / `IRawElementProviderFragment`) fed from the same semantic layout nodes the renderer uses. Without it, Narrator cannot reach any control.
- **Focus model.** The resting surface is a topmost, non-activating window and must never steal focus. Keyboard-opened workspaces, the command field and text input need a deliberate path: temporarily clear `WS_EX_NOACTIVATE`, or use a separate activatable window, and record the previously focused window so it can be restored on close.

# Feature implementation sequence

Each feature below includes the minimum production scope and its definition of done. The numbering is a catalogue, not a build order. The single authoritative build order is the **Suggested delivery roadmap** near the end of this document. The UI guide's functional-slice order matches it.

## 1. Shell, notch and live activities

Build the top-edge surface, keyboard shortcut, tray menu, focus behavior, monitor selection and live-activity queue.

Implementation:

- Use event-driven window updates; do not poll pointer position at animation frequency while idle.
- Never steal keyboard focus when showing passive content.
- Deduplicate live activities by source and event type.
- Apply priority, timeout and interruption rules.
- Hide or reduce activity during fullscreen games, presentations and Do Not Disturb.

Done when:

- It behaves correctly on mixed-DPI multi-monitor setups.
- Browser tabs at the top edge remain usable.
- Fullscreen suppression and manual reveal are predictable.
- Every action is keyboard accessible.

## 2. Home dashboard

Home is a configurable summary, not another application dashboard.

Implementation:

- Create small cards sourced from module snapshots.
- Let users reorder, resize and hide cards.
- Show no empty chart or placeholder when a module is disabled.
- Use freshness timestamps and offline indicators for network data.
- Use a native Rust/Skia dashboard shell with a header, two-row responsive card grid and bottom navigation dock.
- Implement the first visual prototype with deterministic sample data for Screen Time, Weather, Focus, Clipboard and Notes before connecting system services.
- Keep card geometry stable during refresh; changing values must not cause layout jumps.

Better-than-baseline addition:

- Add “Context Sets”: Work, Focus, Meeting, Gaming and Personal. Each set controls visible cards, notification rules and shortcuts.

Done when Home opens without initializing every module and remains useful with only two enabled modules.

## 3. Media and audio activity

Use Windows Global System Media Transport Controls for metadata and commands.

Features:

- Artwork, title, artist and source application
- Play, pause, previous, next and seek
- Multiple-session selector
- Compact progress and optional spectrum visualization
- Output-device and volume controls where supported

Privacy and performance:

- Audio loopback visualization is opt-in.
- Stop WASAPI capture immediately when visualization is disabled or hidden.
- Never upload track metadata unless the user enables an online lyrics provider.

Lyrics provider policy:

- Online lyrics are off by default; local LRC files are the default mode.
- LRCLIB and AMLL publish APIs intended for third-party use and may ship as opt-in providers.
- The inherited QQ Music, NetEase and Kugou providers call unofficial endpoints, which may conflict with those services' terms. Keep them out of distributed builds until their terms are reviewed, or move them to an optional plugin.

## 4. Clipboard history

Capture text, links, images and file references using Windows clipboard events.

Implementation:

- Respect exclusion formats used by password managers.
- Deduplicate consecutive identical items.
- Encrypt bodies with DPAPI.
- Generate thumbnails in a bounded worker queue.
- Provide search, pin, paste, drag-out and item preview.
- Default to a seven-day retention and a disk cap.

Unique improvement:

- Add local-only transformations: clean formatting, extract links, JSON pretty-print, case conversion and OCR through an offline engine.

Done when password-manager exclusions are tested and clipboard monitoring can be disabled without restart.

## 5. File Shelf

Provide named temporary shelves with drag-in and drag-out.

Decide per shelf whether entries are:

- References to existing files
- Managed copies
- Ephemeral files deleted after a chosen period

Show missing references clearly. Never silently delete the original. Use antivirus-friendly ordinary file operations and preserve Zone Identifier metadata where applicable.

Unique improvement: project-scoped shelves that restore automatically when a matching application or directory becomes active.

## 6. Notes and checklists

Features:

- Plain text, Markdown preview and checklists
- Pinning, tags and fast full-text search
- Automatic local save with revision history
- Export to Markdown or text
- Optional private-note encryption

Do not build a full document editor. Optimize for capturing and retrieving small information quickly.

## 7. Links and universal search

Combine saved links, search providers, commands, settings and local module results in one command palette.

Security rules:

- Validate custom search templates.
- Display the destination domain before opening generated URLs.
- Fetch site icons only when enabled.
- Never send the entire local query to multiple online providers.

Unique improvement: local action syntax such as `timer 25`, `convert 10 usd inr`, `open downloads`, `note idea`, and `clip invoice`.

## 8. Timers, focus and wellness

Features:

- Countdown, stopwatch, Pomodoro and interval sequences
- Water and stretch reminders
- Persistent timers across restart and sleep
- Quiet notifications and live compact countdown
- Focus history without productivity scoring

Use monotonic clocks for active timing and wall-clock reconciliation after sleep.

Unique improvement: Focus Scenes that activate a Context Set, mute selected activities and optionally launch a chosen workspace.

## 9. Calendar and reminders

Support local events, reminders, public holiday packs and read-only ICS/webcal feeds.

Implementation:

- Encrypt feed URLs.
- Parse feeds defensively with size and recurrence limits.
- Cache normalized events in SQLite.
- Perform timezone-safe recurrence expansion in a bounded window.
- Surface conflicts and next-event countdown without exposing titles in compact mode unless allowed.

Later, add Microsoft/Google accounts only through standard OAuth and separate consent.

## 10. Clock and timezone planner

Provide world clocks, offsets, daylight-saving awareness and a compact meeting-time planner.

Unique improvement: drag a time slider to compare all saved cities and copy a formatted availability line.

## 11. Weather

Use a provider abstraction, beginning with Open-Meteo.

Requirements:

- Manual city is the default.
- Windows Location is optional and requested only when the user selects automatic location.
- Cache forecasts and show age/offline status.
- Support metric/imperial units independently of Windows locale.
- Never store precise coordinates longer than required when a city is sufficient.

## 12. Unit and currency conversion

Perform physical conversions locally with versioned constants. Fetch currency rates through one configured provider and show the rate timestamp.

Support length, mass, temperature, volume, speed, data size, time, area, energy and currency. Never present currency conversion as a guaranteed transaction price.

## 13. File converter

Start with images:

- PNG, JPEG, WebP, BMP and ICO
- Resize, crop, quality and metadata removal
- Images to PDF and PDF image extraction where libraries permit

Process files locally in isolated workers. Validate decoders, cap dimensions and memory use, use temporary output plus atomic rename, and never overwrite without confirmation.

Unique improvement: reusable conversion recipes and drag-to-recipe targets in the compact surface.

## 14. Screen time

Track active application duration using foreground-window events, not continuous polling.

Privacy defaults:

- Store executable identity and friendly application name.
- Window-title collection is off by default.
- No keystrokes, screenshots, mouse clicks or document contents.
- Retention defaults to seven days.

Unique improvement: local rules that categorize apps without uploading the application list, plus a transparent explanation of every rule.

## 15. System monitor

Show CPU, memory, disk, network, battery, GPU and audio state.

Use adaptive sampling:

- Compact visible: 1–2 seconds
- Expanded chart visible: 250–500 ms
- Hidden: 5–10 seconds or event-driven
- Battery saver/fullscreen: slower or suspended

Do not pretend a process diagnosis is certain. Show measurements and top contributors with clear timestamps.

## 16. Custom message and presence

Let users display a short status line during screen sharing or while away.

Features:

- Presets, scheduled expiry and colors
- Optional automatic activation from Focus Scenes
- A presentation-safe mode that hides private live activities

Avoid distracting marquee animations. Motion must respect reduced-motion settings.

## 17. Local phone transfer

Build a local-network transfer system with an explicit threat model.

Protocol requirements:

- Ephemeral pairing secret encoded in the QR code
- TLS using an ephemeral local certificate, or an authenticated encrypted session protocol
- Short-lived session and single-use tokens
- Optional PIN that strengthens rather than replaces the pairing secret
- File count, size and type limits
- Visible sender, receiver and target directory
- No binding to public interfaces by default
- Automatic shutdown when the transfer surface closes
- Clear Windows Firewall behavior

Never market “local network” as automatically secure. Test hostile devices on the same Wi-Fi.

## 18. AI coding usage

Read only metadata from supported local Codex and Claude Code logs through versioned adapters.

Rules:

- Never store prompt or response bodies.
- Project paths are hidden by default and can be reduced to a local alias.
- Explain estimated cost assumptions and distinguish subscription use from API billing.
- Adapter failure must not modify source logs.

Unique improvement: agent status cards that show running, waiting, completed or failed without exposing prompt content.

## 19. Website analytics

Support Plausible and compatible privacy-focused providers first.

Implementation:

- Store API secrets in Credential Manager.
- Request the narrowest read-only access.
- Cache aggregated metrics only.
- Provide a “test connection” result without logging the token.
- Show provider, property and last refresh on every dashboard.

## 20. Revenue dashboard

Support Stripe and Gumroad through read-only restricted credentials where available.

Safety requirements:

- No refund, payout, transfer or write operation.
- Clearly separate gross, fees, refunds, net and available balance.
- Display currency and reporting timezone.
- Hide amounts automatically during screen sharing or presentation mode.
- Never include financial values in diagnostics.

## 21. Companion character

If a companion is included, make it optional and subordinate to productivity. It is new code. The inherited BuddyDesk companion, its `~/.buddydesk` file IPC and its artwork are being removed and must not be revived.

Implementation:

- State-machine scenes rather than unrestricted background behavior
- Cursor reactions sampled only while visible
- No content inspection for contextual messages
- Local memory with a clear reset control
- Suppression during meetings, fullscreen, battery saver and reduced motion

Unique direction: let the companion represent system state through calm poses rather than interrupting with frequent speech.

## 22. Backup, restore and privacy dashboard

This is a release-blocking feature, not later polish.

Provide:

- Per-module data inventory and approximate size
- Last network access by module and destination category
- Clear, export and retention controls
- Encrypted portable backup
- Restore preview with merge/replace choice
- Automatic local backup, off by default
- Complete local reset and plugin-safe mode

## 23. Plugin platform

Retain WinIsland’s versioned ABI and signed marketplace model, but improve user understanding.

The marketplace model needs this product's own infrastructure before it is re-enabled:

- A catalog host this project controls.
- A new Ed25519 catalog signing key, held offline or in a hardware token, with a documented rotation and revocation procedure.
- `CATALOG_PUBLIC_KEY` replaced in `winisland-plugin-package`. The upstream WinIsland key must never be trusted.
- `MARKETPLACE_ENABLED` switched on only after the above are in place.

- Display publisher, source URL, version and requested capabilities.
- Explain that native plugins execute with the host’s user privileges.
- Require explicit approval for high-risk capabilities before first enablement.
- Allow disabling network access through a host policy where technically enforceable.
- Preserve crash attribution and automatic disable-on-crash.
- Provide a developer SDK, templates and ABI compatibility matrix.

## 24. Differentiating capabilities

After the core is stable, prioritize features that produce a distinct product:

- Context Sets and Focus Scenes
- Universal local command palette
- Project-aware shelves and notes
- Privacy dashboard with network history
- Local automation rules using trigger/action blocks
- Plugin capability consent
- Workspace snapshots that restore selected apps, files, timer and notes
- Offline OCR and small text transformations
- Presentation-safe mode

# Security and privacy verification

Maintain a data-flow table in the repository:

| Data | Default storage | Protection | Network behavior |
|---|---|---|---|
| Clipboard bodies | SQLite/blob store | DPAPI | Never |
| Notes | SQLite | Optional DPAPI/private mode | Never |
| Screen-time app IDs | SQLite | User-profile ACL | Never |
| Window titles | Disabled | DPAPI if enabled | Never |
| Calendar feed URLs | Credential/secret store | DPAPI | Configured feed only |
| Integration API keys | Credential Manager | OS protected | Named provider only |
| Weather city | SQLite | User-profile ACL | Weather provider |
| Precise location | Memory/cache | Minimized retention | Selected providers |
| AI usage metadata | SQLite | Path aliases | Never |
| Transfer files | Temporary/selected folder | Session encryption | Local paired device |

Required security tests:

- Malformed clipboard images and file lists
- Oversized ICS recurrence rules
- Path traversal and ZIP bombs in plugin and backup imports
- Malicious local-network transfer names and content lengths
- Token redaction in every log and error path
- Database corruption and interrupted migrations
- Plugin ABI mismatch and panic recovery
- Untrusted image decoder inputs
- Symlink and reparse-point handling
- Update signature and rollback behavior

# Testing strategy

## Automated tests

- Unit tests for pure domain logic, parsing, retention and migrations
- Property tests for converters, recurrence expansion and protocol parsers
- Fuzzing for ICS, plugin manifests, ZIP extraction, image metadata and transfer frames
- Integration tests using mock platform traits and local HTTP servers
- Snapshot tests for stable render primitives, not entire animated screens
- Performance benchmarks for startup, frame time, idle CPU and database search

## Manual compatibility matrix

Test at minimum:

- Current Windows 11 (primary) and the previous Windows 11 feature update
- Windows 10 22H2 on a best-effort basis only; mainstream support ended in October 2025
- 100%, 125%, 150%, 175% and 200% scaling
- One, two and three monitors with mixed scaling
- Taskbar on supported edges and auto-hide
- Sleep/resume, lock/unlock and user switching
- Remote Desktop
- Battery saver and low-power laptop
- Fullscreen video, borderless game and exclusive fullscreen
- High contrast, reduced motion and screen reader
- English plus one right-to-left and one CJK language

# Branching and delivery

Use short-lived branches:

```text
feat/<module>-<capability>
fix/<area>-<problem>
refactor/<boundary>-<change>
security/<area>-<issue>
docs/<subject>
```

Every pull request must include:

- Problem and user impact
- Architecture boundary touched
- Data and network behavior
- UI screenshots for visual changes
- Manual verification steps
- Accessibility impact
- Performance impact
- Migration and rollback notes

Do not combine architecture refactoring, visual redesign and a new feature in one pull request.

# Release workflow

## Release channels

- Nightly: automated, unsigned development channel where appropriate
- Preview: signed builds for opt-in testers
- Stable: signed, reproducible release with migration and rollback notes

## Update delivery

The inherited updater is disabled (`UPDATE_ENDPOINTS_ENABLED` in `src/utils/updater.rs`). It points at upstream WinIsland releases and verifies only a SHA-256 digest taken from the same GitHub metadata it downloads, so it cannot detect a compromised release. Before re-enabling it:

- Publish a release manifest signed with this project's own Ed25519 key. Keep the key separate from the marketplace key.
- Verify the manifest signature, then the installer hash from the signed manifest, then the Authenticode signature of the installer.
- Refuse downgrades unless the user explicitly chooses one.
- Keep update checks off by default until the privacy dashboard can show them.

## Stable release gate

- Formatting, clippy, tests and audit pass
- Reproducible clean-machine build
- No known critical/high security issue
- Upgrade and downgrade path tested
- Data backup and restore verified
- Privacy inventory matches code
- Dependencies and licences recorded
- Installer and binaries code-signed
- Crash recovery and safe mode tested
- Release notes written for users, not only developers

# Suggested delivery roadmap

For the current Rust-only repository, use this order rather than promising dates before the baseline build:

1. Bootstrap: install Rust/Windows build dependencies and capture a clean baseline build
2. Audit: document startup, windowing, renderer, plugin, storage, network and unsafe boundaries
3. Cleanup: remove BuddyDesk modules, decide the product name, then rebrand identifiers, paths and packaging
4. Design system: semantic tokens, typography, icons, cards, controls, focus states, motion primitives, plus the UI Automation tree and focus model from Phase 4
5. Dashboard shell: header, responsive grid with compact and standard size classes, bottom dock, routing and sample-data states
6. First functional vertical slice: Focus timer with persistence, notifications and compact live activity
7. Core local modules, in this order: Notes, Clipboard, Screen Time, System Monitor, File Shelf
8. Existing strengths: media, audio visualization, opt-in lyrics and Windows live activities
9. Daily information: Weather, Calendar, Clock and Units
10. Privacy and resilience: consent, retention, export/clear, backups, safe mode and diagnostics
11. Extensions: command palette, transfer, integrations, plugins and optional companion
12. Stabilization: accessibility verification, localization, performance, security review, installer and signing

# Definition of done for every feature

A feature is complete only when:

- It works with mouse, keyboard and touch where applicable.
- It respects reduced motion, high contrast and text scaling.
- It can be disabled completely.
- Its data can be inspected, exported and cleared.
- Its network behavior is documented and observable.
- Secrets never enter logs or ordinary settings.
- Background work stops when disabled.
- Failure has a clear recovery path.
- It has automated tests for domain logic and documented manual tests for Windows integration.
- Performance stays within the product budgets.
- User-facing copy explains uncertainty and consequences plainly.

# Reference material

- Active local repository: `C:\Users\Purshotam\Desktop\WinIsland-Product-Blueprint\winisland-next`
- Imported snapshot source: https://github.com/lzsobig/BuddyDesk-pet-claude/tree/main/winisland
- Original WinIsland project: https://github.com/WinIslandProject/WinIsland
- Vayli feature benchmark: https://vayli.in/features/
- Vayli privacy behavior benchmark: https://vayli.in/privacy/
- Microsoft Windows App SDK: https://learn.microsoft.com/windows/apps/windows-app-sdk/
- Microsoft Windows security and DPAPI: https://learn.microsoft.com/windows/win32/secauthn/data-protection
