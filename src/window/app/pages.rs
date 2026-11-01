use std::time::Instant;

use crate::ui::expanded::pager::{self, ExpandedPage, PageAvailability, PagerHit, available_pages};
use crate::utils::mouse::is_point_in_continuous_rounded_rect;
use winisland_platform::MouseWheelDelta;
use winisland_render::{Point, Rect};

use super::{App, IslandLayout};

const WHEEL_PIXEL_THRESHOLD: f32 = 40.0;

impl App {
    pub(super) fn expanded_pages(&self) -> Vec<ExpandedPage> {
        available_pages(&PageAvailability {
            music: self.music_page_available,
        })
    }

    fn page_index(&self, page: ExpandedPage) -> Option<usize> {
        self.expanded_pages()
            .iter()
            .position(|candidate| *candidate == page)
    }

    fn page_distance(&self, page: ExpandedPage) -> Option<f32> {
        self.page_index(page)
            .map(|index| (index as f32 - self.springs.view.value).abs())
    }

    pub(super) fn page_translation(&self, page: ExpandedPage) -> f32 {
        self.page_index(page).map_or(0.0, |index| {
            (index as f32 - self.springs.view.value) * self.springs.w.value
        })
    }

    pub(super) fn page_visible(&self, page: ExpandedPage) -> bool {
        self.page_distance(page)
            .is_some_and(|distance| distance < 1.0)
    }

    pub(super) fn page_focused(&self, page: ExpandedPage) -> bool {
        self.page_distance(page)
            .is_some_and(|distance| distance < 0.5)
    }

    pub(super) fn target_page_position(&self) -> f32 {
        self.page_index(self.current_page).unwrap_or(0) as f32
    }

    pub(super) fn reset_page(&mut self) {
        crate::ui::expanded::calendar_view::reset_to_today();
        self.current_page = self
            .expanded_pages()
            .first()
            .copied()
            .unwrap_or(ExpandedPage::Widgets);
    }

    pub(super) fn snap_to_current_page(&mut self) {
        self.springs.view.value = self.target_page_position();
        self.springs.view.velocity = 0.0;
    }

    fn go_to_page_index(&mut self, index: usize) -> bool {
        match self.expanded_pages().get(index) {
            Some(page) if *page != self.current_page => {
                self.current_page = *page;
                true
            }
            _ => false,
        }
    }

    pub(super) fn set_music_page_available(&mut self, available: bool) {
        if available == self.music_page_available {
            return;
        }
        let previous = self.expanded_pages();
        let view = self.springs.view.value;
        let shown_index = (view.round().max(0.0) as usize).min(previous.len().saturating_sub(1));
        let shown = previous.get(shown_index).copied();
        self.music_page_available = available;
        let pages = self.expanded_pages();
        match shown.and_then(|page| pages.iter().position(|candidate| *candidate == page)) {
            Some(index) => self.springs.view.value = index as f32 + (view - shown_index as f32),
            None => {
                self.springs.view.value = shown_index.min(pages.len().saturating_sub(1)) as f32;
                self.springs.view.velocity = 0.0;
            }
        }
        if !pages.contains(&self.current_page) {
            self.current_page = shown
                .filter(|page| pages.contains(page))
                .or_else(|| pages.first().copied())
                .unwrap_or(ExpandedPage::Widgets);
        }
    }

    fn island_rect(&self, layout: &IslandLayout) -> Rect {
        Rect::from_xywh(
            layout.current_island_x as f32,
            layout.current_island_y as f32,
            self.springs.w.value,
            self.springs.h.value,
        )
    }

    pub(super) fn pager_contains(&self, rel_x: i32, rel_y: i32, layout: &IslandLayout) -> bool {
        self.expanded
            && !self.config.notch_shape
            && self.current_page != ExpandedPage::Home
            && pager::contains(
                self.island_rect(layout),
                self.expanded_pages().len(),
                self.config.expanded_scale,
                layout.dock_bottom,
                Point::new(rel_x as f32, rel_y as f32),
            )
    }

    pub(super) fn handle_pager_press(
        &mut self,
        rel_x: i32,
        rel_y: i32,
        layout: &IslandLayout,
    ) -> bool {
        if !self.expanded {
            return false;
        }
        let Some(hit) = pager::hit_test(
            self.island_rect(layout),
            self.expanded_pages().len(),
            self.springs.view.value,
            self.config.expanded_scale,
            layout.dock_bottom,
            Point::new(rel_x as f32, rel_y as f32),
        ) else {
            return false;
        };
        match hit {
            PagerHit::Page(index) => {
                self.go_to_page_index(index);
            }
            PagerHit::Close => {
                self.expanded = false;
                self.reset_page();
            }
        }
        true
    }

    pub(super) fn update_pager_hover(
        &mut self,
        window: &crate::platform::WindowRef,
        rel_x: i32,
        rel_y: i32,
        layout: &IslandLayout,
        interaction_allowed: bool,
        dt: f32,
    ) {
        let hit = (interaction_allowed && self.expanded)
            .then(|| {
                pager::hit_test(
                    self.island_rect(layout),
                    self.expanded_pages().len(),
                    self.springs.view.value,
                    self.config.expanded_scale,
                    layout.dock_bottom,
                    Point::new(rel_x as f32, rel_y as f32),
                )
            })
            .flatten();
        let close_target = if hit == Some(PagerHit::Close) {
            1.0
        } else {
            0.0
        };
        let bar_target = if matches!(hit, Some(PagerHit::Page(_))) {
            1.0
        } else {
            0.0
        };
        let before = (self.close_hover.value, self.bar_hover.value);
        for (spring, target) in [
            (&mut self.close_hover, close_target),
            (&mut self.bar_hover, bar_target),
        ] {
            spring.update_dt(target, 0.22, 0.62, dt);
            spring.settle(target, 0.002, 0.001);
        }
        let calendar_hover =
            (interaction_allowed && self.expanded && self.page_focused(ExpandedPage::Calendar))
                .then(|| {
                    crate::ui::expanded::calendar_view::hit_test(
                        layout.offset_x as f32 + self.page_translation(ExpandedPage::Calendar),
                        layout.island_y as f32,
                        self.springs.w.value,
                        (self.config.expanded_height * self.config.expanded_scale)
                            .min(self.springs.h.value),
                        self.config.expanded_scale,
                        Point::new(rel_x as f32, rel_y as f32),
                    )
                })
                .flatten();
        let calendar_changed = crate::ui::expanded::calendar_view::set_hover(calendar_hover);
        let dashboard_hover =
            (interaction_allowed && self.expanded && self.page_focused(ExpandedPage::Home))
                .then(|| {
                    crate::ui::dashboard::hit_test(
                        layout.offset_x as f32 + self.page_translation(ExpandedPage::Home),
                        layout.island_y as f32,
                        self.springs.w.value,
                        (self.config.expanded_height * self.config.expanded_scale)
                            .min(self.springs.h.value),
                        self.config.expanded_scale,
                        Point::new(rel_x as f32, rel_y as f32),
                    )
                })
                .flatten();
        let dashboard_changed = crate::ui::dashboard::set_hover(dashboard_hover);
        if (self.close_hover.value, self.bar_hover.value) != before
            || calendar_changed
            || dashboard_changed
        {
            window.request_redraw();
        }
    }

    /// Scrolling over the collapsed notch changes the system volume in 2% steps; the volume
    /// live activity picks the change up on its own.
    fn handle_collapsed_wheel(&mut self, delta: MouseWheelDelta, px: i32, py: i32) -> bool {
        if crate::ui::top_bar::is_active() {
            let point = Point::new((px - self.geom.win_x) as f32, (py - self.geom.win_y) as f32);
            let steps = match delta {
                MouseWheelDelta::Lines { y, .. } => y.signum(),
                MouseWheelDelta::Pixels { y, .. } => (y as f32).signum(),
            };
            if crate::ui::top_bar::scroll_app_volume(point, steps) {
                return true;
            }
        }
        const VOLUME_STEP: f32 = 0.02;
        let layout = self.compute_island_layout();
        let over_island = is_point_in_continuous_rounded_rect(
            (px - self.geom.win_x) as f64,
            (py - self.geom.win_y) as f64,
            layout.current_island_x,
            layout.current_island_y.min(0.0),
            self.springs.w.value as f64,
            self.springs.h.value as f64 + layout.current_island_y.max(0.0),
            self.springs.r.value as f64,
        );
        if !over_island || self.is_hidden() {
            return false;
        }
        let steps = match delta {
            MouseWheelDelta::Lines { y, .. } => y,
            MouseWheelDelta::Pixels { y, .. } => {
                self.wheel_accumulator += y as f32;
                let steps = (self.wheel_accumulator / WHEEL_PIXEL_THRESHOLD).trunc();
                self.wheel_accumulator -= steps * WHEEL_PIXEL_THRESHOLD;
                steps
            }
        };
        if steps == 0.0 {
            return true;
        }
        std::thread::spawn(move || {
            let audio = crate::platform::audio();
            if let Ok(state) = audio.volume() {
                let _ = audio.set_volume((state.level + steps * VOLUME_STEP).clamp(0.0, 1.0));
            }
        });
        true
    }

    pub(super) fn handle_mouse_wheel(&mut self, delta: MouseWheelDelta, px: i32, py: i32) -> bool {
        if !self.expanded {
            return self.handle_collapsed_wheel(delta, px, py);
        }
        let rel_x = px - self.geom.win_x;
        let rel_y = py - self.geom.win_y;
        let layout = self.compute_island_layout();
        if rel_y as f32
            >= layout.current_island_y as f32
                + self.config.expanded_height * self.config.expanded_scale
        {
            return false;
        }
        let over_island = is_point_in_continuous_rounded_rect(
            rel_x as f64,
            rel_y as f64,
            layout.current_island_x,
            layout.current_island_y,
            self.springs.w.value as f64,
            self.springs.h.value as f64,
            self.springs.r.value as f64,
        );
        self.idle_timer = Instant::now();
        // Lists inside a tab (clipboard, shelf) scroll; one notch is one line.
        let lines = match delta {
            MouseWheelDelta::Lines { y, .. } => y,
            MouseWheelDelta::Pixels { y, .. } => y as f32 / WHEEL_PIXEL_THRESHOLD,
        };
        if over_island
            && self.page_focused(ExpandedPage::Home)
            && lines != 0.0
            && crate::ui::dashboard::scroll(
                layout.offset_x as f32 + self.page_translation(ExpandedPage::Home),
                layout.island_y as f32,
                self.springs.w.value,
                (self.config.expanded_height * self.config.expanded_scale)
                    .min(self.springs.h.value),
                self.config.expanded_scale,
                Point::new(rel_x as f32, rel_y as f32),
                lines,
            )
        {
            crate::platform::wake();
            return true;
        }
        // The panel navigates with its dock; the wheel no longer flips to the old full-size
        // pages. It is still taken over the panel so the window behind does not scroll.
        over_island || self.pager_contains(rel_x, rel_y, &layout)
    }
}
