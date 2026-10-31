mod lyrics;
mod migrate;
mod visual;
mod widget_layout;
mod widgets;

use educe::Educe;
use serde::{Deserialize, Serialize};
use settings_schema::Settings;

pub use lyrics::*;
pub use migrate::*;
pub use visual::*;
pub use widget_layout::*;
pub use widgets::*;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APP_AUTHOR: &str = "Eatgrapes";
pub const APP_HOMEPAGE: &str = "https://github.com/WinIslandProject/WinIsland";
pub const WINDOW_TITLE: &str = "WinIsland";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Educe, Settings)]
#[educe(Default)]
#[serde(default)]
pub struct AppConfig {
    #[serde(alias = "global_scale")]
    #[educe(Default = 1.0)]
    #[setting(number(min = 0.5, max = 5.0, step = 0.05, precision = 2))]
    pub compact_scale: f32,
    #[educe(Default = 1.0)]
    #[setting(number(min = 0.5, max = 5.0, step = 0.05, precision = 2))]
    pub expanded_scale: f32,
    #[educe(Default = 120.0)]
    #[setting(number(min = 40.0, max = 400.0, step = 5.0))]
    pub base_width: f32,
    #[educe(Default = 27.0)]
    #[setting(number(min = 15.0, max = 200.0, step = 2.0))]
    pub base_height: f32,
    #[educe(Default = 720.0)]
    #[setting(number(min = 200.0, max = 2000.0, step = 10.0))]
    pub expanded_width: f32,
    #[educe(Default = 320.0)]
    #[setting(number(min = 100.0, max = 1000.0, step = 10.0))]
    pub expanded_height: f32,
    #[educe(Default = true)]
    #[setting(toggle)]
    pub motion_blur: bool,
    /// Attach the island to the top screen edge as a notch with concave shoulders.
    #[educe(Default = true)]
    #[setting(toggle)]
    pub notch_shape: bool,
    /// Replace the Windows taskbar with a top bar: reserves a strip at the top of the screen and
    /// hides the classic taskbar while on. On by default; `--safe-mode` always keeps the normal taskbar.
    #[educe(Default = true)]
    #[setting(toggle)]
    pub top_bar: bool,
    /// Short notch activities: charging, copied, focus, meetings and file drops.
    #[educe(Default = true)]
    #[setting(toggle)]
    pub live_activities: bool,
    #[educe(Default = 0)]
    #[setting(choice(
        30 => "30 FPS",
        60 => "60 FPS",
        90 => "90 FPS",
        120 => "120 FPS",
        0 => "frame_rate_native",
    ))]
    pub animation_fps: u32,
    #[educe(Default = 60)]
    #[setting(choice(30 => "30 FPS", 45 => "45 FPS", 60 => "60 FPS", 90 => "90 FPS"))]
    pub expanded_idle_fps: u32,
    /// "dynamic" washes the panel with a blurred copy of the album art.
    #[educe(Default = "dynamic")]
    pub island_style: String,
    #[educe(Default = true)]
    #[setting(toggle, label = "smtc_control")]
    pub smtc_enabled: bool,
    pub music_notice_acknowledged: bool,
    pub smtc_apps: Vec<String>,
    pub smtc_known_apps: Vec<String>,
    /// Captures system audio for the spectrum visualizer. Off by default: capture
    /// starts only after the user opts in.
    #[setting(toggle, label = "audio_visualizer")]
    pub audio_visualizer_enabled: bool,
    /// Records which apps are in the foreground, stored only on this PC. Off by default.
    #[setting(toggle, label = "screen_time_enabled")]
    pub screen_time_enabled: bool,
    /// Also stores window titles for screen time. Off by default.
    #[setting(toggle, label = "screen_time_titles")]
    pub screen_time_titles: bool,
    /// Daily screen time limit in minutes; 0 turns the reminder off.
    #[setting(
        number(min = 0.0, max = 960.0, step = 30.0),
        label = "screen_time_limit_minutes"
    )]
    pub screen_time_limit_minutes: f32,
    /// Keeps a local, encrypted history of copied text and files. Off by default.
    #[setting(toggle, label = "clipboard_history_enabled")]
    pub clipboard_history_enabled: bool,
    /// Fetches forecasts from Open-Meteo for `weather_place`. Off by default.
    #[setting(toggle, label = "weather_enabled")]
    pub weather_enabled: bool,
    #[setting(toggle, label = "weather_imperial")]
    pub weather_imperial: bool,
    /// Chosen by searching a city name; coordinates are rounded to about 1 km.
    pub weather_place: Option<crate::weather::Place>,
    /// Reads token counts from local Claude Code and Codex logs. Off by default.
    #[setting(toggle, label = "ai_usage_enabled")]
    pub ai_usage_enabled: bool,
    /// Windows time zone keys shown as world clocks.
    pub world_clocks: Vec<String>,
    /// What the robot calls the user in greetings; empty uses the Windows account name.
    pub user_name: String,
    /// Text shown by the Message compact widget.
    pub status_message: String,
    /// Panel theme: pure_black, ember, vivid or silver.
    #[educe(Default = "pure_black")]
    #[setting(choice(
        "pure_black" => "theme_pure_black",
        "ember" => "theme_ember",
        "vivid" => "theme_vivid",
        "silver" => "theme_silver",
    ), label = "ui_theme")]
    pub ui_theme: String,
    /// Glow around the island: off, theme (accent colour) or music (album art colour).
    #[educe(Default = "music")]
    #[setting(choice(
        "off" => "edge_light_off",
        "theme" => "edge_light_theme",
        "music" => "edge_light_music",
    ), label = "edge_light")]
    pub edge_light: String,
    /// Ctrl+Alt+H opens the island with the command palette (true) or hides it (false).
    #[educe(Default = true)]
    #[setting(toggle, label = "hotkey_opens_island")]
    pub hotkey_opens_island: bool,
    /// Opens the island after the cursor rests on it, and closes it when the cursor leaves.
    #[educe(Default = true)]
    #[setting(toggle, label = "hover_to_open")]
    pub hover_to_open: bool,
    /// Set once the first-run tips have been shown in the notch.
    pub onboarding_done: bool,
    /// More-page modules the user has hidden, by label key.
    pub hidden_modules: Vec<String>,
    /// Named sets of dashboard modules (Home, Work, Code…); the dock shows the active one.
    #[educe(Default(expression = default_workspaces()))]
    pub workspaces: Vec<Workspace>,
    pub active_workspace: usize,
    /// Subscribed ICS or webcal feed links.
    pub calendar_feeds: Vec<String>,
    /// Quick links as `(name, url)`.
    pub links: Vec<(String, String)>,
    /// Minutes between water reminders; 0 turns them off.
    #[setting(
        number(min = 0.0, max = 240.0, step = 15.0),
        label = "water_reminder_minutes"
    )]
    pub water_reminder_minutes: f32,
    /// Minutes between stretch reminders; 0 turns them off.
    #[setting(
        number(min = 0.0, max = 240.0, step = 15.0),
        label = "stretch_reminder_minutes"
    )]
    pub stretch_reminder_minutes: f32,
    #[educe(Default = true)]
    #[setting(toggle)]
    pub show_lyrics: bool,
    #[setting(toggle)]
    pub show_secondary_lyrics: bool,
    #[educe(Default = "lrc")]
    #[setting(choice("online" => "lyrics_mode_online", "lrc" => "lyrics_mode_lrc"))]
    pub lyrics_mode: String,
    pub lyrics_local_dir: Option<String>,
    pub custom_font_path: Option<String>,
    pub auto_start: bool,
    #[setting(toggle)]
    pub auto_hide: bool,
    #[setting(toggle)]
    pub fullscreen_auto_hide: bool,
    #[educe(Default = 5.0)]
    #[setting(number(min = 1.0, max = 60.0, step = 1.0), label = "hide_delay")]
    pub auto_hide_delay: f32,
    #[educe(Default = 5.0)]
    #[setting(number(min = MIN_HIDDEN_WIDTH, max = MAX_HIDDEN_WIDTH, step = 1.0))]
    pub hidden_width: f32,
    #[setting(toggle, label = "check_updates")]
    pub check_for_updates: bool,
    #[educe(Default = 4.0)]
    #[setting(number(min = 1.0, max = 24.0, step = 1.0), label = "update_interval")]
    pub update_check_interval: f32,
    #[educe(Default = "auto")]
    pub language: String,
    #[educe(Default = "163")]
    #[setting(choice(
        "163" => "lyrics_source_163",
        "qq" => "lyrics_source_qq",
        "kugou" => "lyrics_source_kugou",
        "lrclib" => "lyrics_source_lrclib",
        "amll" => "AMLL",
    ))]
    pub lyrics_source: String,
    #[setting(number(min = -10.0, max = 10.0, step = 0.1, precision = 1))]
    pub lyrics_delay: f64,
    #[setting(toggle)]
    pub lyrics_scroll: bool,
    #[educe(Default = 300.0)]
    #[setting(number(min = 100.0, max = 500.0, step = 10.0))]
    pub lyrics_scroll_max_width: f32,
    #[educe(Default = 6.0)]
    #[setting(number(min = 0.0, max = 32.0, step = 1.0))]
    pub lyrics_side_gap: f32,
    #[setting(choice(
        "random" => "lyrics_transition_random",
        "blur" => "lyrics_transition_blur",
        "slide" => "lyrics_transition_slide",
        "fade" => "lyrics_transition_fade",
    ))]
    pub lyrics_transition_animation: LyricTransitionMode,
    #[setting(number(step = 5.0))]
    pub position_x_offset: i32,
    #[setting(number(step = 5.0))]
    pub position_y_offset: i32,
    #[serde(rename = "dock_position", skip_serializing_if = "Option::is_none")]
    pub legacy_dock_position: Option<DockPosition>,
    pub monitor_index: i32,
    #[setting(number(min = 0.0, max = 30.0, step = 1.0))]
    pub font_size: f32,
    #[educe(Default = "system")]
    #[setting(choice("system" => "theme_system", "light" => "theme_light", "dark" => "theme_dark"))]
    pub settings_theme: String,
    #[educe(Default = "square")]
    pub mini_cover_shape: String,
    #[educe(Default = "square")]
    pub expanded_cover_shape: String,
    pub cover_rotate: bool,
    #[educe(Default = "stable")]
    #[setting(choice("stable" => "channel_stable", "beta" => "channel_beta"))]
    pub update_channel: String,
    #[setting(toggle)]
    pub right_click_drag: bool,
    #[setting(toggle)]
    pub notification_display: bool,
    #[educe(Default = true)]
    #[setting(toggle)]
    pub replace_native_volume_flyout: bool,
    #[educe(Default = true)]
    #[setting(toggle, label = "brightness_overlay")]
    pub brightness_overlay_enabled: bool,
    #[educe(Default(expression = default_widget_layout()))]
    pub widget_layout: Vec<WidgetSlot>,
    #[serde(deserialize_with = "widgets::deserialize_widget_sizes")]
    pub widget_sizes: Vec<WidgetSize>,
    pub plugin_widget_layout: Vec<PluginWidgetSlot>,
    #[educe(Default(expression = default_compact_widget_layout()))]
    pub compact_widget_layout: Vec<CompactWidgetSlot>,
    #[educe(Default(expression = default_resource_metrics()))]
    pub resource_metrics: Vec<ResourceMetricConfig>,
    #[educe(Default(expression = default_resource_metrics()))]
    pub compact_resource_metrics: Vec<ResourceMetricConfig>,
    #[educe(Default = 2)]
    pub resource_widget_columns: usize,
    #[educe(Default = 2)]
    pub resource_widget_rows: usize,
    #[serde(default)]
    #[educe(Default(expression = CONFIG_VERSION))]
    pub config_version: u32,
}

/// A named set of dashboard modules, by label key.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    pub name: String,
    pub modules: Vec<String>,
    /// Home cards in order as `(kind, column span)`; empty means the default set.
    #[serde(default)]
    pub cards: Vec<(String, usize)>,
}

fn default_workspaces() -> Vec<Workspace> {
    let space = |name: &str, modules: &[&str]| Workspace {
        name: name.to_string(),
        modules: modules.iter().map(|key| (*key).to_string()).collect(),
        cards: Vec::new(),
    };
    vec![
        space(
            "Home",
            &[
                "dashboard_media",
                "dashboard_weather",
                "dashboard_agenda",
                "dashboard_clipboard",
                "dashboard_shelf",
                "dashboard_drop",
            ],
        ),
        space(
            "Work",
            &[
                "dashboard_focus",
                "dashboard_agenda",
                "dashboard_notes",
                "dashboard_clipboard",
                "dashboard_shelf",
                "dashboard_links",
                "dashboard_screen_time",
            ],
        ),
        space(
            "Code",
            &[
                "dashboard_ai_usage",
                "dashboard_focus",
                "dashboard_clipboard",
                "dashboard_notes",
                "dashboard_links",
                "dashboard_screen_time",
            ],
        ),
    ]
}
