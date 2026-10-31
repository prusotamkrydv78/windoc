use std::time::Instant;

use crate::ui::expanded::music_view::{
    get_cover_rect, get_next_btn_rect, get_pause_btn_rect, get_prev_btn_rect,
    get_progress_bar_rect, trigger_cover_flip, trigger_next_click, trigger_pause_click,
    trigger_prev_click,
};
use crate::ui::widget::expanded::{widget_corner_radius, widget_grid_layout};
use crate::utils::mouse::{
    double_click_interval, is_point_in_continuous_rounded_rect, is_point_in_rect,
};
use winisland_core::config::{MIN_HIDDEN_WIDTH, WidgetKind};
use winisland_platform::InputState;

use super::{App, DragAxis, IslandLayout};
use crate::ui::expanded::pager::ExpandedPage;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum InputSource {
    Mouse,
    Touch,
}

impl App {
    pub(super) fn handle_input(
        &mut self,
        state: InputState,
        px: i32,
        py: i32,
        source: InputSource,
    ) {
        let fullscreen_suppressed = self.fullscreen_hide_active() && !self.hide.overlay_reveal;
        if fullscreen_suppressed || (source == InputSource::Mouse && self.is_cursor_suppressed) {
            return;
        }
        let rel_x = px - self.geom.win_x;
        let rel_y = py - self.geom.win_y;
        // An activity's action button (Open, Settings…) acts on press.
        if state == InputState::Pressed
            && !self.expanded
            && let Some(target) = self
                .compact_overlay
                .activity_action_at(winisland_render::Point::new(rel_x as f32, rel_y as f32))
        {
            if target == super::frame::CALL_MUTE_ACTION {
                self.toggle_call_mute();
            } else if target == super::frame::CALL_CAMERA_ACTION {
                self.send_call_shortcut(false);
            } else if target == super::frame::CALL_LEAVE_ACTION {
                self.send_call_shortcut(true);
            } else {
                let _ = crate::platform::shell().open_url(&target);
            }
            return;
        }
        let layout = self.compute_island_layout();

        if state == InputState::Pressed {
            self.handle_press(rel_x, rel_y, &layout);
        } else if state == InputState::Released {
            self.update_volume_drag_position(rel_x, &layout);
            self.update_brightness_drag_position(rel_x, &layout);
            self.handle_release(px, py);
        }
    }

    pub(super) fn handle_right_input(&mut self, state: InputState, px: i32, py: i32) {
        if !self.config.right_click_drag
            || self.expanded
            || self.is_cursor_suppressed
            || (self.is_fullscreen_suppressed && self.is_hidden())
        {
            return;
        }
        match state {
            InputState::Pressed => {
                let rel_x = px - self.geom.win_x;
                let rel_y = py - self.geom.win_y;
                let layout = self.compute_island_layout();
                let is_hovering = is_point_in_continuous_rounded_rect(
                    rel_x as f64,
                    rel_y as f64,
                    layout.current_island_x,
                    layout.current_island_y,
                    self.springs.w.value as f64,
                    self.springs.h.value as f64,
                    self.springs.r.value as f64,
                );
                if is_hovering {
                    self.right_press_cursor = Some((px, py));
                    self.right_drag_start_offset = Some((
                        self.config.position_x_offset + self.geom.win_x - self.geom.configured_x,
                        self.config.position_y_offset + self.geom.win_y - self.geom.configured_y,
                    ));
                }
            }
            InputState::Released => {
                if self.is_right_dragging {
                    self.is_right_dragging = false;
                    crate::core::persistence::save_config(&self.config);
                    log::info!(
                        "Right click drag offsets saved: ({}, {})",
                        self.config.position_x_offset,
                        self.config.position_y_offset
                    );
                }
                self.right_press_cursor = None;
                self.right_drag_start_offset = None;
            }
        }
    }

    pub(super) fn handle_press(&mut self, rel_x: i32, rel_y: i32, layout: &IslandLayout) {
        let island_y = layout.island_y;
        let offset_x = layout.offset_x;
        let current_island_x = layout.current_island_x;
        let current_island_y = layout.current_island_y;
        let is_hovering_visible = is_point_in_continuous_rounded_rect(
            rel_x as f64,
            rel_y as f64,
            current_island_x,
            current_island_y,
            self.springs.w.value as f64,
            self.springs.h.value as f64,
            self.springs.r.value as f64,
        ) || self.pager_contains(rel_x, rel_y, layout);
        let is_on_hidden_reveal = self.is_hidden()
            && self.config.hidden_width <= MIN_HIDDEN_WIDTH
            && self.springs.hide.value >= 0.999
            && is_point_in_rect(
                rel_x as f64,
                rel_y as f64,
                layout.hidden_reveal_x,
                layout.hidden_reveal_y,
                layout.hidden_reveal_w,
                layout.hidden_reveal_h,
            );

        if !self.expanded
            && is_hovering_visible
            && self.compact_overlay.begin_volume_drag(
                rel_x as f32,
                rel_y as f32,
                winisland_render::Rect::from_xywh(
                    current_island_x as f32,
                    current_island_y as f32,
                    self.springs.w.value,
                    self.springs.h.value,
                ),
                self.config.compact_scale,
            )
        {
            self.idle_timer = Instant::now();
            return;
        }

        if !self.expanded
            && is_hovering_visible
            && self.compact_overlay.begin_brightness_drag(
                rel_x as f32,
                rel_y as f32,
                winisland_render::Rect::from_xywh(
                    current_island_x as f32,
                    current_island_y as f32,
                    self.springs.w.value,
                    self.springs.h.value,
                ),
                self.config.compact_scale,
            )
        {
            self.idle_timer = Instant::now();
            return;
        }

        if !(self.expanded || self.fullscreen_hide_active())
            && self.compact_overlay.is_notification_visible()
            && is_hovering_visible
        {
            self.dismissing_notification = true;
            self.is_dragging = true;
            self.drag_start_px = rel_x + self.geom.win_x;
            self.drag_start_py = rel_y + self.geom.win_y;
            self.drag_has_moved = false;
            self.drag_axis = None;
            return;
        }

        if self.fullscreen_hide_active() {
            return;
        }

        if self.expanded {
            self.expanded_press_started_inside = true;
            self.expanded_header_press = None;
            if self.handle_pager_press(rel_x, rel_y, layout) {
                return;
            }
            let w = self.springs.w.value as f64;
            let h = (self.expanded_content_height() * self.config.expanded_scale)
                .min(self.springs.h.value) as f64;
            let scale = self.config.expanded_scale as f64;

            if self.page_focused(ExpandedPage::Music) {
                let media = self.current_media_info().clone();
                let music_on = !media.title.is_empty()
                    && (self.plugin_media_source.is_some() || self.config.smtc_enabled);
                let cx = rel_x as f32 - self.page_translation(ExpandedPage::Music);
                let cy = rel_y as f32;
                let (cover_x, cover_y, cover_w, cover_h) =
                    get_cover_rect(offset_x as f32, island_y as f32, self.config.expanded_scale);
                if music_on
                    && !media.source_app_id.is_empty()
                    && cx >= cover_x
                    && cx <= cover_x + cover_w
                    && cy >= cover_y
                    && cy <= cover_y + cover_h
                {
                    if self
                        .cover_click
                        .register((cx, cy), Instant::now(), double_click_interval())
                    {
                        if crate::platform::shell()
                            .activate_media_app(&media.source_app_id)
                            .unwrap_or_else(|error| {
                                log::warn!("Media application activation failed: {error}");
                                false
                            })
                        {
                            log::info!("Media application activated: {}", media.source_app_id);
                        } else {
                            log::warn!(
                                "Media application could not be activated: {}",
                                media.source_app_id
                            );
                        }
                    }
                    return;
                }
                self.cover_click.reset();

                let (bx, by, bw, bh) = get_pause_btn_rect(
                    offset_x as f32,
                    island_y as f32,
                    w as f32,
                    self.config.expanded_scale,
                );
                if music_on
                    && self.media_control_available(
                        winisland_plugin_api::types::v2::context::MEDIA_CONTROL_TOGGLE_PLAY,
                    )
                    && cx >= bx
                    && cx <= bx + bw
                    && cy >= by
                    && cy <= by + bh
                {
                    trigger_pause_click(media.is_playing);
                    self.dispatch_media_command(
                        winisland_plugin_api::types::v2::context::MEDIA_COMMAND_TOGGLE_PLAY,
                        0,
                    );
                    return;
                }

                let (px, py, pw, ph) = get_prev_btn_rect(
                    offset_x as f32,
                    island_y as f32,
                    w as f32,
                    self.config.expanded_scale,
                );
                if music_on
                    && self.media_control_available(
                        winisland_plugin_api::types::v2::context::MEDIA_CONTROL_PREVIOUS,
                    )
                    && cx >= px
                    && cx <= px + pw
                    && cy >= py
                    && cy <= py + ph
                {
                    trigger_cover_flip();
                    trigger_prev_click();
                    self.dispatch_media_command(
                        winisland_plugin_api::types::v2::context::MEDIA_COMMAND_PREVIOUS,
                        0,
                    );
                    return;
                }

                let (nx, ny, nw, nh) = get_next_btn_rect(
                    offset_x as f32,
                    island_y as f32,
                    w as f32,
                    self.config.expanded_scale,
                );
                if music_on
                    && self.media_control_available(
                        winisland_plugin_api::types::v2::context::MEDIA_CONTROL_NEXT,
                    )
                    && cx >= nx
                    && cx <= nx + nw
                    && cy >= ny
                    && cy <= ny + nh
                {
                    trigger_cover_flip();
                    trigger_next_click();
                    self.dispatch_media_command(
                        winisland_plugin_api::types::v2::context::MEDIA_COMMAND_NEXT,
                        0,
                    );
                    return;
                }

                if let Some((bar_left, bar_right, bar_top, bar_hit_h)) = get_progress_bar_rect(
                    offset_x as f32,
                    island_y as f32,
                    w as f32,
                    music_on,
                    self.config.expanded_scale,
                ) && self.media_control_available(
                    winisland_plugin_api::types::v2::context::MEDIA_CONTROL_SEEK,
                ) && cx >= bar_left
                    && cx <= bar_right
                    && cy >= bar_top
                    && cy <= bar_top + bar_hit_h
                {
                    let ratio = ((cx - bar_left) / (bar_right - bar_left)).clamp(0.0, 1.0);
                    let duration_ms = media.effective_duration_ms();
                    let seek_ms = (ratio as f64 * duration_ms as f64) as u64;
                    self.seek.begin(
                        bar_left,
                        bar_right,
                        duration_ms,
                        seek_ms,
                        self.plugin_media_source
                            .as_ref()
                            .map(|source| source.resource_id),
                    );
                    return;
                }
            }

            if self.page_focused(ExpandedPage::Widgets) {
                let widget_shift = self.page_translation(ExpandedPage::Widgets) as f64;
                let settings_hit = self
                    .config
                    .widget_layout
                    .iter()
                    .find(|entry| entry.widget == Some(WidgetKind::Settings))
                    .is_some_and(|entry| {
                        let layout = widget_grid_layout(
                            offset_x as f32,
                            island_y as f32,
                            w as f32,
                            h as f32,
                            self.config.expanded_scale,
                        );
                        let (x, y, width, height) =
                            layout.footprint_rect(WidgetKind::Settings, entry.slot);
                        is_point_in_continuous_rounded_rect(
                            rel_x as f64,
                            rel_y as f64,
                            x as f64 + widget_shift,
                            y as f64,
                            width as f64,
                            height as f64,
                            widget_corner_radius(width, height, self.config.expanded_scale) as f64,
                        )
                    });
                if settings_hit {
                    self.open_settings();
                    return;
                }
            }

            if self.page_focused(ExpandedPage::Home)
                && let Some(hit) = crate::ui::dashboard::hit_test(
                    offset_x as f32 + self.page_translation(ExpandedPage::Home),
                    island_y as f32,
                    w as f32,
                    h as f32,
                    self.config.expanded_scale,
                    winisland_render::Point::new(rel_x as f32, rel_y as f32),
                )
            {
                crate::ui::dashboard::apply(hit);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                return;
            }
            if crate::ui::dashboard::palette::is_open() {
                // A click outside the search field takes focus away; the query is kept.
                crate::ui::dashboard::palette::hide();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }

            if self.page_focused(ExpandedPage::Calendar)
                && let Some(action) = crate::ui::expanded::calendar_view::hit_test(
                    offset_x as f32 + self.page_translation(ExpandedPage::Calendar),
                    island_y as f32,
                    w as f32,
                    h as f32,
                    self.config.expanded_scale,
                    winisland_render::Point::new(rel_x as f32, rel_y as f32),
                )
            {
                crate::ui::expanded::calendar_view::apply_action(action);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                return;
            }

            if (rel_y as f64) < island_y + 40.0 * scale {
                self.expanded_header_press =
                    Some((rel_x + self.geom.win_x, rel_y + self.geom.win_y));
            }
        } else if is_hovering_visible || is_on_hidden_reveal {
            if self.is_hidden() && !self.components_hidden {
                self.reveal_island();
                return;
            }
            self.is_dragging = true;
            self.drag_start_px = rel_x + self.geom.win_x;
            self.drag_start_py = rel_y + self.geom.win_y;
            self.drag_start_hide_val = self.springs.hide.value;
            self.drag_has_moved = false;
            self.drag_axis = None;
        }
    }

    pub(super) fn handle_release(&mut self, px: i32, py: i32) {
        self.expanded_press_started_inside = false;
        let expanded_header_press = self.expanded_header_press.take();
        if self.compact_overlay.finish_volume_drag() {
            return;
        }
        if self.compact_overlay.finish_brightness_drag() {
            return;
        }
        if self.finish_seek() {
            return;
        }
        if let Some((start_x, start_y)) = expanded_header_press {
            let threshold = (10.0 * self.config.expanded_scale).max(8.0) as i32;
            if self.expanded
                && (px - start_x).abs() <= threshold
                && (py - start_y).abs() <= threshold
            {
                self.expanded = false;
                self.reset_page();
            }
            return;
        }
        if self.dismissing_notification {
            self.dismissing_notification = false;
            self.is_dragging = false;
            self.drag_axis = None;
            if self.drag_start_py - py > 20 {
                self.compact_overlay.dismiss_notification();
            } else if !self.compact_overlay.activate_notification() {
                self.expand();
            }
            return;
        }
        if self.is_dragging {
            self.is_dragging = false;
            let dx = px - self.drag_start_px;
            let dy = py - self.drag_start_py;
            let drag_axis = self.drag_axis.take();
            if !self.expanded
                && drag_axis != Some(DragAxis::Vertical)
                && dx.abs() >= 40
                && dx.abs() > dy.abs() * 2
            {
                self.components_hidden = !self.components_hidden;
                if self.is_hidden() {
                    self.reveal_island();
                }
                self.idle_timer = Instant::now();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                return;
            }
            if drag_axis == Some(DragAxis::Horizontal) {
                return;
            }
            if !self.drag_has_moved
                || (self.components_hidden
                    && drag_axis.is_none()
                    && dx.abs() <= 10
                    && dy.abs() <= 10)
            {
                if self.is_hidden() {
                    self.reveal_island();
                } else {
                    // With hover-to-open the panel opens on its own, so a click on the resting
                    // robot makes it hop instead.
                    let robot_resting = self.config.hover_to_open
                        && !self.current_media_info().is_playing
                        && self.config.compact_widget_layout.iter().any(|slot| {
                            slot.widget == Some(winisland_core::config::CompactWidgetKind::Mascot)
                        });
                    if robot_resting && !self.expanded {
                        crate::ui::widget::compact::mascot::react(
                            crate::ui::widget::compact::mascot::Reaction::Jump,
                        );
                    } else {
                        self.expand();
                    }
                }
            } else if self.springs.hide.value > 0.3 {
                self.hide.manual = true;
                self.hide.auto = false;
                self.hide.fullscreen = false;
            } else {
                self.hide.manual = false;
                self.hide.auto = false;
                self.hide.fullscreen = false;
            }
        }
    }

    pub(super) fn update_volume_drag_position(&mut self, rel_x: i32, layout: &IslandLayout) {
        self.compact_overlay.update_volume_drag(
            rel_x as f32,
            winisland_render::Rect::from_xywh(
                layout.current_island_x as f32,
                layout.current_island_y as f32,
                self.springs.w.value,
                self.springs.h.value,
            ),
            self.config.compact_scale,
        );
    }

    pub(super) fn update_brightness_drag_position(&mut self, rel_x: i32, layout: &IslandLayout) {
        self.compact_overlay.update_brightness_drag(
            rel_x as f32,
            winisland_render::Rect::from_xywh(
                layout.current_island_x as f32,
                layout.current_island_y as f32,
                self.springs.w.value,
                self.springs.h.value,
            ),
            self.config.compact_scale,
        );
    }

    pub(super) fn expand(&mut self) {
        let compact_height = self.compact_content_height();
        let interrupts_collapse = self.springs.h.value - compact_height
            > 0.5 * self.config.compact_scale
            || self.springs.h.velocity.abs() > 0.001;
        self.reset_page();
        if !interrupts_collapse {
            self.snap_to_current_page();
        }
        self.expanded = true;
    }
}

impl App {
    /// Middle-clicking the collapsed notch plays or pauses the current media.
    pub(super) fn handle_middle_click(&mut self, px: i32, py: i32) {
        if self.expanded || self.is_hidden() {
            return;
        }
        let layout = self.compute_island_layout();
        let over_island = is_point_in_continuous_rounded_rect(
            (px - self.geom.win_x) as f64,
            (py - self.geom.win_y) as f64,
            layout.current_island_x,
            layout.current_island_y,
            self.springs.w.value as f64,
            self.springs.h.value as f64,
            self.springs.r.value as f64,
        );
        if over_island && !self.toggle_call_mute() {
            self.smtc.request_toggle_play();
        }
    }
}

/// Opens or closes a bar panel anchored under the clicked item.
fn toggle_popover(popover: crate::ui::top_bar::popover::Popover, hit: crate::ui::top_bar::BarHit) {
    let anchor = crate::ui::top_bar::target_rect(hit).map_or(0.0, |rect| rect.right);
    crate::ui::top_bar::popover::toggle(popover, anchor);
}

impl App {
    /// Handles a press while a bar panel is open: acts on its controls, keeps it open for
    /// clicks inside, and closes it for clicks anywhere else except the bar itself (whose
    /// buttons toggle panels). Returns `true` when the click was consumed.
    pub(super) fn handle_popover_click(&mut self, rel_x: i32, rel_y: i32) -> bool {
        use crate::ui::top_bar::popover::{self, PopHit};
        if !popover::is_open() {
            return false;
        }
        let point = winisland_render::Point::new(rel_x as f32, rel_y as f32);
        let redraw = |app: &Self| app.request_bar_redraw();
        match popover::hit(point) {
            Some(PopHit::Settings(uri)) => {
                let _ = crate::platform::shell().open_url(uri);
                popover::close();
            }
            Some(PopHit::Volume(level)) => {
                std::thread::spawn(move || {
                    let _ = crate::platform::audio().set_volume(level);
                });
                self.refresh_popover_data(Some(level));
            }
            Some(PopHit::PlayPause) => self.smtc.request_toggle_play(),
            Some(PopHit::WindowsNotifications) => {
                use winisland_platform_windows::shell_bar;
                shell_bar::press_shortcut(&[shell_bar::VK_WIN, shell_bar::VK_N]);
                popover::close();
            }
            Some(PopHit::Panel) => {}
            None => {
                if rel_y as f32 >= self.bar_top() {
                    return false; // let the bar button toggle panels
                }
                popover::close();
            }
        }
        redraw(self);
        true
    }
}

impl App {
    /// Right-clicking the top bar opens its context menu (Task Manager, Taskbar settings,
    /// Show desktop, WinIsland settings), like the classic taskbar. Returns `true` when the
    /// click was on the bar.
    pub(super) fn show_top_bar_menu(&mut self, px: i32, py: i32) -> bool {
        use winisland_platform_windows::shell_bar::{self, BarMenuCommand};
        let Some(window) = self.bar_window else {
            return false;
        };
        let Some(surface) = crate::platform::window().native_surface(window.id()) else {
            return false;
        };
        let tr = winisland_core::i18n::tr;
        let labels = [
            tr("bar_menu_task_manager"),
            tr("bar_menu_taskbar_settings"),
            tr("bar_menu_show_desktop"),
            tr("bar_menu_app_settings"),
        ];
        let chosen = shell_bar::show_bar_menu(
            surface.handle() as isize,
            px,
            py,
            [&labels[0], &labels[1], &labels[2], &labels[3]],
        );
        match chosen {
            Some(BarMenuCommand::TaskManager) => {
                const CTRL: u16 = 0x11;
                const SHIFT: u16 = 0x10;
                const ESCAPE: u16 = 0x1B;
                shell_bar::press_shortcut(&[CTRL, SHIFT, ESCAPE]);
            }
            Some(BarMenuCommand::TaskbarSettings) => {
                let _ = crate::platform::shell().open_url("ms-settings:taskbar");
            }
            Some(BarMenuCommand::ShowDesktop) => shell_bar::toggle_desktop(),
            Some(BarMenuCommand::AppSettings) => self.open_settings(),
            None => {}
        }
        true
    }
}

impl App {
    /// Acts on a press on a top bar item, like the taskbar (on press, not release).
    pub(super) fn press_top_bar_item(&mut self, hit: crate::ui::top_bar::BarHit) {
        use winisland_platform_windows::shell_bar;
        match hit {
            crate::ui::top_bar::BarHit::Start => shell_bar::open_start_menu(),
            crate::ui::top_bar::BarHit::ShowDesktop => shell_bar::toggle_desktop(),
            crate::ui::top_bar::BarHit::Search => {
                if self.is_hidden() {
                    self.reveal_island();
                }
                self.expanded = true;
                crate::ui::dashboard::show(crate::ui::dashboard::Destination::Home);
                self.current_page = crate::ui::expanded::pager::ExpandedPage::Home;
                self.snap_to_current_page();
                crate::ui::dashboard::palette::open();
            }
            crate::ui::top_bar::BarHit::App(handle) => shell_bar::activate_window(handle),
            crate::ui::top_bar::BarHit::Clock => {
                toggle_popover(crate::ui::top_bar::popover::Popover::Calendar, hit)
            }
            crate::ui::top_bar::BarHit::Tray => {
                // Briefly show the taskbar so its overflow can open, then hide it again
                // on the next watchdog pass.
                shell_bar::show_taskbar_windows();
                if let Some(session) = self.top_bar.as_mut() {
                    session.pause_watchdog(std::time::Duration::from_secs(8));
                }
                std::thread::spawn(shell_bar::open_tray_overflow);
            }
            crate::ui::top_bar::BarHit::Status => {
                toggle_popover(crate::ui::top_bar::popover::Popover::ControlCentre, hit)
            }
            crate::ui::top_bar::BarHit::Notifications => {
                toggle_popover(crate::ui::top_bar::popover::Popover::Notifications, hit)
            }
        }
        self.refresh_top_bar_apps();
        if crate::ui::top_bar::popover::is_open() {
            self.refresh_popover_data(None);
        }
        self.request_bar_redraw();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
