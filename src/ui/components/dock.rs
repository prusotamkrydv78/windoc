//! Bottom navigation dock: equal-width destinations with an active indicator.

use winisland_render::tokens::{SemanticColors, space, typography};
use winisland_render::{Painter, Point, Radius, Rect};

use super::semantics::{InteractionState, Role, SemanticNode};
use super::text::{Align, draw_text};

/// Meets the comfortable minimum target size.
pub const DOCK_HEIGHT: f32 = 44.0;

#[derive(Clone, PartialEq, Debug)]
pub struct DockLayout {
    pub bounds: Rect,
    pub items: Vec<Rect>,
}

impl DockLayout {
    pub fn new(bounds: Rect, count: usize) -> Self {
        if count == 0 {
            return Self {
                bounds,
                items: Vec::new(),
            };
        }
        let width = bounds.width().max(0.0) / count as f32;
        let items = (0..count)
            .map(|index| {
                let left = bounds.left + index as f32 * width;
                Rect::from_ltrb(left, bounds.top, left + width, bounds.bottom)
            })
            .collect();
        Self { bounds, items }
    }

    pub fn hit_test(&self, point: Point) -> Option<usize> {
        self.items.iter().position(|rect| rect.contains(point))
    }

    pub fn semantics(
        &self,
        labels: &[&str],
        active: usize,
        focused: Option<usize>,
    ) -> SemanticNode {
        let tabs = self
            .items
            .iter()
            .zip(labels)
            .enumerate()
            .map(|(index, (rect, label))| {
                SemanticNode::new(
                    Role::Tab,
                    *label,
                    *rect,
                    InteractionState {
                        selected: index == active,
                        focused: focused == Some(index),
                        ..InteractionState::default()
                    },
                )
            })
            .collect();
        SemanticNode::new(Role::TabList, "", self.bounds, InteractionState::default())
            .with_children(tabs)
    }
}

pub fn draw_dock(
    painter: Painter<'_>,
    layout: &DockLayout,
    labels: &[&str],
    icons: &[super::icon::Icon],
    active: usize,
    hovered: Option<usize>,
    focused: Option<usize>,
    colors: &SemanticColors,
) {
    // A flat row of icons under a hairline, like a launcher strip. The active page sits in a
    // soft round bubble with a small accent dot underneath; hovering shows the same bubble.
    // Labels stay for accessibility.
    painter.fill_rect(
        Rect::from_xywh(
            layout.bounds.left,
            layout.bounds.top,
            layout.bounds.width(),
            1.0,
        ),
        colors.border_subtle,
    );
    for (index, (rect, label)) in layout.items.iter().zip(labels).enumerate() {
        let is_active = index == active;
        let is_hovered = hovered == Some(index);
        let icon_only = true;
        let diameter = (rect.height() - space::S2).min(rect.width() - space::S1);
        let bubble = Rect::from_xywh(
            rect.center_x() - diameter / 2.0,
            rect.center_y() - diameter / 2.0,
            diameter,
            diameter,
        );
        if is_active || is_hovered {
            painter.fill_round_rect(
                Rect::from_xywh(rect.center_x() - 17.0, rect.center_y() - 17.0, 34.0, 34.0),
                Radius::uniform(17.0),
                if is_active {
                    colors.surface_interactive
                } else {
                    colors.surface_overlay
                },
            );
        }
        let color = if is_active {
            colors.text_primary
        } else {
            colors.text_secondary
        };
        match icons.get(index) {
            Some(icon) if icon_only => {
                let size = (diameter * 0.5).min(16.0);
                if is_active {
                    painter.fill_circle(
                        winisland_render::Point::new(
                            rect.center_x(),
                            rect.center_y() + size / 2.0 + 6.0,
                        ),
                        2.0,
                        winisland_render::Rgba::from_rgb(255, 176, 32),
                    );
                }
                super::icon::draw_icon(
                    painter,
                    *icon,
                    Rect::from_xywh(
                        rect.center_x() - size / 2.0,
                        rect.center_y() - size / 2.0,
                        size,
                        size,
                    ),
                    color,
                );
            }
            Some(icon) => {
                let size = 16.0;
                let text_width = 60.0f32.min(rect.width() - size - space::S3);
                let left = rect.center_x() - (size + space::S1 + text_width) / 2.0;
                super::icon::draw_icon(
                    painter,
                    *icon,
                    Rect::from_xywh(left, rect.center_y() - size / 2.0, size, size),
                    color,
                );
                draw_text(
                    painter,
                    label,
                    typography::LABEL,
                    Rect::from_xywh(left + size + space::S1, rect.top, text_width, rect.height()),
                    Align::Start,
                    color,
                );
            }
            None => draw_text(
                painter,
                label,
                typography::LABEL,
                *rect,
                Align::Center,
                color,
            ),
        }
        if focused == Some(index) {
            super::draw_focus_ring(painter, bubble, diameter / 2.0, colors);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_partition_the_dock() {
        let layout = DockLayout::new(Rect::from_xywh(10.0, 0.0, 600.0, DOCK_HEIGHT), 6);
        assert_eq!(layout.items.len(), 6);
        assert!((layout.items[0].left - 10.0).abs() < 0.01);
        assert!((layout.items[5].right - 610.0).abs() < 0.01);
        for pair in layout.items.windows(2) {
            assert!((pair[0].right - pair[1].left).abs() < 0.01);
        }
    }

    #[test]
    fn hit_test_finds_each_item() {
        let layout = DockLayout::new(Rect::from_xywh(0.0, 0.0, 600.0, DOCK_HEIGHT), 6);
        for (index, rect) in layout.items.iter().enumerate() {
            let center = Point::new(rect.center_x(), rect.center_y());
            assert_eq!(layout.hit_test(center), Some(index));
        }
        assert_eq!(layout.hit_test(Point::new(-1.0, 1.0)), None);
    }

    #[test]
    fn empty_dock_is_safe() {
        let layout = DockLayout::new(Rect::from_xywh(0.0, 0.0, 600.0, DOCK_HEIGHT), 0);
        assert!(layout.items.is_empty());
        assert_eq!(layout.hit_test(Point::new(10.0, 10.0)), None);
    }

    #[test]
    fn semantics_mark_active_tab_selected() {
        let layout = DockLayout::new(Rect::from_xywh(0.0, 0.0, 300.0, DOCK_HEIGHT), 3);
        let node = layout.semantics(&["Home", "Media", "More"], 1, None);
        assert_eq!(node.role, Role::TabList);
        let selected: Vec<bool> = node.children.iter().map(|tab| tab.state.selected).collect();
        assert_eq!(selected, [false, true, false]);
    }
}
