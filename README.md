# Notchy

Notchy is a living notch for Windows, written in Rust: a notch with a little robot friend at the top of the screen, a full panel of tools behind it, and an optional taskbar of its own. (Internally the code and crates still use the `winisland` names.)

The initial codebase is derived from the GPL-3.0-licensed WinIsland source distributed in the BuddyDesk repository. This repository starts with new Git history so the application can be redesigned incrementally while retaining the licence and attribution required by the upstream project.

## Repository layout

This one repository holds both halves of Notchy:

| Folder | What it is |
|---|---|
| `src/`, `crates/`, `resources/`, `build.rs`, `Cargo.toml` | **PC app** (Rust workspace): the notch, dashboard, Notchy Link, Memory and AI |
| `crates/notchy-unlock/` | "Unlock with phone" Windows credential provider (DLL) |
| `packaging/` | Installers and the unlock install/uninstall scripts |
| `android/` | **Phone app** (Kotlin, Gradle): phone notch robot, Link service, notifications, unlock |
| `docs/` | Product, engineering and UI guides |

## Build the PC app

Requirements:

- Rust and Cargo
- Windows SDK
- LLVM/Clang

```powershell
cargo build --release
```

## Build the phone app

```powershell
cd android
.\Setup-AndroidSdk.ps1   # first time only
.\Build-Android.ps1      # APK goes to dist/
```

Install the APK on the phone, open the notch on the PC → Phone page (or Settings → Phone → Show pairing QR) and scan the code from the app. Both devices must be on the same Wi-Fi.

## Development checks

```powershell
cargo check
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
```

## Safe mode

If a plugin or module stops the app from starting, run it with `--safe-mode`. No plugins are loaded and audio capture, screen time, clipboard history and weather stay off for that session. Saved settings are not changed.

## Development guides

- [`docs/PRODUCT_ENGINEERING_WORKFLOW.md`](docs/PRODUCT_ENGINEERING_WORKFLOW.md) defines the Rust-only architecture, feature order, privacy requirements and release gates.
- [`docs/UI_UX_DESIGN_GUIDE.md`](docs/UI_UX_DESIGN_GUIDE.md) defines the native Skia dashboard, component system, motion rules and accessibility requirements.

## Licence and attribution

This project is licensed under the GNU General Public License v3.0. See `LICENSE` and `UPSTREAM_NOTICE.md`.
