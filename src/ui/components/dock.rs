//! Bottom navigation dock: equal-width destinations with an active indicator.

use winisland_render::tokens::{SemanticColors, radius, space, typography};
use winisland_render::{Painter, Point, Radius, Rect};

use super::semantics::{InteractionState, Role, SemanticNode};
use super::text::{Align, draw_text};

/// Meets the comfortable minimum target size.
pub const DOCK_HEIGHT: f32 = 44.0;
const INDICATOR_WIDTH: f32 = 18.0;
const INDICATOR_HEIGHT: f32 = 3.0;

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
    active: usize,
    hovered: Option<usize>,
    focused: Option<usize>,
    colors: &SemanticColors,
) {
    for (index, (rect, label)) in layout.items.iter().zip(labels).enumerate() {
        let cell = rect.inset(space::S1);
        if hovered == Some(index) {
            painter.fill_round_rect(
                cell,
                Radius::uniform(radius::CONTROL),
                colors.surface_interactive,
            );
        }
        let is_active = index == active;
        let color = if is_active {
            colors.text_primary
        } else {
            colors.text_secondary
        };
        let label_rect = Rect::from_ltrb(
            cell.left,
            cell.top,
            cell.right,
            cell.bottom - INDICATOR_HEIGHT - space::S1,
        );
        draw_text(
            painter,
            label,
            typography::LABEL,
            label_rect,
            Align::Center,
            color,
        );
        if is_active {
            // Shape plus color, so the active state never relies on color alone.
            let indicator = Rect::from_xywh(
                cell.center_x() - INDICATOR_WIDTH / 2.0,
                cell.bottom - INDICATOR_HEIGHT,
                INDICATOR_WIDTH,
                INDICATOR_HEIGHT,
            );
            painter.fill_round_rect(
                indicator,
                Radius::uniform(INDICATOR_HEIGHT / 2.0),
                colors.accent_primary,
            );
        }
        if focused == Some(index) {
            super::draw_focus_ring(painter, cell, radius::CONTROL, colors);
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
