//! The taskbar replacement's own window, docked along the bottom of the monitor.
//!
//! The window is the bar plus a transparent area above it where the Control Centre,
//! Calendar and Notifications panels and the window previews open upward. Only the bar and an
//! open panel take mouse input; the rest of the window is click-through.

use std::time::Instant;

use winisland_platform::{
    InputState, MouseButton, MouseWheelDelta, OverlaySpec, OverlayStyles, PlatformEvent,
    WindowPosition, WindowSize,
};
use winisland_platform_windows::shell_bar::ScreenRect;
use winisland_render::Point;

use crate::platform::{WindowRef, window};

use super::App;

const BAR_WINDOW_TITLE: &str = "Notchy Bar";
/// Room above the bar for panels and previews, in logical pixels.
const PANEL_SPACE: f32 = 420.0;

/// Geometry of the bar window in physical pixels.
#[derive(Clone, Copy, Default, PartialEq)]
pub(super) struct BarGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

impl BarGeometry {
    /// Top of the bar strip inside the window.
    pub fn bar_top(&self) -> f32 {
        self.height as f32 - crate::core::top_bar::BAR_HEIGHT * self.scale
    }
}

impl App {
    /// Creates, moves or removes the bar window to match `active`.
    pub(super) fn sync_bar_window(&mut self, active: bool, monitor: ScreenRect, scale: f32) {
        let hidden = !active || self.is_fullscreen_suppressed;
        if !active {
            self.destroy_bar_window();
            return;
        }
        let height = ((crate::core::top_bar::BAR_HEIGHT + PANEL_SPACE) * scale).round() as u32;
        let geometry = BarGeometry {
            x: monitor.left,
            y: monitor.bottom - height as i32,
            width: (monitor.right - monitor.left).max(1) as u32,
            height,
            scale,
        };
        if self.bar_window.is_none() {
            self.create_bar_window(geometry);
        } else if self.bar_geometry != geometry {
            self.place_bar_window(geometry);
        }
        if let Some(bar) = self.bar_window {
            // Like the classic taskbar, the bar steps aside for full-screen apps.
            window().set_visible(bar.id(), !hidden);
        }
    }

    fn create_bar_window(&mut self, geometry: BarGeometry) {
        let size = WindowSize::new(geometry.width, geometry.height);
        let id = match window().create_overlay(OverlaySpec {
            title: BAR_WINDOW_TITLE,
            size,
        }) {
            Ok(id) => id,
            Err(error) => {
                log::warn!("Bar window could not be created: {error}");
                return;
            }
        };
        let bar = WindowRef(id);
        window().apply_overlay_styles(
            id,
            OverlayStyles {
                skip_taskbar: true,
                topmost: true,
            },
        );
        self.bar_window = Some(bar);
        self.place_bar_window(geometry);
        let target = window().native_surface(id).and_then(|surface| {
            self.renderer.as_mut().and_then(|renderer| {
                renderer
                    .create_target(surface, geometry.width, geometry.height)
                    .map_err(|error| log::warn!("Bar renderer target failed: {error}"))
                    .ok()
            })
        });
        self.bar_target = target;
        window().set_visible(id, true);
        bar.request_redraw();
        log::info!(
            "Bar window created: {}x{} at ({}, {})",
            geometry.width,
            geometry.height,
            geometry.x,
            geometry.y
        );
    }

    fn place_bar_window(&mut self, geometry: BarGeometry) {
        let Some(bar) = self.bar_window else {
            return;
        };
        let size = WindowSize::new(geometry.width, geometry.height);
        if self.bar_geometry.width != geometry.width || self.bar_geometry.height != geometry.height
        {
            let _ = bar.request_inner_size(size);
            if let (Some(renderer), Some(target)) = (self.renderer.as_mut(), self.bar_target) {
                let _ = renderer.resize(target, geometry.width, geometry.height);
            }
        }
        window().set_position(bar.id(), WindowPosition::new(geometry.x, geometry.y));
        self.bar_geometry = geometry;
        bar.request_redraw();
    }

    pub(super) fn destroy_bar_window(&mut self) {
        self.bar_preview = None;
        crate::ui::top_bar::set_preview_card(None);
        crate::ui::top_bar::popover::close();
        self.bar_target = None;
        if let Some(bar) = self.bar_window.take() {
            window().destroy_window(bar.id());
        }
        self.bar_geometry = BarGeometry::default();
    }

    pub(super) fn request_bar_redraw(&self) {
        if let Some(bar) = self.bar_window {
            bar.request_redraw();
        }
    }

    /// Top of the bar strip in bar-window coordinates.
    pub(super) fn bar_top(&self) -> f32 {
        self.bar_geometry.bar_top()
    }

    pub(super) fn is_bar_window(&self, id: winisland_platform::WindowId) -> bool {
        self.bar_window.is_some_and(|bar| bar.id() == id)
    }

    /// Per-frame: hover highlights, the live window preview, and which parts of the bar window
    /// take the mouse (the bar and an open panel; everything else clicks through).
    pub(super) fn update_bar_hover(&mut self, now: Instant) {
        let Some(bar) = self.bar_window else {
            return;
        };
        let (px, py) = crate::utils::mouse::get_global_cursor_pos();
        let local = Point::new(
            (px - self.bar_geometry.x) as f32,
            (py - self.bar_geometry.y) as f32,
        );
        let bar_top = self.bar_top();
        let inside = local.x >= 0.0
            && local.x < self.bar_geometry.width as f32
            && local.y < self.bar_geometry.height as f32;
        let over_bar = inside && local.y >= bar_top;
        let over_panel =
            crate::ui::top_bar::popover::rect().is_some_and(|rect| rect.contains(local));
        bar.set_cursor_hittest(over_bar || over_panel);
        let bar_point = Point::new(local.x, local.y - bar_top);
        if crate::ui::top_bar::set_hover(over_bar.then_some(bar_point)) {
            bar.request_redraw();
        }
        let hovered_app = over_bar
            .then(|| crate::ui::top_bar::hit(bar_point))
            .flatten()
            .and_then(|hit| match hit {
                crate::ui::top_bar::BarHit::App(handle) => Some(handle),
                _ => None,
            });
        self.update_bar_preview(&bar, hovered_app, now);
    }

    /// Events delivered to the bar window.
    pub(super) fn on_bar_window_event(&mut self, event: PlatformEvent) {
        let Some(bar) = self.bar_window else {
            return;
        };
        match event {
            PlatformEvent::RedrawRequested { .. } => self.draw_bar_window(),
            PlatformEvent::Resized { size, .. } if size.width > 0 && size.height > 0 => {
                // Keep the bottom edge on the screen bottom whatever height Windows gave us.
                if size.height != self.bar_geometry.height {
                    let bottom = self.bar_geometry.y + self.bar_geometry.height as i32;
                    self.bar_geometry.height = size.height;
                    self.bar_geometry.width = size.width;
                    self.bar_geometry.y = bottom - size.height as i32;
                    window().set_position(
                        bar.id(),
                        WindowPosition::new(self.bar_geometry.x, self.bar_geometry.y),
                    );
                }
                if let (Some(renderer), Some(target)) = (self.renderer.as_mut(), self.bar_target) {
                    let _ = renderer.resize(target, size.width, size.height);
                }
                bar.request_redraw();
            }
            PlatformEvent::MouseInput {
                state: InputState::Pressed,
                button,
                ..
            } => {
                let (px, py) = crate::utils::mouse::get_global_cursor_pos();
                let local = Point::new(
                    (px - self.bar_geometry.x) as f32,
                    (py - self.bar_geometry.y) as f32,
                );
                match button {
                    MouseButton::Left => {
                        if self.handle_popover_click(local.x as i32, local.y as i32) {
                            return;
                        }
                        let bar_point = Point::new(local.x, local.y - self.bar_top());
                        if let Some(hit) = crate::ui::top_bar::hit(bar_point) {
                            self.press_top_bar_item(hit);
                        }
                    }
                    MouseButton::Right if local.y >= self.bar_top() => {
                        self.show_top_bar_menu(px, py);
                    }
                    _ => {}
                }
            }
            PlatformEvent::MouseWheel { delta, .. } => {
                let (px, py) = crate::utils::mouse::get_global_cursor_pos();
                let point = Point::new(
                    (px - self.bar_geometry.x) as f32,
                    (py - self.bar_geometry.y) as f32 - self.bar_top(),
                );
                let steps = match delta {
                    MouseWheelDelta::Lines { y, .. } => y.signum(),
                    MouseWheelDelta::Pixels { y, .. } => (y as f32).signum(),
                };
                crate::ui::top_bar::scroll_app_volume(point, steps);
            }
            _ => {}
        }
    }

    fn draw_bar_window(&mut self) {
        let (Some(target), Some(mut renderer)) = (self.bar_target, self.renderer.take()) else {
            return;
        };
        let geometry = self.bar_geometry;
        let bar_top = geometry.bar_top();
        let bar_height = geometry.height as f32 - bar_top;
        let width = geometry.width as f32;
        let result = renderer.frame(target, |_, painter| {
            painter.save();
            painter.translate(winisland_render::Vec2::new(0.0, bar_top));
            // Apps run until the status area on the right.
            let apps_limit = width - 460.0 * geometry.scale;
            crate::ui::top_bar::draw(
                painter,
                width,
                bar_height,
                geometry.scale,
                (apps_limit, apps_limit),
            );
            painter.restore();
            crate::ui::top_bar::draw_preview_card(painter, geometry.scale);
            crate::ui::top_bar::popover::draw(painter, bar_top, width, geometry.scale);
        });
        self.renderer = Some(renderer);
        if let Err(error) = result {
            log::warn!("Bar frame failed: {error}");
        }
    }
}
