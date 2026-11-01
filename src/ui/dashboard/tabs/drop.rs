//! Drop tab: send files between phone and PC. Not built yet; the legacy page draws.

use std::time::Instant;

use winisland_platform::Key;
use winisland_render::{Point, Rect};

use super::TabContext;
use crate::ui::dashboard::DashboardHit;

pub fn draw(_ctx: &TabContext<'_>) -> bool {
    false
}

pub fn custom_layout() -> bool {
    false
}

pub fn hit(_grid: Rect, _point: Point) -> Option<DashboardHit> {
    None
}

pub fn apply(_hit: DashboardHit) -> bool {
    false
}

pub fn wants_keyboard() -> bool {
    false
}

pub fn handle_key(_key: &Key) -> bool {
    false
}

pub fn poll(_now: Instant) {}
