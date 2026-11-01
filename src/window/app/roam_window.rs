//! The see-through strip the robot plays in when it leaves the notch: along the top of the
//! taskbar when it shows, or the bottom of the screen when it is hidden. The window exists
//! only while the robot is out, and only the robot itself takes the mouse.

use winisland_platform::{
    InputState, MouseButton, OverlaySpec, OverlayStyles, PlatformEvent, WindowPosition, WindowSize,
};
use winisland_render::Point;

use crate::platform::{WindowRef, window};
use crate::ui::widget::compact::mascot::roam;

use super::App;

const ROAM_WINDOW_TITLE: &str = "WinIsland Robot";

/// Geometry of the playground window in physical pixels.
#[derive(Clone, Copy, Default, PartialEq)]
pub(super) struct RoamGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

impl App {
    /// Per tick: lets the robot plan trips and keeps its window in step with them.
    pub(super) fn sync_roam_window(&mut self) {
        let robot_shown = self.config.robot_enabled
            && self.config.compact_widget_layout.iter().any(|slot| {
                slot.widget == Some(winisland_core::config::CompactWidgetKind::Mascot)
            });
        let hour = crate::platform::shell().local_datetime().hour;
        // The test hook sends the robot out whatever the hour or Do Not Disturb.
        let test_trips = std::env::var_os("WINISLAND_MASCOT_DEMO_ROAM").is_some();
        let allowed = robot_shown
            && !self.expanded
            && !self.is_hidden()
            && (test_trips
                || (!self.is_fullscreen_suppressed
                    && (5..23).contains(&hour)
                    && !crate::ui::dashboard::focus_running()
                    && !crate::core::system_watch::snapshot().do_not_disturb));
        roam::tick(self.config.robot_roam, allowed);
        if !allowed {
            roam::come_home();
        }
        if !roam::active() {
            self.destroy_roam_window();
            return;
        }
        let Some(geometry) = self.roam_geometry() else {
            return;
        };
        if self.roam_window.is_none() {
            self.create_roam_window(geometry);
        } else if self.roam_geometry != geometry {
            self.place_roam_window(geometry);
        }
        if let Some(roam_window) = self.roam_window {
            // Only the robot takes clicks; the rest of the strip clicks through.
            let (px, py) = crate::utils::mouse::get_global_cursor_pos();
            let local = Point::new(
                (px - self.roam_geometry.x) as f32,
                (py - self.roam_geometry.y) as f32,
            );
            let over_robot = roam::robot_rect(geometry.scale, geometry.height as f32)
                .is_some_and(|rect| rect.contains(local));
            roam_window.set_cursor_hittest(over_robot);
            roam_window.request_redraw();
        }
    }

    /// The strip along the work area's bottom (the taskbar's top, or the screen's bottom
    /// when the taskbar is hidden) on the notch's monitor.
    fn roam_geometry(&self) -> Option<RoamGeometry> {
        let window_ref = self.window?;
        let scale = window().scale_factor(window_ref.id()) as f32;
        let (mx, my) = self.geom.monitor_pos;
        let (mw, mh) = self.geom.monitor_size;
        let (_, work) = winisland_platform_windows::monitor_areas_at(
            mx + mw as i32 / 2,
            my + mh as i32 / 2,
        )?;
        let height = (roam::PLAYGROUND_HEIGHT * scale).round() as u32;
        Some(RoamGeometry {
            x: work.left,
            y: work.bottom - height as i32,
            width: (work.right - work.left).max(1) as u32,
            height,
            scale,
        })
    }

    fn create_roam_window(&mut self, geometry: RoamGeometry) {
        let size = WindowSize::new(geometry.width, geometry.height);
        let id = match window().create_overlay(OverlaySpec {
            title: ROAM_WINDOW_TITLE,
            size,
        }) {
            Ok(id) => id,
            Err(error) => {
                log::warn!("Robot playground could not be created: {error}");
                roam::come_home();
                return;
            }
        };
        window().apply_overlay_styles(
            id,
            OverlayStyles {
                skip_taskbar: true,
                topmost: true,
            },
        );
        self.roam_window = Some(WindowRef(id));
        self.place_roam_window(geometry);
        self.roam_target = window().native_surface(id).and_then(|surface| {
            self.renderer.as_mut().and_then(|renderer| {
                renderer
                    .create_target(surface, geometry.width, geometry.height)
                    .map_err(|error| log::warn!("Robot playground target failed: {error}"))
                    .ok()
            })
        });
        window().set_visible(id, true);
    }

    fn place_roam_window(&mut self, geometry: RoamGeometry) {
        let Some(roam_window) = self.roam_window else {
            return;
        };
        if self.roam_geometry.width != geometry.width
            || self.roam_geometry.height != geometry.height
        {
            let _ = roam_window.request_inner_size(WindowSize::new(geometry.width, geometry.height));
            if let (Some(renderer), Some(target)) = (self.renderer.as_mut(), self.roam_target) {
                let _ = renderer.resize(target, geometry.width, geometry.height);
            }
        }
        window().set_position(roam_window.id(), WindowPosition::new(geometry.x, geometry.y));
        self.roam_geometry = geometry;
    }

    pub(super) fn destroy_roam_window(&mut self) {
        self.roam_target = None;
        if let Some(roam_window) = self.roam_window.take() {
            window().destroy_window(roam_window.id());
        }
        self.roam_geometry = RoamGeometry::default();
    }

    pub(super) fn is_roam_window(&self, id: winisland_platform::WindowId) -> bool {
        self.roam_window.is_some_and(|roam_window| roam_window.id() == id)
    }

    /// Events delivered to the playground window.
    pub(super) fn on_roam_window_event(&mut self, event: PlatformEvent) {
        match event {
            PlatformEvent::RedrawRequested { .. } => self.draw_roam_window(),
            PlatformEvent::Resized { size, .. } if size.width > 0 && size.height > 0 => {
                if let (Some(renderer), Some(target)) = (self.renderer.as_mut(), self.roam_target) {
                    let _ = renderer.resize(target, size.width, size.height);
                }
            }
            PlatformEvent::MouseInput {
                state: InputState::Pressed,
                button: MouseButton::Left,
                ..
            } => roam::poked(),
            _ => {}
        }
    }

    fn draw_roam_window(&mut self) {
        let (Some(target), Some(mut renderer)) = (self.roam_target, self.renderer.take()) else {
            return;
        };
        let geometry = self.roam_geometry;
        let result = renderer.frame(target, |_, painter| {
            roam::draw(
                painter,
                geometry.width as f32,
                geometry.height as f32,
                geometry.scale,
            );
        });
        self.renderer = Some(renderer);
        if let Err(error) = result {
            log::warn!("Robot playground frame failed: {error}");
        }
    }
}
