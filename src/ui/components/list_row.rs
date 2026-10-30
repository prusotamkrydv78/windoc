//! List row with optional leading icon, subtitle and trailing accessory.
//!
//! Hover, pressed, selected, focused and dragging states each look different.

use winisland_render::tokens::{Density, SemanticColors, radius, space, typography};
use winisland_render::{Painter, Radius, Rect};

use super::semantics::{InteractionState, Role, SemanticNode};
use super::text::{Align, draw_text};

pub const LEADING_SIZE: f32 = 24.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ListRowLayout {
    pub bounds: Rect,
    pub leading: Option<Rect>,
    pub title: Rect,
    pub subtitle: Option<Rect>,
    pub trailing: Option<Rect>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ListRowSpec {
    pub density: Density,
    pub has_leading: bool,
    pub has_subtitle: bool,
    /// Zero for no trailing accessory.
    pub trailing_width: f32,
}

impl ListRowLayout {
    pub fn new(left: f32, top: f32, width: f32, spec: ListRowSpec) -> Self {
        let height = spec.density.list_row_height();
        let bounds = Rect::from_xywh(left, top, width.max(0.0), height);
        let pad = space::S3;
        let mut text_left = bounds.left + pad;
        let mut text_right = (bounds.right - pad).max(text_left);

        let leading = spec.has_leading.then(|| {
            let rect = Rect::from_xywh(
                text_left,
                bounds.center_y() - LEADING_SIZE / 2.0,
                LEADING_SIZE,
                LEADING_SIZE,
            );
            text_left = rect.right + space::S3;
            rect
        });
        let trailing = (spec.trailing_width > 0.0).then(|| {
            let right = text_right.max(text_left);
            let rect = Rect::from_ltrb(
                (right - spec.trailing_width).max(text_left),
                bounds.top,
                right,
                bounds.bottom,
            );
            text_right = rect.left - space::S2;
            rect
        });
        let text_right = text_right.max(text_left);

        let (title, subtitle) = if spec.has_subtitle {
            let title_h = typography::BODY.size + space::S1;
            let subtitle_h = typography::CAPTION.size + space::S1;
            let top = bounds.center_y() - (title_h + subtitle_h) / 2.0;
            (
                Rect::from_ltrb(text_left, top, text_right, top + title_h),
                Some(Rect::from_ltrb(
                    text_left,
                    top + title_h,
                    text_right,
                    top + title_h + subtitle_h,
                )),
            )
        } else {
            (
                Rect::from_ltrb(text_left, bounds.top, text_right, bounds.bottom),
                None,
            )
        };
        Self {
            bounds,
            leading,
            title,
            subtitle,
            trailing,
        }
    }

    pub fn semantics(&self, name: &str, state: InteractionState) -> SemanticNode {
        SemanticNode::new(Role::ListItem, name, self.bounds, state)
    }
}

/// Draws the row background, text and focus ring. Callers draw the leading icon and
/// trailing accessory into their rects.
pub fn draw_list_row(
    painter: Painter<'_>,
    layout: &ListRowLayout,
    title: &str,
    subtitle: Option<&str>,
    colors: &SemanticColors,
    state: InteractionState,
) {
    let shape = Radius::uniform(radius::ROW);
    let background = if state.dragging {
        Some(colors.surface_raised)
    } else if state.selected {
        Some(colors.accent_soft)
    } else if state.pressed {
        Some(colors.surface_interactive)
    } else if state.hovered {
        Some(colors.surface_overlay)
    } else {
        None
    };
    if let Some(color) = background {
        painter.fill_round_rect(layout.bounds, shape, color);
    }
    if state.dragging {
        painter.stroke_round_rect(layout.bounds, shape, 1.0, colors.border_strong);
    }
    let (title_color, subtitle_color) = if state.disabled {
        (colors.text_disabled, colors.text_disabled)
    } else {
        (colors.text_primary, colors.text_secondary)
    };
    draw_text(
        painter,
        title,
        typography::BODY,
        layout.title,
        Align::Start,
        title_color,
    );
    if let (Some(text), Some(rect)) = (subtitle, layout.subtitle) {
        draw_text(
            painter,
            text,
            typography::CAPTION,
            rect,
            Align::Start,
            subtitle_color,
        );
    }
    if state.focused {
        super::draw_focus_ring(painter, layout.bounds, radius::ROW, colors);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(density: Density) -> ListRowSpec {
        ListRowSpec {
            density,
            has_leading: true,
            has_subtitle: true,
            trailing_width: 40.0,
        }
    }

    #[test]
    fn height_follows_density() {
        for density in [Density::Comfortable, Density::Compact, Density::Touch] {
            let layout = ListRowLayout::new(0.0, 0.0, 320.0, spec(density));
            assert_eq!(layout.bounds.height(), density.list_row_height());
        }
    }

    #[test]
    fn text_sits_between_leading_and_trailing() {
        let layout = ListRowLayout::new(0.0, 0.0, 320.0, spec(Density::Comfortable));
        let leading = layout.leading.expect("leading");
        let trailing = layout.trailing.expect("trailing");
        assert!(layout.title.left >= leading.right);
        assert!(layout.title.right <= trailing.left);
        let subtitle = layout.subtitle.expect("subtitle");
        assert!(subtitle.top >= layout.title.bottom);
        assert!(layout.bounds.contains_rect(subtitle));
    }

    #[test]
    fn narrow_row_never_inverts_rects() {
        let layout = ListRowLayout::new(0.0, 0.0, 30.0, spec(Density::Compact));
        assert!(layout.title.width() >= 0.0);
        assert!(layout.trailing.expect("trailing").width() >= 0.0);
    }
}
