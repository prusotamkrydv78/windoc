use std::time::{Duration, Instant};

use winisland_platform::WindowPosition;

use crate::platform::WindowRef;
use crate::ui::compact::CompactOverlayState;
use crate::ui::expanded::music_view::{
    get_progress_bar_rect, set_progress_dragging, set_progress_hover,
};
use crate::ui::expanded::pager::ExpandedPage;
use crate::utils::mouse::{
    double_click_interval, get_global_cursor_pos, is_cursor_hidden, is_foreground_fullscreen,
    is_left_button_pressed, is_point_in_continuous_rounded_rect, is_point_in_rect,
};
use winisland_core::config::MIN_HIDDEN_WIDTH;

use super::{App, DragAxis, RIGHT_DRAG_THRESHOLD};

const INTERACTIVE_FRAME_INTERVAL: Duration = Duration::from_millis(20);
/// Island shape springs: slightly stiffer and livelier than the original 0.10 / 0.68,
/// so expanding and collapsing feel quicker with a gentle overshoot.
const SHAPE_STIFFNESS: f32 = 0.12;
const SHAPE_DAMPING: f32 = 0.71;
/// Page swipe spring.
const PAGE_STIFFNESS: f32 = 0.14;
const PAGE_DAMPING: f32 = 0.70;
const PLAYBACK_FRAME_INTERVAL: Duration = Duration::from_millis(20);
const IDLE_FRAME_INTERVAL: Duration = Duration::from_millis(50);
const SETTLE_PIXELS: f32 = 0.1;
const SETTLE_PIXEL_VELOCITY: f32 = 0.05;
const HIDDEN_FRAME_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const WORKING_SET_TRIM_INTERVAL: Duration = Duration::from_secs(20);
const RENDERER_RECOVERY_INTERVAL: Duration = Duration::from_secs(1);
const LYRIC_TRANSITION_STEP: f32 = 0.05;
const LYRIC_TRANSITION_LEAD_MS: u64 = (1000.0 / (60.0 * LYRIC_TRANSITION_STEP as f64)) as u64;

impl App {
    fn input_pressed(&self) -> bool {
        self.touch_id.is_some()
            || (self
                .last_touch_at
                .is_none_or(|time| time.elapsed() >= Duration::from_millis(250))
                && is_left_button_pressed())
    }
    pub(super) fn on_about_to_wait(&mut self) {
        let window = match self.window {
            Some(w) => w,
            None => return,
        };
        let now = Instant::now();
        if let Some(error) = self
            .renderer
            .as_mut()
            .and_then(winisland_render::Renderer::take_failure)
        {
            self.invalidate_renderer(&error, now);
        }
        self.recover_renderer(&window, now, RENDERER_RECOVERY_INTERVAL);
        if now < self.next_frame_deadline {
            return;
        }
        if let Some(event) = crate::ui::dashboard::poll(now) {
            crate::ui::widget::compact::mascot::focus_finished();
            if !self.expanded && !self.is_hidden() {
                self.compact_overlay
                    .push_activity(crate::ui::compact::Activity {
                        icon: crate::ui::components::icon::Icon::Focus,
                        accent: winisland_render::Rgba::from_argb(255, 255, 159, 10),
                        primary: winisland_core::i18n::tr("focus_notify_title"),
                        secondary: crate::ui::dashboard::phase_finished_message(event),
                        progress: None,
                        actions: Vec::new(),
                        tall: false,
                        priority: crate::ui::compact::Priority::High,
                    });
            }
            window.request_redraw();
        }
        self.screen_time
            .apply_settings(crate::core::screen_time::settings_for(&self.config), now);
        self.screen_time.poll(now);
        self.screen_time
            .check_limit(self.config.screen_time_limit_minutes, now);
        if crate::ui::dashboard::showing(crate::ui::dashboard::Destination::ScreenTime) {
            crate::ui::dashboard::set_screen_time_rows(self.screen_time.report_rows(now));
            crate::ui::dashboard::set_screen_time_apps(self.screen_time.today_minutes(now));
        }
        if self.expanded {
            crate::ui::dashboard::set_screen_time_week(self.screen_time.week_minutes(now));
        }
        self.clipboard.apply_settings(
            crate::core::clipboard_history::settings_for(&self.config),
            now,
        );
        self.clipboard.poll(now);
        if let Some((label, level)) = crate::ui::top_bar::take_app_volume()
            && !self.expanded
        {
            self.compact_overlay
                .push_activity(crate::ui::compact::Activity {
                    icon: crate::ui::components::icon::Icon::Music,
                    accent: winisland_render::Rgba::WHITE,
                    primary: label,
                    secondary: format!("{}%", (level * 100.0).round()),
                    progress: Some(level),
                    actions: Vec::new(),
                    tall: false,
                    priority: crate::ui::compact::Priority::High,
                });
        }
        self.poll_battery_activity(now);
        if self.file_drag_hover {
            // Keep the drop hint up for as long as the drag lasts.
            self.push_drop_activity(None);
        }
        if let Some(index) = crate::ui::dashboard::take_clipboard_copy()
            && self.clipboard.copy_again(index)
        {
            self.attention_pulse_started = Some(now);
        }
        if self.clipboard.take_captured() {
            self.attention_pulse_started = Some(now);
            self.push_copied_activity();
            window.request_redraw();
        }
        crate::ui::dashboard::sync_clipboard(
            self.clipboard.enabled(),
            self.clipboard.revision(),
            self.clipboard.items(),
        );
        if self.weather.poll(&self.config, now) {
            window.request_redraw();
        }
        crate::ui::dashboard::set_weather(self.weather.summary(&self.config));
        if crate::ui::dashboard::showing(crate::ui::dashboard::Destination::Weather) {
            crate::ui::dashboard::set_weather_rows(self.weather.detail_rows(&self.config));
            crate::ui::dashboard::set_weather_hourly(
                self.weather.hourly_temperatures(&self.config),
            );
        }
        crate::ui::dashboard::set_world_clocks(&self.config.world_clocks);
        crate::ui::widget::compact::message::set(&self.config.status_message);
        crate::ui::theme::set(&self.config.ui_theme);
        crate::ui::island::set_edge_light(&self.config.edge_light);
        crate::ui::island::set_notch_color(&self.config.notch_color);
        crate::ui::dashboard::set_links(&self.config.links);
        crate::ui::dashboard::set_hidden_modules(&self.config.hidden_modules);
        crate::ui::dashboard::set_workspaces(&self.config.workspaces, self.config.active_workspace);
        if let Some(request) = crate::ui::dashboard::system_page::take_request() {
            // Audio calls block, so they run off the UI thread.
            std::thread::spawn(move || {
                let audio = crate::platform::audio();
                let _ = match request {
                    crate::ui::dashboard::system_page::SystemRequest::Volume(level) => {
                        audio.set_volume(level)
                    }
                    crate::ui::dashboard::system_page::SystemRequest::ToggleMute => {
                        audio.toggle_mute()
                    }
                };
            });
        }
        if let Some((workspaces, active)) = crate::ui::dashboard::take_workspace_change() {
            self.config.workspaces = workspaces;
            self.config.active_workspace = active;
            crate::core::persistence::save_config(&self.config);
        }
        if !self.expanded {
            // A collapsed panel cannot hold keyboard focus; the query is kept for later.
            crate::ui::dashboard::palette::hide();
        }
        match crate::ui::dashboard::take_header_request() {
            Some(true) => self.open_settings(),
            Some(false) => {
                crate::ui::dashboard::palette::hide();
                crate::ui::dashboard::editor::close();
                self.expanded = false;
                self.reset_page();
                window.request_redraw();
            }
            None => {}
        }
        let media = self.current_media_info();
        let has_track = self.media_active() && !media.title.is_empty();
        crate::ui::dashboard::set_now_playing(has_track.then(|| media.title.clone()));
        crate::ui::dashboard::set_media(has_track.then_some(&media));
        if let Some(fraction) = crate::ui::dashboard::take_media_seek() {
            self.smtc
                .request_seek(media.duration_ms * u64::from(fraction) / 1000);
        }
        match crate::ui::dashboard::take_media_request() {
            Some(0) => self.smtc.request_prev(),
            Some(1) => self.smtc.request_toggle_play(),
            Some(_) => self.smtc.request_next(),
            None => {}
        }
        let (water, stretch) = self.wellness.poll(&self.config, now);
        if water {
            crate::ui::widget::compact::mascot::react(
                crate::ui::widget::compact::mascot::Reaction::Water,
            );
        }
        if stretch {
            crate::ui::widget::compact::mascot::react(
                crate::ui::widget::compact::mascot::Reaction::StretchBreak,
            );
        }
        self.poll_image_job();
        self.poll_drop(now);
        self.calendar.poll(&self.config.calendar_feeds, now);
        if crate::ui::dashboard::showing(crate::ui::dashboard::Destination::Agenda) {
            crate::ui::dashboard::sync_agenda(self.calendar.revision(), || {
                self.calendar.agenda_rows()
            });
        }
        if let Some(rates) = self
            .currency
            .poll(crate::ui::dashboard::palette::take_rates_request(), now)
        {
            crate::ui::dashboard::palette::set_rates(rates);
            window.request_redraw();
        }
        crate::ui::dashboard::sync_shelf(
            self.shelf.revision(),
            || self.shelf.active_name(),
            || self.shelf.entries(),
        );
        if crate::ui::dashboard::take_shelf_cycle() {
            self.shelf.cycle();
        }
        self.ai_usage.poll(
            self.config.ai_usage_enabled && !crate::core::safe_mode::active(),
            now,
        );
        crate::ui::dashboard::sync_ai_usage(
            self.config.ai_usage_enabled,
            self.ai_usage.revision(),
            self.ai_usage.report(),
            self.ai_usage.active_sessions(),
        );
        let accessible = if self.expanded && self.page_visible(ExpandedPage::Home) {
            let layout = self.compute_island_layout();
            crate::ui::dashboard::accessible_elements(
                layout.offset_x as f32 + self.page_translation(ExpandedPage::Home),
                layout.island_y as f32,
                self.springs.w.value,
                (self.config.expanded_height * self.config.expanded_scale)
                    .min(self.springs.h.value),
                self.config.expanded_scale,
                (f64::from(self.geom.win_x), f64::from(self.geom.win_y)),
            )
        } else {
            Vec::new()
        };
        winisland_platform_windows::accessibility::publish(accessible);
        if let Some(entry_id) = crate::ui::dashboard::take_shelf_removal() {
            self.shelf.remove(entry_id);
        }
        crate::ui::dashboard::set_screen_time_total(self.config.screen_time_enabled.then(|| {
            self.screen_time
                .today(now)
                .map_or(Duration::ZERO, |day| day.total)
        }));
        if self
            .geom
            .position_restore_after
            .is_some_and(|restore_after| now >= restore_after)
        {
            self.geom.position_restore_after = None;
            self.geom.win_x = self.geom.configured_x;
            self.geom.win_y = self.geom.configured_y;
            window.set_outer_position(WindowPosition::new(self.geom.win_x, self.geom.win_y));
        }
        self.sync_palette_focus();
        if self.topmost_check.due(now) && !self.palette_focused {
            Self::enforce_overlay_window(&window);
        }
        self.handle_tray_events(&window);
        self.handle_hide_hotkey(&window, now);
        self.reload_config_if_changed(&window);
        if self.is_hidden() && !self.can_hide() {
            self.reveal_island();
        }

        self.poll_pending_plugin_install();
        self.poll_pending_plugin_marketplace();
        if let Some(pages) = self.next_v2_settings_pages()
            && let Some(settings) = self.settings.as_mut()
        {
            settings.set_plugin_settings_pages(pages);
        }
        if self.ctx_mgr.tick() {
            window.request_redraw();
        }

        let dt = (self.last_update_time.elapsed().as_secs_f32() * 60.0).clamp(0.1, 6.0);
        self.last_update_time = now;

        if !self.visible {
            self.compact_overlay.finish_volume_drag();
            self.compact_overlay.finish_brightness_drag();
            self.audio.set_gate_override(false);
            self.next_frame_deadline = now + HIDDEN_FRAME_INTERVAL;
            return;
        }
        let (px, py) = if self.touch_id.is_some() {
            (
                (self.touch_pos.x + self.geom.win_x as f64) as i32,
                (self.touch_pos.y + self.geom.win_y as f64) as i32,
            )
        } else {
            get_global_cursor_pos()
        };
        self.update_right_drag(&window, px, py);

        let (music_active, media_is_playing) = self.poll_media_info(&window);

        if self.fullscreen_check.due(now) {
            self.update_fullscreen_suppression(&window, now);
        }

        let rel_x = px - self.geom.win_x;
        let rel_y = py - self.geom.win_y;
        let layout = self.compute_island_layout();
        let island_y = layout.island_y;
        let offset_x = layout.offset_x;
        let current_island_x = layout.current_island_x;
        let current_island_y = layout.current_island_y;
        let is_hovering_island = is_point_in_continuous_rounded_rect(
            rel_x as f64,
            rel_y as f64,
            current_island_x,
            current_island_y,
            self.springs.w.value as f64,
            self.springs.h.value as f64,
            self.springs.r.value as f64,
        ) || self.pager_contains(rel_x, rel_y, &layout);
        let is_over_hidden_reveal = self.is_hidden()
            && (is_hovering_island
                || (self.config.hidden_width <= MIN_HIDDEN_WIDTH
                    && self.springs.hide.value >= 0.999
                    && is_point_in_rect(
                        rel_x as f64,
                        rel_y as f64,
                        layout.hidden_reveal_x,
                        layout.hidden_reveal_y,
                        layout.hidden_reveal_w,
                        layout.hidden_reveal_h,
                    )));
        let cursor_interaction_suppressed = self.is_cursor_suppressed && self.touch_id.is_none();
        let interaction_suppressed = cursor_interaction_suppressed
            || (self.fullscreen_hide_active() && !self.hide.overlay_reveal);
        let is_hovering_visible = !interaction_suppressed && is_hovering_island;
        let is_on_hidden_reveal = !interaction_suppressed
            && is_over_hidden_reveal
            && self.config.hidden_width <= MIN_HIDDEN_WIDTH
            && self.springs.hide.value >= 0.999;

        let passive_reveal_active = self.is_fullscreen_suppressed
            && !self.fullscreen_hide_active()
            && is_over_hidden_reveal;
        if self.hidden_reveal_click.update(
            passive_reveal_active,
            self.input_pressed(),
            (px as f32, py as f32),
            now,
            double_click_interval(),
        ) {
            self.reveal_island();
            window.request_redraw();
            log::info!("Island revealed by fullscreen edge double-click");
        }

        self.update_bar_hover(now);
        if interaction_suppressed {
            window.set_cursor_hittest(false);
        } else {
            window.set_cursor_hittest(
                is_hovering_visible
                    || is_on_hidden_reveal
                    || self.compact_overlay.is_volume_dragging()
                    || self.compact_overlay.is_brightness_dragging(),
            );
        }

        let compact_overlay_visible = self.update_compact_and_auto_hide(
            &window,
            is_hovering_visible,
            music_active,
            media_is_playing,
        );

        if self.compact_overlay.is_volume_dragging() {
            if self.input_pressed() && !interaction_suppressed {
                self.update_volume_drag_position(rel_x, &layout);
            } else {
                self.compact_overlay.finish_volume_drag();
            }
            window.request_redraw();
        }
        if self.compact_overlay.is_brightness_dragging() {
            if self.input_pressed() && !interaction_suppressed {
                self.update_brightness_drag_position(rel_x, &layout);
            } else {
                self.compact_overlay.finish_brightness_drag();
            }
            window.request_redraw();
        }

        self.update_seeking_input(&window, rel_x);
        self.update_progress_hover(rel_x, rel_y, offset_x, island_y, music_active);
        self.update_pager_hover(&window, rel_x, rel_y, &layout, !interaction_suppressed, dt);
        self.update_expand_collapse_click(&window, is_hovering_visible);
        self.update_hide_drag(&window, px, py, dt);

        let is_paused = music_active && !media_is_playing;
        self.update_lyrics(&window, music_active, is_paused, dt);
        self.update_spring_targets(&window, music_active, is_paused, dt);
        self.update_compact_widget_refresh(&window, now);

        self.schedule_next_frame(
            &window,
            now,
            FramePacing {
                media_is_playing,
                is_hovering_visible,
                compact_overlay_visible,
                passive_reveal_active,
            },
        );
    }

    fn poll_pending_plugin_install(&mut self) {
        match self.pending_install.poll() {
            Some(Ok(Ok((manifest, staging)))) => {
                let activation = self
                    .plugin_host
                    .as_ref()
                    .ok_or_else(|| "ABI v2 plugin host is unavailable".to_string())
                    .and_then(|host| {
                        host.activate_staged_plugin(&manifest, &staging)
                            .map_err(|error| error.to_string())
                    });
                if let Err(error) = activation {
                    let _ = std::fs::remove_dir_all(staging);
                    Self::show_toast("Plugin Error", &error);
                    log::error!("Failed to activate installed plugin: {error}");
                    self.set_plugin_install_error(winisland_core::i18n::tr_args(
                        "plugin_install_failed",
                        &[&error],
                    ));
                    return;
                }
                Self::show_toast(
                    "Plugin Installed",
                    &format!("{} loaded successfully!", manifest.name),
                );
                if let Some(settings) = self.settings.as_mut() {
                    settings.finish_marketplace_install();
                    settings.set_plugin_inventory_scan(self.plugin_mgr.installed_plugins_async());
                    settings.set_plugin_status(
                        winisland_core::i18n::tr("plugin_install_success"),
                        false,
                    );
                }
                log::info!("Plugin '{}' installed via drop", manifest.name);
            }
            Some(Ok(Err(e))) => {
                Self::show_toast("Plugin Error", &e);
                self.set_plugin_install_error(winisland_core::i18n::tr_args(
                    "plugin_install_failed",
                    &[&e],
                ));
                log::error!("Failed to install plugin from drop: {e}");
            }
            Some(Err(_)) => {
                Self::show_toast("Plugin Error", "Installation thread crashed");
                log::error!("Plugin installation thread disconnected unexpectedly");
                self.set_plugin_install_error(winisland_core::i18n::tr_args(
                    "plugin_install_failed",
                    &["installation thread crashed"],
                ));
            }
            None => {}
        }
    }

    fn poll_pending_plugin_marketplace(&mut self) {
        match self.pending_marketplace_catalog.poll() {
            Some(Ok(Ok(catalog))) => {
                self.marketplace_catalog = Some(catalog.clone());
                if let Some(settings) = self.settings.as_mut() {
                    settings.set_marketplace_catalog(catalog);
                }
            }
            Some(Ok(Err(error))) => {
                log::error!("Failed to load plugin marketplace: {error}");
                if self.marketplace_catalog.is_none()
                    && let Some(settings) = self.settings.as_mut()
                {
                    settings.set_marketplace_error(error);
                }
            }
            Some(Err(_)) => {
                if self.marketplace_catalog.is_none()
                    && let Some(settings) = self.settings.as_mut()
                {
                    settings
                        .set_marketplace_error("The marketplace task stopped unexpectedly".into());
                }
            }
            None => {}
        }

        match self.pending_marketplace_download.poll() {
            Some(Ok(Ok(path))) => {
                if let Some(settings) = self.settings.as_mut() {
                    settings
                        .set_plugin_status(winisland_core::i18n::tr("plugin_installing"), false);
                }
                self.install_zip_drop(&path);
            }
            Some(Ok(Err(error))) => {
                log::error!("Failed to download marketplace plugin: {error}");
                self.set_plugin_install_error(winisland_core::i18n::tr_args(
                    "plugin_install_failed",
                    &[&error],
                ));
            }
            Some(Err(_)) => {
                self.set_plugin_install_error(winisland_core::i18n::tr_args(
                    "plugin_install_failed",
                    &["the marketplace download task stopped unexpectedly"],
                ));
            }
            None => {}
        }
    }

    fn set_plugin_install_error(&mut self, message: String) {
        if let Some(settings) = self.settings.as_mut() {
            settings.finish_marketplace_install();
            settings.set_plugin_status(message, false);
        }
    }

    fn update_right_drag(&mut self, window: &WindowRef, px: i32, py: i32) {
        let (Some((start_cx, start_cy)), Some((start_ox, start_oy))) =
            (self.right_press_cursor, self.right_drag_start_offset)
        else {
            return;
        };
        let dx = px - start_cx;
        let dy = py - start_cy;
        if !self.is_right_dragging
            && (dx.abs() >= RIGHT_DRAG_THRESHOLD || dy.abs() >= RIGHT_DRAG_THRESHOLD)
        {
            self.is_right_dragging = true;
            log::info!("Right click drag started at offsets: ({start_ox}, {start_oy})");
        }
        if !self.is_right_dragging {
            return;
        }
        self.config.position_x_offset = start_ox + dx;
        self.config.position_y_offset = start_oy + dy;
        if let Some(monitor) = Self::get_target_monitor(window, self.config.monitor_index) {
            let mon_size = monitor.size();
            let mon_pos = monitor.position();
            let (new_x, new_y) = self.compute_window_position(mon_pos, mon_size);
            if new_x != self.geom.configured_x || new_y != self.geom.configured_y {
                self.set_configured_window_position(window, new_x, new_y);
            }
        }
        window.request_redraw();
    }

    fn update_fullscreen_suppression(&mut self, window: &WindowRef, now: Instant) {
        let prev_fullscreen = self.is_fullscreen_suppressed;
        self.is_fullscreen_suppressed = is_foreground_fullscreen(
            self.geom.monitor_pos.0,
            self.geom.monitor_pos.1,
            self.geom.monitor_size.0,
            self.geom.monitor_size.1,
        );
        self.is_cursor_suppressed = is_cursor_hidden();
        if self.is_fullscreen_suppressed && !prev_fullscreen {
            self.fullscreen_hide_paused = false;
        }
        self.apply_fullscreen_hide(window, now);
        if self.is_fullscreen_suppressed != prev_fullscreen {
            log::info!(
                "Fullscreen state: {}",
                if self.is_fullscreen_suppressed {
                    "active"
                } else {
                    "normal"
                }
            );
        }
    }

    fn handle_hide_hotkey(&mut self, window: &WindowRef, now: Instant) {
        if crate::platform::shell()
            .poll_hotkey_presses()
            .is_multiple_of(2)
        {
            return;
        }
        if self.fullscreen_hide_active() {
            self.fullscreen_hide_paused = true;
            log::info!("Fullscreen auto-hide paused by hotkey");
            self.apply_fullscreen_hide(window, now);
        } else if self.fullscreen_hide_paused
            && self.config.fullscreen_auto_hide
            && self.is_fullscreen_suppressed
        {
            self.fullscreen_hide_paused = false;
            log::info!("Fullscreen auto-hide resumed by hotkey");
            self.apply_fullscreen_hide(window, now);
        } else if self.config.hotkey_opens_island && !self.expanded {
            if self.is_hidden() {
                self.reveal_island();
            }
            self.expanded = true;
            crate::ui::dashboard::show(crate::ui::dashboard::Destination::Home);
            self.current_page = crate::ui::expanded::pager::ExpandedPage::Home;
            self.snap_to_current_page();
            log::info!("Island opened by hotkey");
        } else if self.config.hotkey_opens_island {
            crate::ui::dashboard::palette::hide();
            crate::ui::dashboard::editor::close();
            self.expanded = false;
            log::info!("Island closed by hotkey");
        } else if self.is_hidden() {
            self.reveal_island();
            log::info!("Island revealed by hotkey");
        } else if self.prepare_hide(window) {
            self.expanded = false;
            self.reset_page();
            self.hide.manual = true;
            self.hide.auto = false;
            log::info!("Island hidden by hotkey");
        }
        window.request_redraw();
    }

    fn apply_fullscreen_hide(&mut self, window: &WindowRef, now: Instant) {
        let should_hide_for_fullscreen = self.fullscreen_hide_active();
        if should_hide_for_fullscreen != self.hide.fullscreen {
            if should_hide_for_fullscreen {
                self.hide.fullscreen = true;
                let hide_started = if self.hide.origin.is_some() {
                    true
                } else {
                    self.prepare_hide(window)
                };
                if hide_started {
                    self.expanded = false;
                    self.reset_page();
                    self.hide.notification_reveal = false;
                    self.hide.overlay_reveal = false;
                    self.attention_pulse_started = Some(now);
                } else {
                    self.hide.fullscreen = false;
                }
            } else {
                let was_fullscreen_hidden = self.hide.fullscreen;
                self.hide.fullscreen = false;
                self.attention_pulse_started = None;
                self.idle_timer = Instant::now();
                if was_fullscreen_hidden && !self.is_hidden() {
                    self.springs.hide.velocity = -0.65;
                }
            }
            window.request_redraw();
        }
    }

    fn poll_media_info(&mut self, window: &WindowRef) -> (bool, bool) {
        self.smtc
            .set_enabled(self.config.smtc_enabled && crate::platform::capabilities().media_session);
        let smtc_cover_changed = self.smtc.take_info_if_changed().is_some_and(|media| {
            let cover_changed = self.plugin_media_source.is_none()
                && media.thumbnail.is_some()
                && media.thumbnail_hash != self.smtc_media_info.thumbnail_hash;
            self.smtc_media_info = media;
            cover_changed
        });
        self.audio
            .set_enabled(self.config.audio_visualizer_enabled && !crate::core::safe_mode::active());
        self.audio.set_target_app_id(self.audio_target_app_id());
        let music_active = self.media_active();
        let media_is_playing = music_active && self.current_media_info().is_playing;
        let music_became_available = music_active && !self.music_page_available;
        self.set_music_page_available(music_active);
        if music_became_available && self.expanded {
            self.current_page = ExpandedPage::Music;
        }
        let media = self.current_media_info();
        let title = media.title.clone();
        let artist = media.artist.clone();
        let album = media.album.clone();
        if !music_active {
            self.audio.set_gate_override(false);
            if !self.last_media_title.is_empty() {
                self.last_media_title.clear();
                crate::ui::expanded::music_view::clear_cover_cache();
                crate::utils::backdrop::clear_blurred_cover_cache();
            }
        }
        let track_changed = music_active && title != self.last_media_title;
        if track_changed {
            log::info!("Track changed: {title} - {artist} / {album}");
            self.last_media_title = title;
            crate::ui::expanded::music_view::trigger_cover_flip();
            window.request_redraw();
        } else if smtc_cover_changed {
            log::info!("SMTC: late thumbnail applied");
            crate::ui::expanded::music_view::trigger_cover_flip();
            window.request_redraw();
        }
        (music_active, media_is_playing)
    }

    fn update_compact_and_auto_hide(
        &mut self,
        window: &WindowRef,
        is_hovering_visible: bool,
        music_active: bool,
        media_is_playing: bool,
    ) -> bool {
        let is_paused_idle = music_active && !media_is_playing;
        let overlay_present = !self.expanded && !self.is_hidden();
        let fullscreen_hidden = self.fullscreen_hide_active();
        let volume_state = if overlay_present && (!fullscreen_hidden || self.hide.overlay_reveal) {
            CompactOverlayState::Present
        } else if fullscreen_hidden || (self.hide.auto && !self.hide.manual) {
            CompactOverlayState::Defer
        } else {
            CompactOverlayState::Discard
        };
        let notification_state = if overlay_present && !fullscreen_hidden {
            CompactOverlayState::Present
        } else if !self.expanded && self.hide.has_hidden_reason() {
            CompactOverlayState::Defer
        } else {
            CompactOverlayState::Discard
        };
        let compact_update = self.compact_overlay.update(
            volume_state,
            notification_state,
            self.config.notification_display,
        );
        if compact_update.notification_received
            && !fullscreen_hidden
            && self.hide.has_hidden_reason()
            && !self.hide.notification_reveal
        {
            self.hide.notification_reveal = true;
            self.idle_timer = Instant::now();
            self.springs.hide.velocity = -0.65;
            self.compact_overlay.update(
                CompactOverlayState::Present,
                CompactOverlayState::Present,
                self.config.notification_display,
            );
            log::info!("Island temporarily revealed for notification");
        } else if (compact_update.volume_changed || compact_update.brightness_changed)
            && (fullscreen_hidden || (self.hide.auto && !self.hide.manual))
        {
            let reveal_started = if fullscreen_hidden {
                !self.hide.overlay_reveal
            } else {
                self.hide.auto
            };
            if fullscreen_hidden {
                self.hide.overlay_reveal = true;
            } else if self.hide.auto && !self.hide.manual {
                self.hide.auto = false;
            }
            self.idle_timer = Instant::now();
            if reveal_started {
                self.springs.hide.velocity = -0.65;
            }
            self.compact_overlay.update(
                CompactOverlayState::Present,
                CompactOverlayState::Present,
                self.config.notification_display,
            );
            log::info!("Island un-hidden (compact overlay event)");
        }
        if self.hide.notification_reveal && !self.compact_overlay.is_notification_visible() {
            self.hide.notification_reveal = false;
            if self.hide.has_hidden_reason() {
                self.prepare_hide(window);
            }
            window.request_redraw();
        }
        let compact_overlay_visible = self.compact_overlay.is_visible();
        if self.hide.overlay_reveal && !compact_overlay_visible {
            self.hide.overlay_reveal = false;
            if self.hide.has_hidden_reason() {
                self.prepare_hide(window);
            }
            window.request_redraw();
        }
        let has_widgets = !self.components_hidden
            && (self
                .compact_layout()
                .iter()
                .any(|entry| entry.widget.is_some())
                || matches!(
                    self.ctx_mgr.current_mini(),
                    Some(winisland_core::context::MiniContent::Plugin(_))
                ));
        let is_idle = (!is_hovering_visible || self.components_hidden)
            && !self.expanded
            && !self.is_dragging
            && !compact_overlay_visible
            && (self.components_hidden || !music_active || is_paused_idle)
            && !has_widgets;
        if !self.config.auto_hide {
            let was_auto_hidden = self.hide.auto;
            self.hide.auto = false;
            self.idle_timer = Instant::now();
            if was_auto_hidden && !self.is_hidden() {
                self.springs.hide.velocity = -0.65;
            }
        } else if media_is_playing && !self.components_hidden && self.hide.auto && !self.hide.manual
        {
            self.hide.auto = false;
            self.idle_timer = Instant::now();
            if !self.is_hidden() {
                self.springs.hide.velocity = -0.65;
            }
            log::info!("Island un-hidden (media playing)");
        } else if !self.is_hidden() && is_idle {
            if self.idle_timer.elapsed().as_secs_f32() > self.config.auto_hide_delay
                && self.prepare_hide(window)
            {
                self.hide.auto = true;
                log::info!(
                    "Island auto-hidden (idle {:.1}s)",
                    self.config.auto_hide_delay
                );
            }
        } else if !self.is_hidden() && !is_idle {
            self.idle_timer = Instant::now();
        }
        compact_overlay_visible
    }

    fn update_seeking_input(&mut self, window: &WindowRef, rel_x: i32) {
        if self.seek.active && self.input_pressed() {
            let click_x = rel_x as f32 - self.page_translation(ExpandedPage::Music);
            self.seek.preview_at(click_x);
            window.request_redraw();
        } else if self.finish_seek() {
            window.request_redraw();
        }
    }

    fn update_progress_hover(
        &self,
        rel_x: i32,
        rel_y: i32,
        offset_x: f64,
        island_y: f64,
        music_active: bool,
    ) {
        let progress_hover_active = if self.seek.active {
            true
        } else if self.expanded
            && self.page_focused(ExpandedPage::Music)
            && self.media_control_available(
                winisland_plugin_api::types::v2::context::MEDIA_CONTROL_SEEK,
            )
        {
            if let Some((bar_left, bar_right, bar_top, bar_hit_h)) = get_progress_bar_rect(
                offset_x as f32,
                island_y as f32,
                self.springs.w.value,
                music_active,
                self.config.expanded_scale,
            ) {
                let cx = rel_x as f32 - self.page_translation(ExpandedPage::Music);
                let cy = rel_y as f32;
                let margin = 4.0 * self.config.expanded_scale;
                cx >= bar_left - margin
                    && cx <= bar_right + margin
                    && cy >= bar_top - margin
                    && cy <= bar_top + bar_hit_h + margin
            } else {
                false
            }
        } else {
            false
        };
        set_progress_hover(progress_hover_active);
        set_progress_dragging(self.seek.active);
    }

    fn update_hide_drag(&mut self, window: &WindowRef, px: i32, py: i32, dt: f32) {
        if self.is_dragging && !self.dismissing_notification && !self.is_hidden() {
            let upward_distance = self.drag_start_py - py;
            let horizontal_distance = px - self.drag_start_px;
            if upward_distance.abs() > 3 || horizontal_distance.abs() > 3 {
                self.drag_has_moved = true;
            }
            if self.drag_axis.is_none() {
                let horizontal = horizontal_distance.abs();
                let vertical = upward_distance.abs();
                if horizontal >= 10 && horizontal as f32 > vertical as f32 * 1.4 {
                    self.drag_axis = Some(DragAxis::Horizontal);
                } else if vertical >= 24 && vertical as f32 > horizontal as f32 * 1.4 {
                    self.drag_axis = Some(DragAxis::Vertical);
                }
            }
            if self.drag_axis == Some(DragAxis::Vertical)
                && upward_distance > 3
                && self.hide.origin.is_none()
            {
                self.prepare_hide(window);
            }
            if self.drag_axis == Some(DragAxis::Vertical) && self.hide.origin.is_some() {
                let drag_layout = self.compute_island_layout();
                if drag_layout.hide_distance > 0.0 {
                    let mut new_val = self.drag_start_hide_val
                        + (upward_distance as f32 / drag_layout.hide_distance as f32);
                    new_val = new_val.clamp(0.0, 1.0);
                    self.springs.hide.value = new_val;
                    self.springs.hide.velocity = 0.0;
                    window.request_redraw();
                }
            }
        } else {
            let hide_target = if self.is_hidden() { 1.0 } else { 0.0 };
            let (stiffness, damping) = if self.is_hidden() {
                (0.12, 0.70)
            } else {
                (0.08, 0.78)
            };
            self.springs
                .hide
                .update_dt(hide_target, stiffness, damping, dt);
            let hide_span = self.compute_island_layout().hide_distance.max(50.0) as f32;
            self.springs.hide.settle(
                hide_target,
                SETTLE_PIXELS / hide_span,
                SETTLE_PIXEL_VELOCITY / hide_span,
            );
        }
        if !(self.is_hidden() || self.hide.fullscreen && self.hide.overlay_reveal) {
            self.restore_hide_origin(window);
        }
        if self.springs.hide.velocity.abs() > 0.001
            || (self.springs.hide.value > 0.0 && self.springs.hide.value < 1.0)
        {
            window.request_redraw();
        }
    }

    fn update_expand_collapse_click(&mut self, window: &WindowRef, is_hovering_visible: bool) {
        if self.fullscreen_hide_active() {
            return;
        }
        let pressing = self.input_pressed();
        if !pressing {
            self.expanded_press_started_inside = false;
            self.expanded_header_press = None;
        }
        if self.expanded && !is_hovering_visible && pressing && !self.expanded_press_started_inside
        {
            self.expanded = false;
            self.reset_page();
            window.request_redraw();
        }
        if !self.expanded && is_hovering_visible && pressing {
            self.idle_timer = Instant::now();
        }
        self.update_hover_open(window, is_hovering_visible, pressing);
    }

    /// Opens the island after a short rest on it and closes a hover-opened island once the
    /// cursor has left for a moment. Clicks and typing keep their usual behaviour.
    fn update_hover_open(&mut self, window: &WindowRef, hovering: bool, pressing: bool) {
        // Open quickly once the pointer settles on the island, instantly when it is pushed
        // against the top screen edge, and never while it is just passing through.
        const OPEN_DELAY: Duration = Duration::from_millis(150);
        const CLOSE_DELAY: Duration = Duration::from_millis(450);
        const SETTLED_SPEED: f32 = 0.3; // px per ms
        let now = Instant::now();
        let cursor = crate::utils::mouse::get_global_cursor_pos();
        let speed = match self.hover_last_cursor {
            Some((last, at)) => {
                let ms = now.duration_since(at).as_secs_f32() * 1000.0;
                let distance = ((cursor.0 - last.0) as f32).hypot((cursor.1 - last.1) as f32);
                if ms > 0.0 { distance / ms } else { 0.0 }
            }
            None => 0.0,
        };
        self.hover_speed = self.hover_speed * 0.6 + speed * 0.4;
        self.hover_last_cursor = Some((cursor, now));
        let at_top_edge = cursor.1 <= self.geom.monitor_pos.1 + 1;
        if hovering {
            crate::ui::island::set_idle_dimmed(false);
        }
        // An activity with buttons stays collapsed under the pointer so they can be clicked.
        let holding_activity = self
            .compact_overlay
            .set_activity_hovered(hovering && !self.expanded);
        if !self.config.hover_to_open || self.is_hidden() {
            self.hover_open_since = None;
            self.hover_leave_since = None;
            return;
        }
        if !self.expanded {
            self.hover_opened = false;
            self.hover_leave_since = None;
            if holding_activity {
                self.hover_open_since = None;
            } else if hovering && !pressing && !at_top_edge && self.hover_speed > SETTLED_SPEED {
                // Passing through: restart the dwell.
                self.hover_open_since = None;
            } else if hovering && !pressing {
                let since = *self.hover_open_since.get_or_insert(now);
                if at_top_edge || now.duration_since(since) >= OPEN_DELAY {
                    self.hover_open_since = None;
                    self.expand();
                    self.hover_opened = true;
                    window.request_redraw();
                }
            } else {
                self.hover_open_since = None;
            }
            return;
        }
        self.hover_open_since = None;
        if !self.hover_opened || hovering || crate::ui::dashboard::wants_keyboard() {
            self.hover_leave_since = None;
            return;
        }
        let left_at = *self.hover_leave_since.get_or_insert(now);
        if now.duration_since(left_at) >= CLOSE_DELAY {
            self.hover_leave_since = None;
            self.hover_opened = false;
            self.expanded = false;
            self.reset_page();
            window.request_redraw();
        }
    }

    fn update_lyrics(&mut self, window: &WindowRef, music_active: bool, is_paused: bool, dt: f32) {
        let current_lyric = if music_active && self.config.show_lyrics && !is_paused {
            self.current_media_info()
                .current_lyric(
                    (self.config.lyrics_delay * 1000.0) as i64,
                    LYRIC_TRANSITION_LEAD_MS,
                )
                .map(|lyric| {
                    (
                        lyric.text.to_owned(),
                        lyric.secondary_text.unwrap_or_default().to_owned(),
                        lyric.highlight,
                        lyric.started,
                    )
                })
        } else {
            None
        };
        if let Some((lyric, secondary_lyric, highlight, started)) = current_lyric {
            if lyric != self.lyrics.current_text
                || secondary_lyric != self.lyrics.current_secondary_text
            {
                self.lyrics.transition_to(
                    lyric,
                    secondary_lyric,
                    highlight,
                    started,
                    self.config.lyrics_transition_animation,
                );
                window.request_redraw();
            } else if highlight != self.lyrics.highlight {
                self.lyrics.highlight = highlight;
                window.request_redraw();
            }
        } else if !is_paused && !self.lyrics.current_text.is_empty() {
            self.lyrics.transition_to(
                String::new(),
                String::new(),
                None,
                false,
                self.config.lyrics_transition_animation,
            );
        }

        if self.lyrics.transition < 1.0 {
            self.lyrics.transition += LYRIC_TRANSITION_STEP * dt;
            if self.lyrics.transition > 1.0 {
                self.lyrics.transition = 1.0;
            }
            window.request_redraw();
        }
        if self.lyrics.transition >= 1.0 && !self.lyrics.old_text.is_empty() {
            self.lyrics.old_text = String::new();
            self.lyrics.old_secondary_text = String::new();
        }
    }

    fn update_spring_targets(
        &mut self,
        window: &WindowRef,
        music_active: bool,
        is_paused: bool,
        dt: f32,
    ) {
        let was_animating = self.springs.any_animating();
        let compact_components_visible = self.expanded || !self.components_hidden;
        let music_active = music_active && compact_components_visible;
        let lyric_target_w = self.compute_lyric_target_width(window, music_active, is_paused, dt);
        let preserve_compact_widget_width = !self.is_width_hiding()
            || !crate::ui::widget::compact::hide_fade_complete(self.springs.hide.value);
        let compact_widget_target_w = if !self.expanded
            && !self.compact_overlay.is_visible()
            && preserve_compact_widget_width
        {
            let scale = self.config.compact_scale.max(f32::EPSILON);
            let has_mini_content = self.ctx_mgr.current_mini().is_some();
            let compact_layout = self.compact_layout();
            let center_content_width =
                (!self.components_hidden && has_mini_content).then_some(lyric_target_w / scale);
            crate::ui::widget::compact::target_width(
                if self.components_hidden {
                    &[]
                } else {
                    &compact_layout
                },
                self.config.base_width,
                center_content_width,
            ) * scale
        } else {
            lyric_target_w
        };
        let compact_content_h = self.compact_content_height();
        let default_target_h = if self.expanded {
            self.expanded_content_height() * self.config.expanded_scale
        } else {
            compact_content_h
        };
        let default_target_r = if self.expanded {
            crate::utils::shape::expanded_island_radius(
                self.config.expanded_width * self.config.expanded_scale,
                default_target_h,
                self.config.expanded_scale,
            )
        } else {
            self.compact_radius(compact_content_h)
        };
        let (target_w, target_h, target_r) = if let Some(size) = self.compact_overlay.target_size(
            self.config.base_width,
            self.config.base_height,
            self.config.compact_scale,
        ) {
            (size.width, size.height, self.compact_radius(size.height))
        } else {
            (compact_widget_target_w, default_target_h, default_target_r)
        };
        let target_view = self.target_page_position();
        self.springs
            .retarget_expansion(self.expanded, target_w, target_h, target_r, target_view);
        let width_hiding = self.is_width_hiding();
        if self.width_hiding_last_frame && !width_hiding {
            self.restoring_hide_width = true;
        } else if width_hiding {
            self.restoring_hide_width = false;
        }
        self.width_hiding_last_frame = width_hiding;

        if self.restoring_hide_width {
            let progress = 1.0 - 0.78_f32.powf(dt);
            self.springs.w.value += (target_w - self.springs.w.value) * progress;
            self.springs.w.velocity = 0.0;
            if (target_w - self.springs.w.value).abs() <= 0.25 * self.config.compact_scale {
                self.springs.w.value = target_w;
                self.restoring_hide_width = false;
            }
        } else {
            self.springs
                .w
                .update_dt(target_w, SHAPE_STIFFNESS, SHAPE_DAMPING, dt);
            self.springs
                .w
                .settle(target_w, SETTLE_PIXELS, SETTLE_PIXEL_VELOCITY);
        }
        self.springs
            .h
            .update_dt(target_h, SHAPE_STIFFNESS, SHAPE_DAMPING, dt);
        self.springs
            .h
            .settle(target_h, SETTLE_PIXELS, SETTLE_PIXEL_VELOCITY);
        self.springs
            .r
            .update_dt(target_r, SHAPE_STIFFNESS, SHAPE_DAMPING, dt);
        self.springs
            .r
            .settle(target_r, SETTLE_PIXELS, SETTLE_PIXEL_VELOCITY);
        self.springs
            .view
            .update_dt(target_view, PAGE_STIFFNESS, PAGE_DAMPING, dt);
        let page_width = self.springs.w.value.max(1.0);
        self.springs.view.settle(
            target_view,
            SETTLE_PIXELS / page_width,
            SETTLE_PIXEL_VELOCITY / page_width,
        );
        if was_animating && !self.springs.any_animating() {
            window.request_redraw();
        }
    }

    fn update_compact_widget_refresh(&mut self, window: &WindowRef, now: Instant) {
        if self.expanded
            || self.components_hidden
            || self.is_hidden()
            || self.compact_overlay.is_visible()
        {
            self.compact_widget_refresh.clear();
            return;
        }
        let Some(delay) = crate::ui::widget::compact::next_refresh_delay(&self.compact_layout())
        else {
            self.compact_widget_refresh.clear();
            return;
        };
        if self.compact_widget_refresh.is_ready(now) {
            window.request_redraw();
            self.compact_widget_refresh.start(now, delay);
        }
    }

    fn periodic_effect_redraw_due(&mut self) -> bool {
        let is_dynamic = self.config.island_style == "dynamic";
        !self.is_hidden() && (is_dynamic || self.expanded) && self.effect_refresh.due_now()
    }

    fn schedule_next_frame(&mut self, window: &WindowRef, now: Instant, pacing: FramePacing) {
        let should_periodic_redraw = self.periodic_effect_redraw_due();
        let transition_active = self.springs.any_animating()
            || self
                .attention_pulse_started
                .is_some_and(|started| now.duration_since(started) < Duration::from_millis(1600))
            || self.compact_overlay.is_volume_dragging()
            || self.compact_overlay.is_brightness_dragging()
            || self.lyrics.transition < 1.0
            || self.is_dragging
            || self.seek.active
            || self.is_right_dragging
            || (self.expanded
                && self.page_visible(ExpandedPage::Calendar)
                && crate::ui::expanded::calendar_view::is_animating());
        // Live values that change about once a second share the slower refresh tier.
        let resource_usage_active = self.resource_usage_animating()
            || (self.expanded
                && self.page_visible(ExpandedPage::Home)
                && crate::ui::dashboard::is_animating());
        let compact_components_visible = self.expanded || !self.components_hidden;
        let playback_active = !self.is_hidden()
            && compact_components_visible
            && pacing.media_is_playing
            && (!self.expanded || self.page_visible(ExpandedPage::Music));
        let dynamic_effect_active = !self.is_hidden()
            && compact_components_visible
            && self.config.island_style == "dynamic"
            && self.current_media_info().thumbnail.is_some();
        let interactive_active = (pacing.is_hovering_visible
            && (!self.expanded || self.page_visible(ExpandedPage::Music)))
            || pacing.compact_overlay_visible
            || pacing.passive_reveal_active
            || self.right_press_cursor.is_some();
        if !transition_active
            && !interactive_active
            && !resource_usage_active
            && (!self.expanded || (!playback_active && !dynamic_effect_active))
            && self.working_set_trim.due(now)
            && let Err(error) = crate::platform::metrics().trim_working_set()
        {
            log::warn!("Working set trim failed: {error}");
        }
        let frame_interval = if transition_active {
            self.animation_frame_interval
        } else if self.expanded
            && (playback_active
                || dynamic_effect_active
                || interactive_active
                || resource_usage_active)
        {
            self.aligned_frame_interval(Duration::from_secs_f64(
                1.0 / f64::from(self.config.expanded_idle_fps),
            ))
        } else if playback_active || dynamic_effect_active {
            self.aligned_frame_interval(PLAYBACK_FRAME_INTERVAL)
        } else if interactive_active || resource_usage_active {
            self.aligned_frame_interval(INTERACTIVE_FRAME_INTERVAL)
        } else if self.is_hidden() {
            HIDDEN_FRAME_INTERVAL
        } else {
            IDLE_FRAME_INTERVAL
        };
        self.next_frame_deadline = now + frame_interval;
        if transition_active
            || resource_usage_active
            || playback_active
            || dynamic_effect_active
            || interactive_active
            || should_periodic_redraw
        {
            window.request_redraw();
        }
    }

    fn aligned_frame_interval(&self, desired: Duration) -> Duration {
        let refresh = self.display_frame_interval.as_nanos().max(1);
        let refreshes = ((desired.as_nanos() + refresh / 2) / refresh).max(1) as u32;
        self.display_frame_interval.saturating_mul(refreshes)
    }

    fn resource_usage_animating(&self) -> bool {
        use winisland_core::config::{CompactWidgetKind, WidgetKind};

        if self.is_hidden() || self.compact_overlay.is_visible() {
            return false;
        }
        let visible = if self.expanded {
            self.page_visible(ExpandedPage::Widgets)
                && self
                    .config
                    .widget_layout
                    .iter()
                    .any(|slot| slot.widget == Some(WidgetKind::ResourceUsage))
        } else {
            self.compact_layout()
                .iter()
                .any(|slot| slot.widget == Some(CompactWidgetKind::ResourceUsage))
        };
        visible
            && if self.expanded {
                crate::ui::widget::resource_usage::is_animating(&self.config.resource_metrics)
            } else {
                crate::ui::widget::resource_usage::is_animating(
                    &self.config.compact_resource_metrics,
                )
            }
    }
}

struct FramePacing {
    media_is_playing: bool,
    is_hovering_visible: bool,
    compact_overlay_visible: bool,
    passive_reveal_active: bool,
}

impl App {
    /// Shows "Charging", "On battery", "Fully charged" and low-battery activities when the
    /// power state changes. Polled every two seconds.
    fn poll_battery_activity(&mut self, now: Instant) {
        use crate::ui::compact::Activity;
        use crate::ui::components::icon::Icon;
        use winisland_core::i18n::tr;
        use winisland_render::Rgba;
        const POLL_INTERVAL: Duration = Duration::from_secs(2);
        const LOW_BATTERY: u8 = 15;
        if self
            .battery_checked_at
            .is_some_and(|at| now.duration_since(at) < POLL_INTERVAL)
        {
            return;
        }
        self.battery_checked_at = Some(now);
        // Blocking system queries run on the watcher thread; this only reads the snapshot.
        crate::core::system_watch::start();
        let system = crate::core::system_watch::snapshot();
        crate::ui::dashboard::system_page::set_snapshot(system.clone());
        if self.config.auto_modes && !self.expanded {
            // Modes follow what you are doing; never while the panel is open.
            let clock = crate::platform::shell().local_datetime();
            let foreground = system
                .apps
                .iter()
                .find(|app| app.foreground)
                .map_or("", |app| app.executable.as_str());
            let mode = crate::ui::dashboard::mode_for(foreground, clock.day_of_week, clock.hour);
            let target = self
                .config
                .workspaces
                .iter()
                .position(|space| space.name.eq_ignore_ascii_case(mode));
            if let Some(target) = target
                && target != self.config.active_workspace
            {
                self.config.active_workspace = target;
                crate::core::persistence::save_config(&self.config);
            }
        }
        self.sync_top_bar(now, &system);
        self.show_shell_events();
        self.compact_overlay.set_quiet(system.do_not_disturb);
        self.compact_overlay
            .set_activities_enabled(self.config.live_activities);
        crate::ui::island::set_capture_in_use(system.microphone, system.camera);
        self.update_call(system.microphone, &system.apps, now);
        self.poll_onboarding();
        const IDLE_DIM_AFTER: Duration = Duration::from_secs(10 * 60);
        crate::ui::island::set_idle_dimmed(
            !self.expanded
                && winisland_platform_windows::input_idle_duration()
                    .is_some_and(|idle| idle >= IDLE_DIM_AFTER),
        );
        if !self.expanded
            && !self.is_hidden()
            && let Some((title, minutes)) = self.calendar.take_starting_soon(5)
        {
            let when = if minutes == 0 {
                winisland_core::i18n::tr("activity_starting_now")
            } else {
                winisland_core::i18n::tr("activity_in_minutes").replacen(
                    "{}",
                    &minutes.to_string(),
                    1,
                )
            };
            self.compact_overlay
                .push_activity(crate::ui::compact::Activity {
                    icon: crate::ui::components::icon::Icon::Calendar,
                    accent: winisland_render::Rgba::from_argb(255, 255, 69, 58),
                    primary: title,
                    secondary: when,
                    progress: None,
                    actions: Vec::new(),
                    tall: false,
                    priority: crate::ui::compact::Priority::High,
                });
        }
        crate::ui::dashboard::set_battery(system.battery);
        let Some((level, plugged)) = system.battery else {
            return;
        };
        let previous = self.battery_state.replace((level, plugged));
        let Some((previous_level, was_plugged)) = previous else {
            return;
        };
        let green = Rgba::from_argb(255, 52, 199, 89);
        if plugged != was_plugged {
            crate::ui::widget::compact::mascot::react(if plugged {
                crate::ui::widget::compact::mascot::Reaction::Charging
            } else {
                crate::ui::widget::compact::mascot::Reaction::Surprised
            });
        }
        let activity = if plugged && !was_plugged {
            Some((Icon::Bolt, green, tr("activity_charging")))
        } else if !plugged && was_plugged {
            Some((Icon::Battery, Rgba::WHITE, tr("activity_on_battery")))
        } else if plugged && level == 100 && previous_level < 100 {
            Some((Icon::Battery, green, tr("activity_fully_charged")))
        } else if !plugged && level <= LOW_BATTERY && previous_level > LOW_BATTERY {
            Some((
                Icon::Battery,
                Rgba::from_argb(255, 255, 69, 58),
                tr("activity_low_battery"),
            ))
        } else {
            None
        };
        if let Some((icon, accent, secondary)) = activity
            && !self.expanded
            && !self.is_hidden()
        {
            let low = !plugged && level <= LOW_BATTERY;
            self.compact_overlay.push_activity(Activity {
                icon: if low { Icon::PetTired } else { icon },
                accent,
                primary: format!("{level}%"),
                secondary,
                progress: Some(f32::from(level) / 100.0),
                actions: (!plugged && level <= LOW_BATTERY)
                    .then(|| {
                        (
                            tr("activity_settings"),
                            "ms-settings:batterysaver".to_string(),
                        )
                    })
                    .into_iter()
                    .collect(),
                tall: low,
                priority: if !plugged && level <= LOW_BATTERY {
                    crate::ui::compact::Priority::High
                } else {
                    crate::ui::compact::Priority::Normal
                },
            });
        }
    }

    /// Confirms a clipboard capture with a "Copied · Text" activity.
    fn push_copied_activity(&mut self) {
        crate::ui::widget::compact::mascot::react(
            crate::ui::widget::compact::mascot::Reaction::Copied,
        );
        use crate::ui::compact::Activity;
        use crate::ui::components::icon::Icon;
        use winisland_core::clipboard::ClipboardBody;
        use winisland_core::i18n::tr;
        use winisland_render::Rgba;
        if self.expanded || self.is_hidden() {
            return;
        }
        let Some(item) = self
            .clipboard
            .items()
            .iter()
            .max_by_key(|item| item.captured_at)
        else {
            return;
        };
        let kind = match &item.body {
            ClipboardBody::Text(_) => tr("activity_text"),
            ClipboardBody::Link(_) => tr("activity_link"),
            ClipboardBody::Image { .. } => tr("activity_image"),
            ClipboardBody::Files(files) => {
                tr("clipboard_files").replacen("{}", &files.len().to_string(), 1)
            }
        };
        self.compact_overlay.push_activity(Activity {
            icon: Icon::Clipboard,
            accent: Rgba::from_rgb(255, 159, 10),
            primary: tr("activity_copied"),
            secondary: kind,
            progress: None,
            actions: Vec::new(),
            tall: false,
            priority: crate::ui::compact::Priority::Normal,
        });
    }
}

impl App {
    /// "Drop to save · Shelf" while a file is dragged over the notch, then
    /// "Saved to Shelf · name" once it lands.
    pub(super) fn push_drop_activity(&mut self, dropped: Option<String>) {
        use crate::ui::compact::Activity;
        use crate::ui::components::icon::Icon;
        use winisland_core::i18n::tr;
        use winisland_render::Rgba;
        if self.expanded || self.is_hidden() {
            return;
        }
        let blue = Rgba::from_argb(255, 10, 132, 255);
        let (primary, secondary) = match dropped {
            Some(name) => (tr("activity_saved_to_shelf"), name),
            None => (tr("activity_drop_here"), tr("activity_shelf")),
        };
        self.compact_overlay.push_activity(Activity {
            icon: Icon::Shelf,
            accent: blue,
            primary,
            secondary,
            progress: None,
            actions: Vec::new(),
            tall: false,
            priority: crate::ui::compact::Priority::Normal,
        });
    }
}

impl App {
    /// First-run tips shown in the notch itself, one every four seconds: hover to open,
    /// scroll for volume, drop files. Runs once, then records `onboarding_done`.
    fn poll_onboarding(&mut self) {
        use crate::ui::compact::Activity;
        use crate::ui::components::icon::Icon;
        use winisland_core::i18n::tr;
        use winisland_render::Rgba;
        const TIPS: [(Icon, &str, &str); 3] = [
            (Icon::Home, "onboarding_hover", "onboarding_hover_detail"),
            (Icon::Music, "onboarding_scroll", "onboarding_scroll_detail"),
            (Icon::Shelf, "onboarding_drop", "onboarding_drop_detail"),
        ];
        if self.config.onboarding_done || self.expanded || self.is_hidden() {
            return;
        }
        // Polled every two seconds; a tip on every other poll.
        let tick = self.onboarding_tick;
        self.onboarding_tick = tick.saturating_add(1);
        if tick % 2 != 0 {
            return;
        }
        let Some((icon, primary, secondary)) = TIPS.get(usize::from(tick / 2)) else {
            self.config.onboarding_done = true;
            crate::core::persistence::save_config(&self.config);
            return;
        };
        self.compact_overlay.push_activity(Activity {
            icon: *icon,
            accent: Rgba::from_argb(255, 10, 132, 255),
            primary: tr(primary),
            secondary: tr(secondary),
            progress: None,
            actions: Vec::new(),
            tall: false,
            priority: crate::ui::compact::Priority::Low,
        });
    }
}

impl App {
    /// Starts or stops the top bar session to match the setting. Never runs in safe mode, so
    /// `--safe-mode` is always a way back to the normal taskbar.
    fn sync_top_bar(&mut self, now: Instant, system: &crate::core::system_watch::SystemSnapshot) {
        use winisland_platform_windows::shell_bar::ScreenRect;
        let wanted = self.config.top_bar && !crate::core::safe_mode::active();
        let Some(window_ref) = self.window else {
            return;
        };
        let scale = crate::platform::window().scale_factor(window_ref.id()) as f32;
        let (x, y) = self.geom.monitor_pos;
        let (width, height) = self.geom.monitor_size;
        let monitor = ScreenRect {
            left: x,
            top: y,
            right: x + width as i32,
            bottom: y + height as i32,
        };
        let was_active = self.top_bar.is_some();
        match (&mut self.top_bar, wanted) {
            (Some(session), true) => session.poll(monitor, scale, now),
            (None, true) if width > 0 && height > 0 => {
                self.top_bar = crate::core::top_bar::TopBarSession::start(monitor, scale);
                if self.top_bar.is_none() {
                    log::warn!("Top bar: Windows refused the AppBar registration");
                }
            }
            (Some(_), false) => self.top_bar = None,
            _ => {}
        }
        let active = self.top_bar.is_some();
        crate::ui::top_bar::set_active(active);
        let _ = was_active;
        // The bar has its own window docked at the bottom of the monitor.
        // Anchor to the strip Windows actually reserved: its bottom is the true screen bottom,
        // even after the reservation shrinks the work area other code may report.
        let anchor = self.top_bar.as_ref().map_or(monitor, |session| {
            let reserved = session.reserved();
            ScreenRect {
                left: reserved.left,
                right: reserved.right,
                bottom: reserved.bottom,
                ..monitor
            }
        });
        self.sync_bar_window(active, anchor, scale);
        if active {
            self.publish_top_bar_apps(system.apps.clone());
            crate::ui::top_bar::set_battery(system.battery);
            if crate::ui::top_bar::popover::is_open() {
                self.refresh_popover_data(None);
            }
            self.request_bar_redraw();
        }
    }
}

impl App {
    /// Publishes the running app windows to the top bar.
    /// Publishes the running app windows to the top bar, re-reading them now (after a click).
    pub(super) fn refresh_top_bar_apps(&mut self) {
        self.publish_top_bar_apps(winisland_platform_windows::shell_bar::app_windows());
    }

    fn publish_top_bar_apps(
        &mut self,
        windows: Vec<winisland_platform_windows::shell_bar::AppWindow>,
    ) {
        let apps = windows
            .into_iter()
            .map(|window| crate::ui::top_bar::BarApp {
                handle: window.handle,
                process_id: window.process_id,
                label: crate::ui::top_bar::app_label(&window.executable, &window.title),
                title: window.title,
                foreground: window.foreground,
            })
            .collect::<Vec<_>>();
        crate::ui::top_bar::refresh_icons(
            &apps
                .iter()
                .map(|app: &crate::ui::top_bar::BarApp| app.handle)
                .collect::<Vec<_>>(),
        );
        crate::ui::top_bar::set_apps(apps);
    }
}

impl App {
    /// Shows a live preview of an app window after the pointer rests on its bar button for
    /// 450 ms, and removes it when the pointer moves on.
    pub(super) fn update_bar_preview(
        &mut self,
        window: &WindowRef,
        hovered: Option<isize>,
        now: Instant,
    ) {
        const DELAY: Duration = Duration::from_millis(450);
        let (bar_top, bar_width) = (self.bar_top(), self.bar_geometry.width as f32);
        let current = self.bar_preview.as_ref().map(|(handle, ..)| *handle);
        if current != hovered {
            self.bar_preview = hovered.map(|handle| (handle, now, None));
            crate::ui::top_bar::set_preview_card(None);
            window.request_redraw();
            return;
        }
        let Some((handle, since, thumbnail)) = &mut self.bar_preview else {
            return;
        };
        if thumbnail.is_some() || now.duration_since(*since) < DELAY {
            return;
        }
        let Some(surface) = crate::platform::window().native_surface(window.id()) else {
            return;
        };
        let Some(preview) = winisland_platform_windows::shell_bar::WindowThumbnail::new(
            surface.handle() as isize,
            *handle,
        ) else {
            return;
        };
        let scale = crate::platform::window().scale_factor(window.id()) as f32;
        let Some(button) = crate::ui::top_bar::app_rect(*handle) else {
            return;
        };
        let (source_w, source_h) = preview.source_size().unwrap_or((16, 10));
        let max_w = 260.0 * scale;
        let max_h = 160.0 * scale;
        let fit = (max_w / source_w.max(1) as f32).min(max_h / source_h.max(1) as f32);
        let (w, h) = (source_w as f32 * fit, source_h as f32 * fit);
        let pad = 8.0 * scale;
        // The preview card sits just above the bottom bar.
        let top = bar_top - h - pad * 2.0 - 8.0 * scale;
        let left = button.left.clamp(pad, (bar_width - w - pad * 3.0).max(pad));
        let card = winisland_render::Rect::from_xywh(left, top, w + pad * 2.0, h + pad * 2.0);
        preview.place(
            winisland_platform_windows::shell_bar::ScreenRect {
                left: (card.left + pad) as i32,
                top: (card.top + pad) as i32,
                right: (card.left + pad + w) as i32,
                bottom: (card.top + pad + h) as i32,
            },
            255,
        );
        crate::ui::top_bar::set_preview_card(Some(card));
        *thumbnail = Some(preview);
        window.request_redraw();
    }
}

impl App {
    /// Meeting mode: while the microphone is in use and a calling app runs, the notch shows
    /// the call with its duration and mic state, refreshed every poll so it stays up.
    fn update_call(
        &mut self,
        microphone: bool,
        apps: &[winisland_platform_windows::shell_bar::AppWindow],
        now: Instant,
    ) {
        use winisland_platform_windows::shell_bar;
        let caller = microphone
            .then(|| {
                apps.iter().find(|window| {
                    shell_bar::CALL_APPS
                        .iter()
                        .any(|app| window.executable.eq_ignore_ascii_case(app))
                        || window.title.contains("Meet - ")
                })
            })
            .flatten()
            .map(|window| crate::ui::top_bar::app_label(&window.executable, &window.title));
        match (caller, self.call.take()) {
            (Some(name), previous) => {
                let started = previous.map_or(now, |(started, _)| started);
                self.call = Some((started, name));
                self.push_call_activity(now);
            }
            (None, Some((started, name))) => {
                self.call_muted = false;
                if !self.expanded && !self.is_hidden() {
                    self.compact_overlay
                        .push_activity(crate::ui::compact::Activity {
                            icon: crate::ui::components::icon::Icon::Phone,
                            accent: winisland_render::Rgba::WHITE,
                            primary: format!("{name} call ended"),
                            secondary: format_call_time(now.duration_since(started)),
                            progress: None,
                            actions: Vec::new(),
                            tall: false,
                            priority: crate::ui::compact::Priority::Normal,
                        });
                }
            }
            (None, None) => {}
        }
    }

    pub(super) fn push_call_activity(&mut self, now: Instant) {
        let Some((started, name)) = &self.call else {
            return;
        };
        if self.expanded || self.is_hidden() {
            return;
        }
        let (accent, mic) = if self.call_muted {
            (winisland_render::Rgba::from_rgb(255, 69, 58), "Muted")
        } else {
            (winisland_render::Rgba::from_rgb(48, 209, 88), "Mic on")
        };
        let activity = crate::ui::compact::Activity {
            icon: crate::ui::components::icon::Icon::Phone,
            accent,
            primary: name.clone(),
            secondary: format!("{} · {mic}", format_call_time(now.duration_since(*started))),
            progress: None,
            actions: call_actions(name, self.call_muted),
            tall: false,
            priority: crate::ui::compact::Priority::High,
        };
        self.compact_overlay.push_activity(activity);
    }

    /// Middle-click during a call mutes or unmutes the microphone.
    pub(super) fn toggle_call_mute(&mut self) -> bool {
        if self.call.is_none() {
            return false;
        }
        self.call_muted = !self.call_muted;
        std::thread::spawn(winisland_platform_windows::shell_bar::toggle_microphone_mute);
        self.push_call_activity(Instant::now());
        true
    }
}

fn format_call_time(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

impl App {
    /// Turns system events from the top bar's host window into notch activities.
    /// The name from Settings, or the Windows account name with a capital first letter.
    fn display_name(&self) -> String {
        let configured = self.config.user_name.trim();
        if !configured.is_empty() {
            return configured.to_string();
        }
        std::env::var("USERNAME")
            .ok()
            .and_then(|name| {
                let mut chars = name.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().chain(chars).collect::<String>())
            })
            .unwrap_or_default()
    }

    /// The robot waves with "Good morning, Name" (or afternoon or evening) and the battery.
    fn push_greeting(&mut self, priority: crate::ui::compact::Priority) {
        use winisland_core::i18n::tr;
        let now = crate::platform::shell().local_datetime();
        let key = match now.hour {
            5..=11 => "greeting_morning",
            12..=16 => "greeting_afternoon",
            _ => "greeting_evening",
        };
        let name = self.display_name();
        let primary = if name.is_empty() {
            tr(key)
        } else {
            format!("{}, {name}", tr(key))
        };
        let secondary = crate::core::system_watch::snapshot()
            .battery
            .map(|(level, _)| format!("{level}%"))
            .unwrap_or_default();
        self.compact_overlay
            .push_activity(crate::ui::compact::Activity {
                icon: crate::ui::components::icon::Icon::Pet,
                accent: winisland_render::Rgba::WHITE,
                primary,
                secondary,
                progress: None,
                actions: Vec::new(),
                tall: true,
                priority,
            });
    }

    fn show_shell_events(&mut self) {
        if self.shell_listener.is_none() && !crate::core::safe_mode::active() {
            self.shell_listener = winisland_platform_windows::shell_bar::ShellEventListener::new();
        }
        use crate::ui::compact::Activity;
        use crate::ui::components::icon::Icon;
        use winisland_core::i18n::tr;
        use winisland_platform_windows::shell_bar::ShellEvent;
        use winisland_render::Rgba;
        crate::core::chime::set_enabled(
            self.config.robot_sounds && !crate::core::system_watch::snapshot().do_not_disturb,
        );
        // The robot's eyes follow the cursor while it is near the top centre of the screen.
        let cursor = crate::utils::mouse::get_global_cursor_pos();
        let from_centre =
            (cursor.0 - self.geom.monitor_pos.0) as f32 - self.geom.monitor_size.0 as f32 / 2.0;
        let below_top = (cursor.1 - self.geom.monitor_pos.1) as f32;
        crate::ui::widget::compact::mascot::set_gaze(
            (from_centre.abs() < 520.0 && below_top < 360.0).then(|| {
                (
                    (from_centre / 160.0).clamp(-1.6, 1.6),
                    ((below_top - 24.0) / 140.0).clamp(-0.6, 1.2),
                )
            }),
        );
        crate::ui::widget::compact::mascot::set_preferences(
            self.config.robot_liveliness,
            self.config.robot_festive,
            self.config.robot_nudges,
            self.config.robot_pc_reactions,
            &self.config.birthday,
        );
        let media = self.current_media_info();
        let playing_track = media.is_playing.then(|| media.title.clone());
        crate::ui::widget::compact::mascot::set_context(
            &self.display_name(),
            crate::ui::dashboard::focus_running(),
            playing_track.as_deref(),
            winisland_platform_windows::input_idle_duration()
                .map_or(0.0, |idle| idle.as_secs_f32()),
        );
        let system = crate::core::system_watch::snapshot();
        if system.sampled {
            crate::ui::widget::compact::mascot::set_system(system.online, system.cpu_percent);
        }
        if crate::core::screenshot_watch::poll_downloads(std::time::Instant::now()).is_some() {
            crate::ui::widget::compact::mascot::react(
                crate::ui::widget::compact::mascot::Reaction::Downloaded,
            );
        }
        if crate::core::device_watch::mouse_arrived(std::time::Instant::now()) {
            crate::ui::widget::compact::mascot::react(
                crate::ui::widget::compact::mascot::Reaction::MouseArrived,
            );
        }
        if let Some(path) = crate::core::screenshot_watch::poll(std::time::Instant::now())
            && !self.expanded
            && !self.is_hidden()
        {
            // The robot holds up the new picture; the tall card grows the notch for it.
            crate::core::chime::play(crate::core::chime::Sound::Pop);
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.compact_overlay
                .push_activity(crate::ui::compact::Activity {
                    icon: crate::ui::components::icon::Icon::PetPhoto,
                    accent: winisland_render::Rgba::WHITE,
                    primary: winisland_core::i18n::tr("activity_screenshot_saved"),
                    secondary: name,
                    progress: None,
                    actions: vec![(
                        winisland_core::i18n::tr("activity_open"),
                        path.to_string_lossy().into_owned(),
                    )],
                    tall: true,
                    priority: crate::ui::compact::Priority::High,
                });
        }
        // The robot says hello once when the app starts.
        static GREETED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !self.expanded
            && !self.is_hidden()
            && !GREETED.swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            self.push_greeting(crate::ui::compact::Priority::Normal);
        }
        for event in winisland_platform_windows::shell_bar::take_shell_events() {
            if self.expanded || self.is_hidden() {
                continue;
            }
            if matches!(event, ShellEvent::Unlocked) {
                // Welcome back goes first; nothing else should hold it up.
                self.push_greeting(crate::ui::compact::Priority::Critical);
                continue;
            }
            let battery = crate::core::system_watch::snapshot()
                .battery
                .map(|(level, _)| format!("{level}%"))
                .unwrap_or_default();
            let (icon, accent, primary, secondary) = match event {
                // Greeted above.
                ShellEvent::Unlocked => continue,
                ShellEvent::Resumed => (
                    Icon::Clock,
                    Rgba::from_rgb(255, 159, 10),
                    tr("activity_resumed"),
                    battery,
                ),
                ShellEvent::DisplayChanged => (
                    Icon::Grid,
                    Rgba::from_rgb(100, 210, 255),
                    tr("activity_display_changed"),
                    String::new(),
                ),
                ShellEvent::DriveAdded(letter) => (
                    Icon::Shelf,
                    Rgba::from_rgb(10, 132, 255),
                    tr("activity_drive_added"),
                    format!("{letter}:"),
                ),
                ShellEvent::DriveRemoved(letter) => (
                    Icon::Check,
                    Rgba::from_rgb(48, 209, 88),
                    tr("activity_drive_removed"),
                    format!("{letter}:"),
                ),
            };
            // Drives open in Explorer; display changes open display settings.
            let action = match event {
                ShellEvent::DriveAdded(letter) => {
                    crate::ui::widget::compact::mascot::react(
                        crate::ui::widget::compact::mascot::Reaction::Drive,
                    );
                    Some((tr("activity_open"), format!("{letter}:\\")))
                }
                ShellEvent::DisplayChanged => {
                    Some((tr("activity_settings"), "ms-settings:display".to_string()))
                }
                _ => None,
            };
            self.compact_overlay.push_activity(Activity {
                icon,
                accent,
                primary,
                secondary,
                progress: None,
                actions: action.into_iter().collect(),
                tall: matches!(event, ShellEvent::Resumed | ShellEvent::DriveAdded(_)),
                priority: crate::ui::compact::Priority::Normal,
            });
        }
    }
}

impl App {
    /// Publishes the data the bar's panels show. `volume` overrides the measured level right
    /// after the slider is used.
    pub(super) fn refresh_popover_data(&mut self, volume: Option<f32>) {
        let media = self.current_media_info();
        let track = (!media.title.is_empty())
            .then(|| (media.title.clone(), media.artist.clone(), media.is_playing));
        crate::ui::top_bar::popover::set_data(crate::ui::top_bar::popover::PopoverData {
            volume: volume.or_else(|| {
                crate::platform::audio()
                    .volume()
                    .ok()
                    .map(|state| state.level)
            }),
            battery: crate::core::system_watch::snapshot().battery,
            track,
            agenda: self.calendar.agenda_rows(),
        });
    }
}

/// Activity action that toggles the call microphone instead of opening a URL.
pub(super) const CALL_MUTE_ACTION: &str = "winisland:call-mute";

/// Activity action that turns the call camera on or off via the calling app's shortcut.
pub(super) const CALL_CAMERA_ACTION: &str = "winisland:call-camera";
/// Activity action that leaves the call via the calling app's shortcut.
pub(super) const CALL_LEAVE_ACTION: &str = "winisland:call-leave";

/// Keyboard shortcuts for a calling app: `(camera, leave)` as virtual-key chords.
pub(super) fn call_shortcuts(name: &str) -> Option<(&'static [u16], &'static [u16])> {
    const CTRL: u16 = 0x11;
    const SHIFT: u16 = 0x10;
    const ALT: u16 = 0x12;
    let name = name.to_ascii_lowercase();
    if name.contains("teams") {
        Some((&[CTRL, SHIFT, 0x4F], &[CTRL, SHIFT, 0x48])) // Ctrl+Shift+O, Ctrl+Shift+H
    } else if name.contains("zoom") {
        Some((&[ALT, 0x56], &[ALT, 0x51])) // Alt+V, Alt+Q
    } else {
        None
    }
}

/// Mute for every call; Camera and Leave where the app has known shortcuts.
fn call_actions(name: &str, muted: bool) -> Vec<(String, String)> {
    let mut actions = Vec::new();
    if call_shortcuts(name).is_some() {
        actions.push(("Camera".to_string(), CALL_CAMERA_ACTION.to_string()));
    }
    actions.push((
        if muted { "Unmute" } else { "Mute" }.to_string(),
        CALL_MUTE_ACTION.to_string(),
    ));
    if call_shortcuts(name).is_some() {
        actions.push(("Leave".to_string(), CALL_LEAVE_ACTION.to_string()));
    }
    actions
}

impl App {
    /// Sends a camera or leave shortcut to the calling app's window.
    pub(super) fn send_call_shortcut(&mut self, leave: bool) {
        use winisland_platform_windows::shell_bar;
        let Some((_, name)) = &self.call else {
            return;
        };
        let Some((camera, leave_keys)) = call_shortcuts(name) else {
            return;
        };
        let lower = name.to_ascii_lowercase();
        if let Some(window) = shell_bar::app_windows()
            .into_iter()
            .find(|window| window.executable.to_ascii_lowercase().contains(&lower))
        {
            shell_bar::activate_window(window.handle);
        }
        let keys = if leave { leave_keys } else { camera };
        std::thread::spawn(move || {
            // Give the call window a moment to take focus before the shortcut.
            std::thread::sleep(std::time::Duration::from_millis(120));
            shell_bar::press_shortcut(keys);
        });
    }
}

impl App {
    /// Bottom corner radius of the collapsed island. As a notch it has a flat bottom with small
    /// corners, like the Mac notch and Vayli; as a floating island it stays a pill.
    fn compact_radius(&self, height: f32) -> f32 {
        if self.config.notch_shape {
            (height * 0.36).min(10.0 * self.config.compact_scale.max(0.5))
        } else {
            height / 2.0
        }
    }
}
