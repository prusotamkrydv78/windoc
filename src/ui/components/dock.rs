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
    icons: &[super::icon::Icon],
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
        if is_active {
            // A soft pill marks the active tab; the bolder colour repeats it for clarity.
            painter.fill_round_rect(cell, Radius::uniform(radius::CONTROL), colors.accent_soft);
        }
        let color = if is_active {
            colors.accent_primary
        } else {
            colors.text_secondary
        };
        match icons.get(index) {
            Some(icon) => {
                let icon_size = (cell.height() * 0.42).min(18.0);
                let icon_top =
                    cell.top + (cell.height() - icon_size - typography::CAPTION.size - 2.0) / 2.0;
                super::icon::draw_icon(
                    painter,
                    *icon,
                    Rect::from_xywh(
                        cell.center_x() - icon_size / 2.0,
                        icon_top,
                        icon_size,
                        icon_size,
                    ),
                    color,
                );
                draw_text(
                    painter,
                    label,
                    typography::CAPTION,
                    Rect::from_ltrb(
                        cell.left,
                        icon_top + icon_size + 2.0,
                        cell.right,
                        icon_top + icon_size + 2.0 + typography::CAPTION.size + 2.0,
                    ),
                    Align::Center,
                    color,
                );
            }
            None => draw_text(
                painter,
                label,
                typography::LABEL,
                cell,
                Align::Center,
                color,
            ),
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
