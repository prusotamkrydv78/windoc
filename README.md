# winisland-next

`winisland-next` is a fresh development repository for a Windows dynamic-island application written in Rust.

The initial codebase is derived from the GPL-3.0-licensed WinIsland source distributed in the BuddyDesk repository. This repository starts with new Git history so the application can be redesigned incrementally while retaining the licence and attribution required by the upstream project.

## Build

Requirements:

- Rust and Cargo
- Windows SDK
- LLVM/Clang

```powershell
cargo build --release
```

## Development checks

```powershell
cargo check
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
```

## Development guides

- [`docs/PRODUCT_ENGINEERING_WORKFLOW.md`](docs/PRODUCT_ENGINEERING_WORKFLOW.md) defines the Rust-only architecture, feature order, privacy requirements and release gates.
- [`docs/UI_UX_DESIGN_GUIDE.md`](docs/UI_UX_DESIGN_GUIDE.md) defines the native Skia dashboard, component system, motion rules and accessibility requirements.

## Licence and attribution

This project is licensed under the GNU General Public License v3.0. See `LICENSE` and `UPSTREAM_NOTICE.md`.
