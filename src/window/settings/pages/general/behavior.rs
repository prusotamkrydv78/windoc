use crate::utils::settings_ui::ClickResult;
use crate::utils::settings_ui::items::SettingsItem;
use crate::window::settings::PopupState;
use winisland_core::config::{AppConfig, AppConfigField, MAX_HIDDEN_WIDTH};
use winisland_core::i18n::{available_langs, current_lang, init_i18n, set_lang, tr};
use winisland_render::text::FontManager;

use super::super::{PageInput, SettingsPage};
use super::SettingsApp;

#[derive(Clone, Copy)]
pub(super) enum BehaviorAction {
    AutoStart,
    Language,
    CheckUpdatesNow,
    ResetDefaults,
    /// Settings back to defaults and local data cleared, as on first run.
    StartFresh,
    WeatherLocation,
    ClearData(crate::core::data_inventory::DataKind),
    AddWorldClock,
    StatusMessage,
    UserName,
    Birthday,
    PreviewReaction,
    AddLink,
    AddFeed,
    RemoveFeed(usize),
    AddShelf,
    DeleteShelf(u64),
    ToggleModule(crate::ui::dashboard::Destination),
    RemoveLink(usize),
    ExportBackup,
    RestoreBackup,
    RemoveWorldClock(usize),
    HideIsland,
    Exit,
    ConnectPhone,
}

impl SettingsApp {
    pub(super) fn build_behavior_page(&self) -> SettingsPage<BehaviorAction> {
        let caps = crate::platform::capabilities();
        let mut page = SettingsPage::new();
        page.section(tr("section_behavior"));
        page.group_start();
        page.row_switch(
            tr("start_boot"),
            self.config.auto_start,
            caps.autostart,
            BehaviorAction::AutoStart,
        );
        page.push_setting(
            SettingsItem::RowStepper {
                label: tr("hidden_width"),
                value: if self.config.hidden_width >= MAX_HIDDEN_WIDTH {
                    tr("hidden_width_off")
                } else {
                    format!("{:.0}", self.config.hidden_width)
                },
                enabled: true,
            },
            AppConfigField::HiddenWidth,
        );
        if !caps.autostart || !caps.toast_events || !caps.input_hooks || !caps.brightness_control {
            page.row_label(tr("platform_unavailable"));
        }
        if !caps.tray {
            page.row_button(
                tr("tray_hide"),
                tr("tray_hide"),
                true,
                BehaviorAction::HideIsland,
            );
            page.row_button(tr("tray_exit"), tr("tray_exit"), true, BehaviorAction::Exit);
        }

        let language = current_lang();
        page.row_source(
            tr("language"),
            available_langs()
                .iter()
                .map(|entry| (entry.name.clone(), entry.code == language))
                .collect(),
            true,
            BehaviorAction::Language,
        );
        page.group_end();
        page.section("Phone".to_string());
        page.group_start();
        page.row_button(
            if crate::core::phone_link::paired() {
                "Notchy phone app (linked)".to_string()
            } else {
                "Connect the Notchy phone app".to_string()
            },
            "Show pairing QR".to_string(),
            true,
            BehaviorAction::ConnectPhone,
        );
        page.row_label(
            "Opens the notch on the Phone page. In the phone app tap Pair and scan the QR; both must be on the same Wi-Fi.".to_string(),
        );
        page.group_end();
        page.section(tr("section_screen_time"));
        page.group_start();
        page.setting(&self.config, AppConfigField::ScreenTimeEnabled, true);
        if self.config.screen_time_enabled {
            page.setting(&self.config, AppConfigField::ScreenTimeTitles, true);
            page.setting(&self.config, AppConfigField::ScreenTimeLimitMinutes, true);
        }
        page.row_label(tr("screen_time_hint"));
        page.group_end();
        page.section(tr("section_clipboard"));
        page.group_start();
        page.setting(&self.config, AppConfigField::ClipboardHistoryEnabled, true);
        page.row_label(tr("clipboard_history_hint"));
        page.group_end();
        page.section(tr("section_weather"));
        page.group_start();
        page.setting(&self.config, AppConfigField::WeatherEnabled, true);
        if self.config.weather_enabled {
            let place = self
                .config
                .weather_place
                .as_ref()
                .map(|place| {
                    if place.detail.is_empty() {
                        place.name.clone()
                    } else {
                        format!("{}, {}", place.name, place.detail)
                    }
                })
                .unwrap_or_default();
            page.row_text_field(
                tr("weather_location"),
                place,
                tr("weather_location_placeholder"),
                true,
                BehaviorAction::WeatherLocation,
            );
            if let Some(status) = &self.weather_status {
                page.row_label(status.clone());
            }
            page.setting(&self.config, AppConfigField::WeatherImperial, true);
        }
        page.row_label(tr("weather_hint"));
        page.group_end();
        page.section(tr("section_ai_usage"));
        page.group_start();
        page.setting(&self.config, AppConfigField::AiUsageEnabled, true);
        page.row_label(tr("ai_usage_hint"));
        page.group_end();
        page.section(tr("section_wellness"));
        page.group_start();
        page.row_label(tr("wellness_hint"));
        page.group_end();
        page.section(tr("section_modules"));
        page.group_start();
        for destination in crate::ui::dashboard::Destination::MORE_ITEMS {
            let key = destination.label_key();
            let hidden = self
                .config
                .hidden_modules
                .iter()
                .any(|hidden| hidden == key);
            page.row_button(
                tr(key),
                tr(if hidden { "module_show" } else { "module_hide" }),
                true,
                BehaviorAction::ToggleModule(destination),
            );
        }
        page.group_end();
        page.section(tr("section_shelves"));
        page.group_start();
        let shelves = crate::core::shelf_store::saved_shelves();
        for (id, name) in &shelves {
            page.row_button(
                name.clone(),
                tr("world_clock_remove"),
                shelves.len() > 1,
                BehaviorAction::DeleteShelf(*id),
            );
        }
        page.row_text_field(
            tr("shelves_add"),
            String::new(),
            tr("shelves_placeholder"),
            true,
            BehaviorAction::AddShelf,
        );
        page.group_end();
        page.section(tr("section_calendar_feeds"));
        page.group_start();
        for (index, feed) in self.config.calendar_feeds.iter().enumerate() {
            page.row_button(
                feed.clone(),
                tr("world_clock_remove"),
                true,
                BehaviorAction::RemoveFeed(index),
            );
        }
        page.row_text_field(
            tr("calendar_feed_add"),
            String::new(),
            tr("calendar_feed_placeholder"),
            self.config.calendar_feeds.len() < 8,
            BehaviorAction::AddFeed,
        );
        page.row_label(tr("calendar_feed_hint"));
        page.group_end();
        page.section(tr("section_links"));
        page.group_start();
        for (index, (name, url)) in self.config.links.iter().enumerate() {
            page.row_button(
                format!("{name} · {url}"),
                tr("world_clock_remove"),
                true,
                BehaviorAction::RemoveLink(index),
            );
        }
        page.row_text_field(
            tr("links_add"),
            String::new(),
            tr("links_placeholder"),
            self.config.links.len() < 12,
            BehaviorAction::AddLink,
        );
        page.group_end();
        page.section(tr("section_you"));
        page.group_start();
        page.row_text_field(
            tr("user_name"),
            self.config.user_name.clone(),
            tr("user_name_placeholder"),
            true,
            BehaviorAction::UserName,
        );
        page.row_label(tr("user_name_hint"));
        page.group_end();
        page.section(tr("section_robot"));
        page.group_start();
        page.setting(&self.config, AppConfigField::AutoModes, true);
        page.setting(&self.config, AppConfigField::RobotEnabled, true);
        let robot = self.config.robot_enabled;
        page.setting(&self.config, AppConfigField::RobotLiveliness, robot);
        page.setting(&self.config, AppConfigField::RobotFestive, robot);
        page.setting(&self.config, AppConfigField::RobotNudges, robot);
        page.setting(&self.config, AppConfigField::RobotPcReactions, robot);
        page.setting(&self.config, AppConfigField::RobotSounds, robot);
        page.setting(&self.config, AppConfigField::RobotCharacter, robot);
        page.setting(&self.config, AppConfigField::RobotColor, robot);
        page.setting(&self.config, AppConfigField::RobotAccessory, robot);
        page.setting(&self.config, AppConfigField::RobotRoam, robot);
        page.row_label(tr("robot_accessory_hint"));
        page.row_text_field(
            tr("birthday"),
            self.config.birthday.clone(),
            tr("birthday_placeholder"),
            robot,
            BehaviorAction::Birthday,
        );
        page.row_button(
            tr("robot_preview"),
            tr("robot_preview_button"),
            robot,
            BehaviorAction::PreviewReaction,
        );
        page.group_end();
        page.section(tr("section_status_message"));
        page.group_start();
        page.row_text_field(
            tr("status_message"),
            self.config.status_message.clone(),
            tr("status_message_placeholder"),
            true,
            BehaviorAction::StatusMessage,
        );
        page.row_label(tr("status_message_hint"));
        page.group_end();
        page.section(tr("section_world_clocks"));
        page.group_start();
        for (index, key) in self.config.world_clocks.iter().enumerate() {
            page.row_button(
                key.clone(),
                tr("world_clock_remove"),
                true,
                BehaviorAction::RemoveWorldClock(index),
            );
        }
        page.row_text_field(
            tr("world_clock_add"),
            String::new(),
            tr("world_clock_placeholder"),
            self.config.world_clocks.len() < MAX_WORLD_CLOCKS,
            BehaviorAction::AddWorldClock,
        );
        page.group_end();
        page.section(tr("section_backup"));
        page.group_start();
        page.row_text_field(
            tr("backup_export"),
            String::new(),
            tr("backup_passphrase_placeholder"),
            true,
            BehaviorAction::ExportBackup,
        );
        page.row_text_field(
            tr("backup_restore"),
            String::new(),
            tr("backup_passphrase_placeholder"),
            true,
            BehaviorAction::RestoreBackup,
        );
        if let Some(status) = &self.backup_status {
            page.row_label(status.clone());
        }
        page.row_label(tr("backup_hint"));
        page.group_end();
        page.section(tr("section_local_data"));
        page.group_start();
        for kind in crate::core::data_inventory::DataKind::ALL {
            let stored = kind.stored_bytes();
            let label = match stored {
                Some(bytes) => format!(
                    "{} · {}",
                    tr(kind.label_key()),
                    crate::core::data_inventory::format_bytes(bytes)
                ),
                None => format!("{} · {}", tr(kind.label_key()), tr("data_none")),
            };
            page.row_button(
                label,
                tr("data_clear"),
                stored.is_some(),
                BehaviorAction::ClearData(kind),
            );
        }
        page.row_label(tr("local_data_hint"));
        page.group_end();
        page.section(tr("section_updates"));
        page.group_start();
        page.setting(&self.config, AppConfigField::CheckForUpdates, true);
        if self.config.check_for_updates {
            page.setting(&self.config, AppConfigField::UpdateChannel, true);
            page.setting(&self.config, AppConfigField::UpdateCheckInterval, true);
        }
        page.row_button(
            tr("check_updates_manual"),
            tr("update_check_btn"),
            true,
            BehaviorAction::CheckUpdatesNow,
        );
        page.group_end();
        page.section(tr("section_start_fresh"));
        page.group_start();
        let armed = START_FRESH_ARMED
            .with(std::cell::Cell::get)
            .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(5));
        page.row_button(
            tr("start_fresh"),
            tr(if armed {
                "start_fresh_confirm"
            } else {
                "start_fresh_button"
            }),
            true,
            BehaviorAction::StartFresh,
        );
        page.row_label(tr("start_fresh_hint"));
        page.group_end();
        page.spacer(10.0);
        page.center_link(
            tr("reset_defaults"),
            self.theme().danger,
            BehaviorAction::ResetDefaults,
        );
        page
    }

    pub(super) fn handle_behavior_click(&mut self, input: PageInput) {
        let page = self.build_behavior_page();
        let result = input.hit_test(&page);
        if self.handle_setting_click(&page, &result, input) {
            return;
        }
        let Some(action) = page.action(&result).copied() else {
            return;
        };

        let changed = match (action, &result) {
            (BehaviorAction::AutoStart, ClickResult::Switch(_)) => {
                let enabled = !self.config.auto_start;
                match crate::platform::shell().set_autostart(enabled) {
                    Ok(()) => {
                        self.config.auto_start = enabled;
                        true
                    }
                    Err(error) => {
                        crate::platform::update_capabilities(|caps| caps.autostart = false);
                        log::warn!("Autostart is unavailable: {error}");
                        false
                    }
                }
            }
            (BehaviorAction::HideIsland, ClickResult::RowButton(_)) => {
                self.plugin_request =
                    Some(crate::window::settings::PluginSettingsRequest::HideIsland);
                false
            }
            (BehaviorAction::ConnectPhone, ClickResult::RowButton(_)) => {
                self.plugin_request =
                    Some(crate::window::settings::PluginSettingsRequest::ConnectPhone);
                false
            }
            (BehaviorAction::Exit, ClickResult::RowButton(_)) => {
                self.plugin_request = Some(crate::window::settings::PluginSettingsRequest::Exit);
                false
            }
            (BehaviorAction::StartFresh, ClickResult::RowButton(_)) => {
                let armed = START_FRESH_ARMED
                    .with(std::cell::Cell::get)
                    .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(5));
                if armed {
                    START_FRESH_ARMED.with(|cell| cell.set(None));
                    self.config = AppConfig::default();
                    init_i18n(&self.config.language);
                    crate::ui::widget::expanded::calendar::clear_calendar_text_cache();
                    FontManager::global()
                        .set_custom_font_path(self.config.custom_font_path.as_deref());
                    self.plugin_request =
                        Some(crate::window::settings::PluginSettingsRequest::StartFresh);
                    true
                } else {
                    // The first click only arms it; a second within 5 s resets.
                    START_FRESH_ARMED.with(|cell| cell.set(Some(std::time::Instant::now())));
                    self.mark_items_dirty();
                    false
                }
            }
            (BehaviorAction::ResetDefaults, ClickResult::CenterLink(_)) => {
                self.config = AppConfig::default();
                init_i18n(&self.config.language);
                crate::ui::widget::expanded::calendar::clear_calendar_text_cache();
                FontManager::global().set_custom_font_path(self.config.custom_font_path.as_deref());
                true
            }
            (BehaviorAction::WeatherLocation, ClickResult::TextField(index)) => {
                let rect = input.text_field_rect(&page, *index, self.scroll_y);
                self.weather_search_rect = rect;
                let current = self
                    .config
                    .weather_place
                    .as_ref()
                    .map(|place| place.name.clone())
                    .unwrap_or_default();
                self.begin_text_input(
                    rect,
                    &current,
                    WEATHER_QUERY_MAX_CHARS,
                    commit_weather_search,
                );
                false
            }
            (BehaviorAction::RemoveWorldClock(index), ClickResult::RowButton(_)) => {
                if index < self.config.world_clocks.len() {
                    self.config.world_clocks.remove(index);
                }
                true
            }
            (BehaviorAction::ExportBackup, ClickResult::TextField(index)) => {
                let rect = input.text_field_rect(&page, *index, self.scroll_y);
                self.begin_text_input(rect, "", 128, commit_export_backup);
                false
            }
            (BehaviorAction::RestoreBackup, ClickResult::TextField(index)) => {
                let rect = input.text_field_rect(&page, *index, self.scroll_y);
                self.begin_text_input(rect, "", 128, commit_restore_backup);
                false
            }
            (BehaviorAction::ToggleModule(destination), ClickResult::RowButton(_)) => {
                let key = destination.label_key().to_string();
                if let Some(position) = self
                    .config
                    .hidden_modules
                    .iter()
                    .position(|hidden| *hidden == key)
                {
                    self.config.hidden_modules.remove(position);
                } else {
                    self.config.hidden_modules.push(key);
                }
                true
            }
            (BehaviorAction::DeleteShelf(id), ClickResult::RowButton(_)) => {
                self.plugin_request = Some(
                    crate::window::settings::PluginSettingsRequest::DeleteShelf(id),
                );
                false
            }
            (BehaviorAction::AddShelf, ClickResult::TextField(index)) => {
                let rect = input.text_field_rect(&page, *index, self.scroll_y);
                self.begin_text_input(rect, "", 60, commit_add_shelf);
                false
            }
            (BehaviorAction::RemoveFeed(index), ClickResult::RowButton(_)) => {
                if index < self.config.calendar_feeds.len() {
                    self.config.calendar_feeds.remove(index);
                }
                true
            }
            (BehaviorAction::AddFeed, ClickResult::TextField(index)) => {
                let rect = input.text_field_rect(&page, *index, self.scroll_y);
                self.begin_text_input(rect, "", 500, commit_add_feed);
                false
            }
            (BehaviorAction::RemoveLink(index), ClickResult::RowButton(_)) => {
                if index < self.config.links.len() {
                    self.config.links.remove(index);
                }
                true
            }
            (BehaviorAction::AddLink, ClickResult::TextField(index)) => {
                let rect = input.text_field_rect(&page, *index, self.scroll_y);
                self.begin_text_input(rect, "", 200, commit_add_link);
                false
            }
            (BehaviorAction::StatusMessage, ClickResult::TextField(index)) => {
                let rect = input.text_field_rect(&page, *index, self.scroll_y);
                let current = self.config.status_message.clone();
                self.begin_text_input(rect, &current, 60, commit_status_message);
                false
            }
            (BehaviorAction::Birthday, ClickResult::TextField(index)) => {
                let rect = input.text_field_rect(&page, *index, self.scroll_y);
                let current = self.config.birthday.clone();
                self.begin_text_input(rect, &current, 5, commit_birthday);
                false
            }
            (BehaviorAction::PreviewReaction, ClickResult::RowButton(_)) => {
                crate::ui::widget::compact::mascot::preview_next();
                false
            }
            (BehaviorAction::UserName, ClickResult::TextField(index)) => {
                let rect = input.text_field_rect(&page, *index, self.scroll_y);
                let current = self.config.user_name.clone();
                self.begin_text_input(rect, &current, 40, commit_user_name);
                false
            }
            (BehaviorAction::AddWorldClock, ClickResult::TextField(index)) => {
                let rect = input.text_field_rect(&page, *index, self.scroll_y);
                self.weather_search_rect = rect;
                self.begin_text_input(rect, "", 60, commit_world_clock_search);
                false
            }
            (BehaviorAction::ClearData(kind), ClickResult::RowButton(_)) => {
                self.plugin_request = Some(
                    crate::window::settings::PluginSettingsRequest::ClearData(kind),
                );
                false
            }
            (BehaviorAction::CheckUpdatesNow, ClickResult::RowButton(_)) => {
                crate::utils::updater::check_updates_manually();
                false
            }
            _ => false,
        };
        if changed {
            self.persist_settings_change();
            return;
        }

        let ClickResult::SourceButton(item_index) = result else {
            return;
        };
        let button_rect = input.popup_button_rect(&page, item_index, self.scroll_y);
        let (win_w, win_h) = self.logical_window_size();
        let popup = match action {
            BehaviorAction::Language => {
                let languages = available_langs();
                let current = current_lang();
                let selected = languages
                    .iter()
                    .position(|entry| entry.code == current)
                    .unwrap_or(0);
                PopupState::new(
                    select_language,
                    button_rect,
                    languages.iter().map(|entry| entry.name.clone()).collect(),
                    languages.iter().map(|entry| entry.code.clone()).collect(),
                    selected,
                    win_w,
                    win_h,
                )
            }
            _ => return,
        };
        self.show_popup(popup);
    }
}

fn select_language(app: &mut SettingsApp, value: &str) {
    app.config.language = value.to_string();
    set_lang(value);
    crate::ui::widget::expanded::calendar::clear_calendar_text_cache();
}

const WEATHER_QUERY_MAX_CHARS: usize = 80;

/// Starts a place search for the typed city. This user action is what allows the
/// network request; an empty query clears the location instead.
fn commit_weather_search(app: &mut SettingsApp, query: &str) {
    let query = query.trim().to_string();
    if query.is_empty() {
        app.config.weather_place = None;
        app.weather_status = None;
        return;
    }
    let language = current_lang().split('_').next().unwrap_or("en").to_string();
    let (sender, receiver) = std::sync::mpsc::channel();
    tokio::spawn(async move {
        let result = winisland_core::weather::search_places(&query, &language)
            .await
            .map_err(|error| error.to_string());
        let _ = sender.send(result);
    });
    app.weather_search = Some(receiver);
    app.weather_status = Some(tr("weather_searching"));
    app.mark_items_dirty();
}

fn select_weather_place(app: &mut SettingsApp, value: &str) {
    if let Some(place) = value
        .parse::<usize>()
        .ok()
        .and_then(|index| app.weather_results.get(index).cloned())
    {
        app.config.weather_place = Some(place);
    }
    app.weather_results.clear();
    app.weather_status = None;
    app.mark_items_dirty();
}

impl SettingsApp {
    pub(crate) fn poll_weather_search(&mut self) {
        let Some(receiver) = &self.weather_search else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("search was interrupted".to_string())
            }
        };
        self.weather_search = None;
        match result {
            Ok(mut places) if places.len() == 1 => {
                self.config.weather_place = places.pop();
                self.weather_status = None;
                self.persist_settings_change();
            }
            Ok(places) => {
                let (win_w, win_h) = self.logical_window_size();
                let labels = places
                    .iter()
                    .map(|place| {
                        if place.detail.is_empty() {
                            place.name.clone()
                        } else {
                            format!("{}, {}", place.name, place.detail)
                        }
                    })
                    .collect();
                let values = (0..places.len()).map(|index| index.to_string()).collect();
                self.weather_results = places;
                self.weather_status = None;
                self.show_popup(PopupState::new(
                    select_weather_place,
                    self.weather_search_rect,
                    labels,
                    values,
                    0,
                    win_w,
                    win_h,
                ));
            }
            Err(error) => {
                log::warn!("Weather place search failed: {error}");
                self.weather_status = Some(tr("weather_search_failed"));
            }
        }
        self.mark_items_dirty();
        self.request_redraw();
    }
}

const MAX_WORLD_CLOCKS: usize = 8;

/// Offers time zones whose name contains the typed text.
fn commit_world_clock_search(app: &mut SettingsApp, query: &str) {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return;
    }
    let matches: Vec<_> = winisland_platform_windows::time_zones()
        .into_iter()
        .filter(|zone| {
            zone.key.to_lowercase().contains(&query) || zone.display.to_lowercase().contains(&query)
        })
        .take(20)
        .collect();
    if matches.is_empty() {
        return;
    }
    let (win_w, win_h) = app.logical_window_size();
    let labels = matches.iter().map(|zone| zone.display.clone()).collect();
    let values = matches.into_iter().map(|zone| zone.key).collect();
    app.show_popup(PopupState::new(
        select_world_clock,
        app.weather_search_rect,
        labels,
        values,
        0,
        win_w,
        win_h,
    ));
}

fn select_world_clock(app: &mut SettingsApp, key: &str) {
    if !app
        .config
        .world_clocks
        .iter()
        .any(|existing| existing == key)
        && app.config.world_clocks.len() < MAX_WORLD_CLOCKS
    {
        app.config.world_clocks.push(key.to_string());
    }
    app.mark_items_dirty();
}

fn commit_export_backup(app: &mut SettingsApp, passphrase: &str) {
    if passphrase.is_empty() {
        return;
    }
    let Some(path) = rfd::FileDialog::new()
        .set_file_name("notchy-backup.wibk")
        .add_filter("Backup", &["wibk"])
        .save_file()
    else {
        return;
    };
    app.backup_status = Some(
        match crate::core::backup_service::export(&path, passphrase) {
            Ok(()) => tr("backup_export_done"),
            Err(error) => {
                log::warn!("Backup export failed: {error}");
                winisland_core::i18n::tr_args("backup_failed", &[&error])
            }
        },
    );
    app.mark_items_dirty();
}

fn commit_restore_backup(app: &mut SettingsApp, passphrase: &str) {
    if passphrase.is_empty() {
        return;
    }
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Backup", &["wibk"])
        .pick_file()
    else {
        return;
    };
    match crate::core::backup_service::restore(&path, passphrase) {
        Ok(()) => {
            app.plugin_request = Some(crate::window::settings::PluginSettingsRequest::Restart);
        }
        Err(error) => {
            log::warn!("Backup restore failed: {error}");
            app.backup_status = Some(winisland_core::i18n::tr_args("backup_failed", &[&error]));
            app.mark_items_dirty();
        }
    }
}

/// Accepts `MM-DD`; anything else clears the birthday.
fn commit_birthday(app: &mut SettingsApp, text: &str) {
    let text = text.trim();
    let valid = text.split_once('-').is_some_and(|(month, day)| {
        month
            .parse::<u32>()
            .is_ok_and(|month| (1..=12).contains(&month))
            && day.parse::<u32>().is_ok_and(|day| (1..=31).contains(&day))
    });
    app.config.birthday = if valid {
        text.to_string()
    } else {
        String::new()
    };
    app.mark_items_dirty();
}

fn commit_user_name(app: &mut SettingsApp, text: &str) {
    app.config.user_name = text.trim().to_string();
    app.mark_items_dirty();
}

fn commit_status_message(app: &mut SettingsApp, text: &str) {
    app.config.status_message = text.trim().to_string();
    app.mark_items_dirty();
}

/// Accepts `Name https://example.com`, or just a URL (its domain becomes the name).
fn commit_add_link(app: &mut SettingsApp, text: &str) {
    let text = text.trim();
    let (name, url) = match text.rsplit_once(char::is_whitespace) {
        Some((name, url)) => (name.trim().to_string(), url.trim().to_string()),
        None => (String::new(), text.to_string()),
    };
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    let name = if name.is_empty() {
        url.split("://")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or(&url)
            .to_string()
    } else {
        name
    };
    app.config.links.push((name, url));
    app.mark_items_dirty();
}

fn commit_add_shelf(app: &mut SettingsApp, name: &str) {
    let name = name.trim();
    if !name.is_empty() {
        app.plugin_request = Some(crate::window::settings::PluginSettingsRequest::CreateShelf(
            name.to_string(),
        ));
    }
}

fn commit_add_feed(app: &mut SettingsApp, url: &str) {
    let url = url.trim();
    let valid = ["https://", "http://", "webcal://"]
        .iter()
        .any(|scheme| url.starts_with(scheme));
    if valid && !app.config.calendar_feeds.iter().any(|feed| feed == url) {
        app.config.calendar_feeds.push(url.to_string());
        app.mark_items_dirty();
    }
}

thread_local! {
    /// When Reset everything was first clicked; a second click within 5 s confirms.
    static START_FRESH_ARMED: std::cell::Cell<Option<std::time::Instant>> =
        const { std::cell::Cell::new(None) };
}
