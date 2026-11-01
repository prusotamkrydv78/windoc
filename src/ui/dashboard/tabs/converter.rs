//! Panel tab (stub). Replace the bodies below; keep the signatures, the router in
//! `tabs/mod.rs` calls them.

use std::time::Instant;

use winisland_platform::Key;
use winisland_render::{Point, Rect};

use super::TabContext;
use crate::ui::dashboard::DashboardHit;

/// Draws the tab. Return `false` to fall back to the legacy page.
pub fn draw(_ctx: &TabContext<'_>) -> bool {
    false
}

/// `true` when this tab draws its own cards instead of sitting on the shared page card.
pub fn custom_layout() -> bool {
    false
}

/// Hit test in page coordinates. Use `DashboardHit::Tab(id)` with ids private to this tab.
pub fn hit(_grid: Rect, _point: Point) -> Option<DashboardHit> {
    None
}

/// Applies a click from [`hit`]. `true` when handled.
pub fn apply(_hit: DashboardHit) -> bool {
    false
}

pub fn wants_keyboard() -> bool {
    false
}

pub fn handle_key(_key: &Key) -> bool {
    false
}

/// Every frame, visible or not.
pub fn poll(_now: Instant) {}
