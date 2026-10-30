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

## Licence and attribution

This project is licensed under the GNU General Public License v3.0. See `LICENSE` and `UPSTREAM_NOTICE.md`.
