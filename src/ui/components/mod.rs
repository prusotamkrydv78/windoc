//! Shared dashboard components built on `winisland_render::tokens`.
//!
//! Each component separates pure layout (geometry and hit testing, unit-tested without a
//! GPU) from drawing, and describes itself as a [`semantics::SemanticNode`] so the future
//! UI Automation provider can expose the same structure that is drawn.

// The dashboard shell (roadmap step 5) is the first caller; until then these are unused.
#![allow(dead_code)]

pub mod card;
pub mod dock;
pub mod icon;
pub mod list_row;
pub mod semantics;
pub mod text;

use winisland_render::tokens::SemanticColors;
use winisland_render::{Painter, Radius, Rect};

/// Keyboard focus ring, drawn outside the component so it never hides content.
pub fn draw_focus_ring(painter: Painter<'_>, bounds: Rect, radius: f32, colors: &SemanticColors) {
    const GAP: f32 = 2.0;
    const WIDTH: f32 = 2.0;
    painter.stroke_round_rect(
        bounds.inset(-GAP),
        Radius::uniform(radius + GAP),
        WIDTH,
        colors.accent_primary,
    );
}
