//! Dashboard geometry for both size classes.
//!
//! Row height is fixed per layout, so refreshing values never moves cards. When cards
//! need more rows than fit, `content_height` exceeds the grid height and the grid scrolls.

use winisland_render::tokens::{SizeClass, space};
use winisland_render::{Point, Rect};

use super::Destination;
use crate::ui::components::dock::{DOCK_HEIGHT, DockLayout};

pub const HEADER_HEIGHT: f32 = 34.0;
const VISIBLE_ROWS: usize = 2;

#[derive(Clone, PartialEq, Debug)]
pub struct DashboardLayout {
    pub bounds: Rect,
    pub size_class: SizeClass,
    pub header: Rect,
    pub grid: Rect,
    /// One rect per card, in input order. May extend below `grid` when it overflows.
    pub cards: Vec<Rect>,
    /// Total height of the laid-out cards.
    pub content_height: f32,
    pub dock: DockLayout,
    pub destinations: Vec<Destination>,
}

impl DashboardLayout {
    pub const fn columns(size_class: SizeClass) -> usize {
        match size_class {
            SizeClass::Compact => 2,
            SizeClass::Standard => 3,
        }
    }

    /// `spans` gives each card's requested column span.
    pub fn new(bounds: Rect, size_class: SizeClass, spans: &[usize]) -> Self {
        let pad = space::S3;
        let inner_left = bounds.left + pad;
        let inner_right = (bounds.right - pad).max(inner_left);
        let header = Rect::from_ltrb(
            inner_left,
            bounds.top + space::S2,
            inner_right,
            bounds.top + space::S2 + HEADER_HEIGHT,
        );
        let dock_bottom = (bounds.bottom - space::S1).max(header.bottom);
        let dock_rect = Rect::from_ltrb(
            inner_left,
            (dock_bottom - DOCK_HEIGHT).max(header.bottom),
            inner_right,
            dock_bottom,
        );
        let grid = Rect::from_ltrb(
            inner_left,
            header.bottom + space::S1,
            inner_right,
            (dock_rect.top - space::S1).max(header.bottom + space::S1),
        );

        let columns = Self::columns(size_class);
        let gap = space::S2;
        let column_width = ((grid.width() - gap * (columns - 1) as f32) / columns as f32).max(0.0);
        let row_height =
            ((grid.height() - gap * (VISIBLE_ROWS - 1) as f32) / VISIBLE_ROWS as f32).max(0.0);

        let mut cards = Vec::with_capacity(spans.len());
        let (mut column, mut row) = (0, 0);
        for &requested in spans {
            let span = requested.clamp(1, columns);
            if column + span > columns {
                column = 0;
                row += 1;
            }
            let left = grid.left + column as f32 * (column_width + gap);
            let top = grid.top + row as f32 * (row_height + gap);
            cards.push(Rect::from_xywh(
                left,
                top,
                span as f32 * column_width + (span - 1) as f32 * gap,
                row_height,
            ));
            column += span;
        }
        let rows = if cards.is_empty() { 0 } else { row + 1 };
        let content_height = if rows == 0 {
            0.0
        } else {
            rows as f32 * row_height + (rows - 1) as f32 * gap
        };

        // One icon dock lists every module, like a launcher; hidden modules are skipped.
        let _ = size_class;
        let destinations: Vec<Destination> = Destination::DOCK
            .into_iter()
            .filter(|destination| !super::module_hidden(*destination))
            .collect();
        let dock = DockLayout::new(dock_rect, destinations.len());
        Self {
            bounds,
            size_class,
            header,
            grid,
            cards,
            content_height,
            dock,
            destinations,
        }
    }

    pub fn overflows(&self) -> bool {
        self.content_height > self.grid.height() + 0.5
    }

    /// Card under `point`, ignoring cards scrolled outside the grid.
    pub fn hit_card(&self, point: Point) -> Option<usize> {
        if !self.grid.contains(point) {
            return None;
        }
        self.cards.iter().position(|rect| rect.contains(point))
    }

    pub fn hit_destination(&self, point: Point) -> Option<Destination> {
        self.dock
            .hit_test(point)
            .and_then(|index| self.destinations.get(index).copied())
    }
}

#[cfg(test)]
mod tests {
    use super::super::sample::HOME_CARDS;
    use super::*;

    fn spans() -> Vec<usize> {
        HOME_CARDS.iter().map(|card| card.column_span).collect()
    }

    fn standard() -> DashboardLayout {
        DashboardLayout::new(
            Rect::from_xywh(0.0, 0.0, 740.0, 300.0),
            SizeClass::Standard,
            &spans(),
        )
    }

    fn compact() -> DashboardLayout {
        DashboardLayout::new(
            Rect::from_xywh(0.0, 0.0, 480.0, 300.0),
            SizeClass::Compact,
            &spans(),
        )
    }

    fn overlaps(a: Rect, b: Rect) -> bool {
        a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
    }

    #[test]
    fn standard_fits_two_rows_without_scrolling() {
        let layout = standard();
        assert!(!layout.overflows());
        for card in &layout.cards {
            assert!(layout.grid.contains_rect(*card));
        }
    }

    #[test]
    fn regions_stack_header_grid_dock() {
        for layout in [standard(), compact()] {
            assert!(layout.header.bottom <= layout.grid.top);
            assert!(layout.grid.bottom <= layout.dock.bounds.top);
            assert!(layout.bounds.contains_rect(layout.dock.bounds));
        }
    }

    #[test]
    fn cards_never_overlap() {
        for layout in [standard(), compact()] {
            for (index, a) in layout.cards.iter().enumerate() {
                for b in &layout.cards[index + 1..] {
                    assert!(!overlaps(*a, *b), "{a:?} overlaps {b:?}");
                }
            }
        }
    }

    #[test]
    fn compact_uses_fewer_columns_and_scrolls() {
        let layout = compact();
        // Both sizes share the full dock; the compact one scrolls instead.
        assert_eq!(layout.destinations, standard().destinations);
        assert!(layout.overflows());
        assert!(
            DashboardLayout::columns(SizeClass::Compact) < DashboardLayout::columns(SizeClass::Standard)
        );
    }

    #[test]
    fn row_height_is_stable_across_cards() {
        let layout = standard();
        let height = layout.cards[0].height();
        assert!(
            layout
                .cards
                .iter()
                .all(|card| (card.height() - height).abs() < 0.01)
        );
    }

    #[test]
    fn hit_testing_maps_cards_and_destinations() {
        let layout = standard();
        let focus = layout.cards[2];
        assert_eq!(
            layout.hit_card(Point::new(focus.center_x(), focus.center_y())),
            Some(2)
        );
        let first = layout.dock.items[0];
        assert_eq!(
            layout.hit_destination(Point::new(first.center_x(), first.center_y())),
            Some(Destination::Home)
        );
    }

    #[test]
    fn empty_and_degenerate_inputs_are_safe() {
        let empty = DashboardLayout::new(
            Rect::from_xywh(0.0, 0.0, 740.0, 300.0),
            SizeClass::Standard,
            &[],
        );
        assert!(empty.cards.is_empty());
        assert_eq!(empty.content_height, 0.0);
        let tiny = DashboardLayout::new(
            Rect::from_xywh(0.0, 0.0, 10.0, 10.0),
            SizeClass::Compact,
            &[1, 9],
        );
        assert!(
            tiny.cards
                .iter()
                .all(|card| card.width() >= 0.0 && card.height() >= 0.0)
        );
    }
}
