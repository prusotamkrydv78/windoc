//! Module card: a heading, optional status, content area and at most two visible actions.

use winisland_render::tokens::{SemanticColors, radius, space, typography};
use winisland_render::{Painter, Point, Radius, Rect};

use super::semantics::{InteractionState, Role, SemanticNode};
use super::text::{Align, draw_text};

/// Further actions belong in an overflow menu, not on the card.
pub const MAX_VISIBLE_ACTIONS: usize = 2;
pub const ACTION_SIZE: f32 = 28.0;

#[derive(Clone, PartialEq, Debug)]
pub struct CardLayout {
    pub bounds: Rect,
    pub header: Rect,
    pub content: Rect,
    /// Right-aligned in the header row, first action rightmost.
    pub actions: Vec<Rect>,
}

impl CardLayout {
    pub fn new(bounds: Rect, action_count: usize) -> Self {
        let pad = space::S4;
        let header_h = ACTION_SIZE.max(typography::SECTION_TITLE.size + space::S2);
        let inner_left = bounds.left + pad;
        let inner_right = (bounds.right - pad).max(inner_left);
        let header_top = bounds.top + pad;

        let mut actions = Vec::new();
        let mut header_right = inner_right;
        for index in 0..action_count.min(MAX_VISIBLE_ACTIONS) {
            let right = inner_right - index as f32 * (ACTION_SIZE + space::S2);
            let left = right - ACTION_SIZE;
            if left < inner_left {
                break;
            }
            actions.push(Rect::from_ltrb(
                left,
                header_top + (header_h - ACTION_SIZE) / 2.0,
                right,
                header_top + (header_h + ACTION_SIZE) / 2.0,
            ));
            header_right = left - space::S2;
        }

        let header = Rect::from_ltrb(
            inner_left,
            header_top,
            header_right.max(inner_left),
            header_top + header_h,
        );
        let content_top = header.bottom + space::S2;
        let content = Rect::from_ltrb(
            inner_left,
            content_top,
            inner_right,
            (bounds.bottom - pad).max(content_top),
        );
        Self {
            bounds,
            header,
            content,
            actions,
        }
    }

    pub fn hit_action(&self, point: Point) -> Option<usize> {
        self.actions.iter().position(|rect| rect.contains(point))
    }

    pub fn semantics(
        &self,
        title: &str,
        action_names: &[&str],
        state: InteractionState,
    ) -> SemanticNode {
        let actions = self
            .actions
            .iter()
            .zip(action_names)
            .map(|(rect, name)| {
                SemanticNode::new(Role::Button, *name, *rect, InteractionState::default())
            })
            .collect();
        SemanticNode::new(Role::Group, title, self.bounds, state).with_children(actions)
    }
}

/// Draws the card surface and title. Callers draw content into `layout.content` and
/// action glyphs into `layout.actions`.
pub fn draw_card(
    painter: Painter<'_>,
    layout: &CardLayout,
    title: &str,
    icon: Option<super::icon::Icon>,
    colors: &SemanticColors,
    state: InteractionState,
) {
    let shape = Radius::uniform(radius::CARD);
    let fill = if state.pressed {
        colors.surface_interactive
    } else if state.hovered {
        colors.surface_overlay
    } else {
        colors.surface_raised
    };
    painter.fill_round_rect(layout.bounds, shape, fill);
    painter.stroke_round_rect(layout.bounds, shape, 1.0, colors.border_subtle);
    // A quiet label with an icon keeps the value below as the focus of the card.
    let title_color = if state.disabled {
        colors.text_disabled
    } else {
        colors.text_secondary
    };
    let mut title_rect = layout.header;
    if let Some(icon) = icon {
        let size = typography::LABEL.size + 4.0;
        super::icon::draw_icon(
            painter,
            icon,
            Rect::from_xywh(
                title_rect.left,
                title_rect.center_y() - size / 2.0,
                size,
                size,
            ),
            colors.accent_primary,
        );
        title_rect.left += size + space::S2;
    }
    draw_text(
        painter,
        title,
        typography::LABEL,
        title_rect,
        Align::Start,
        title_color,
    );
    if state.focused {
        super::draw_focus_ring(painter, layout.bounds, radius::CARD, colors);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Rect {
        Rect::from_xywh(0.0, 0.0, 240.0, 140.0)
    }

    #[test]
    fn caps_visible_actions() {
        assert_eq!(
            CardLayout::new(bounds(), 5).actions.len(),
            MAX_VISIBLE_ACTIONS
        );
    }

    #[test]
    fn regions_stay_inside_bounds_and_do_not_overlap() {
        let layout = CardLayout::new(bounds(), 2);
        for region in [layout.header, layout.content] {
            assert!(layout.bounds.contains_rect(region));
        }
        for action in &layout.actions {
            assert!(layout.bounds.contains_rect(*action));
            assert!(action.left >= layout.header.right);
            assert!(layout.content.top >= action.bottom);
        }
    }

    #[test]
    fn tiny_card_drops_actions_instead_of_overflowing() {
        let layout = CardLayout::new(Rect::from_xywh(0.0, 0.0, 40.0, 40.0), 2);
        assert!(layout.actions.is_empty());
        assert!(layout.header.width() >= 0.0 && layout.content.height() >= 0.0);
    }

    #[test]
    fn hit_action_maps_points_to_indices() {
        let layout = CardLayout::new(bounds(), 2);
        let first = layout.actions[0];
        assert_eq!(
            layout.hit_action(Point::new(first.center_x(), first.center_y())),
            Some(0)
        );
        assert_eq!(layout.hit_action(Point::new(1.0, 1.0)), None);
    }
}
