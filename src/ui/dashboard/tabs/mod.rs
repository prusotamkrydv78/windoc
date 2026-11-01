//! One file per panel tab. Each tab draws its own page, answers its own hit tests and
//! clicks, and can take the keyboard. A tab returns `false` from `draw` to fall back to the
//! legacy page in `dashboard/mod.rs`.
//!
//! Tabs never edit each other's files: shared helpers live in `dashboard/mod.rs` (reachable
//! as `super::super::…`), and app data the dashboard already receives (weather rows,
//! clipboard items, agenda rows, screen time…) is read from the parent module's state.

use std::time::Instant;

use winisland_platform::Key;
use winisland_render::tokens::SemanticColors;
use winisland_render::{Painter, Point, Rect};

use super::{DashboardHit, Destination};

pub mod ai_coding;
pub mod calendar;
pub mod clipboard;
pub mod clock;
pub mod converter;
pub mod drop;
pub mod links;
pub mod message;
pub mod notes;
pub mod screen_time;
pub mod shelf;
pub mod timers;
pub mod units;
pub mod weather;

/// Everything a tab needs to draw. Coordinates are page coordinates (already scaled).
pub struct TabContext<'a> {
    pub painter: Painter<'a>,
    /// The content area below the header and above the dock.
    pub grid: Rect,
    pub hover: Option<DashboardHit>,
    pub colors: &'a SemanticColors,
    pub now: Instant,
}

macro_rules! route {
    ($destination:expr, $call:ident ( $($arg:expr),* ), $default:expr) => {
        match $destination {
            Destination::Clock => clock::$call($($arg),*),
            Destination::Focus => timers::$call($($arg),*),
            Destination::Agenda => calendar::$call($($arg),*),
            Destination::Weather => weather::$call($($arg),*),
            Destination::Clipboard => clipboard::$call($($arg),*),
            Destination::Notes => notes::$call($($arg),*),
            Destination::Shelf => shelf::$call($($arg),*),
            Destination::Converter => converter::$call($($arg),*),
            Destination::Units => units::$call($($arg),*),
            Destination::Links => links::$call($($arg),*),
            Destination::Message => message::$call($($arg),*),
            Destination::ScreenTime => screen_time::$call($($arg),*),
            Destination::AiUsage => ai_coding::$call($($arg),*),
            Destination::Drop => drop::$call($($arg),*),
            _ => $default,
        }
    };
}

/// Draws `destination`'s page. `false` means the tab is not built yet: draw the legacy page.
pub fn draw(destination: Destination, ctx: &TabContext<'_>) -> bool {
    route!(destination, draw(ctx), false)
}

/// Whether the tab lays out its own cards (no shared page card behind it).
pub fn custom_layout(destination: Destination) -> bool {
    route!(destination, custom_layout(), false)
}

pub fn hit(destination: Destination, grid: Rect, point: Point) -> Option<DashboardHit> {
    route!(destination, hit(grid, point), None)
}

/// Applies a `DashboardHit::Tab(id)` (or any hit the tab claims). `true` when handled.
pub fn apply(destination: Destination, hit: DashboardHit) -> bool {
    route!(destination, apply(hit), false)
}

/// Whether the visible tab is editing text and wants keys.
pub fn wants_keyboard(destination: Destination) -> bool {
    route!(destination, wants_keyboard(), false)
}

/// A key for the visible tab. `true` when used.
pub fn handle_key(destination: Destination, key: &Key) -> bool {
    route!(destination, handle_key(key), false)
}

/// Called every frame for every tab (visible or not): timers, background loads.
pub fn poll_all(now: Instant) {
    clock::poll(now);
    timers::poll(now);
    calendar::poll(now);
    weather::poll(now);
    clipboard::poll(now);
    notes::poll(now);
    shelf::poll(now);
    converter::poll(now);
    units::poll(now);
    links::poll(now);
    message::poll(now);
    screen_time::poll(now);
    ai_coding::poll(now);
    drop::poll(now);
}
