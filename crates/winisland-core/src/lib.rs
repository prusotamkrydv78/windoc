//! # WinIsland Core
//!
//! Platform-independent domain layer of [WinIsland](https://github.com/WinIslandProject/WinIsland):
//! the configuration model, the pure algorithms, and the value types shared by the rest of the
//! application.
//!
//! ## What this crate deliberately does not do
//!
//! - No platform APIs, no window-system types, and no target-OS conditionals.
//! - No rendering: Skia stays behind the renderer.
//! - No UI or window code, and no dependency on any other crate of this workspace.
//! - No decisions about file locations: callers pass paths in.
//!
//! ## Dependencies
//!
//! The standard library plus light third-party crates. Anything platform or backend specific is
//! kept out, so that this crate stays compilable and reusable on its own.
//!
//! ## Dependents
//!
//! The `WinIsland` application crate, which owns every platform-specific detail and injects the
//! platform results this crate operates on.

pub mod ai_usage;
pub mod anim;
pub mod backup;
pub mod clipboard;
pub mod config;
pub mod context;
pub mod currency;
pub mod focus;
pub mod i18n;
pub mod lyrics;
pub mod notes;
pub mod palette;
pub mod persistence;
pub mod physics;
pub mod plugin_settings;
pub mod screen_time;
pub mod shelf;
pub mod text_edit;
pub mod units;
pub mod weather;
pub mod widgets;
