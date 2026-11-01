use super::AppConfig;

pub const CONFIG_VERSION: u32 = 7;

/// Smallest expanded panel that fits the standard dashboard layout comfortably.
const MIN_DASHBOARD_WIDTH: f32 = 720.0;
const MIN_DASHBOARD_HEIGHT: f32 = 320.0;

/// Upgrades a config loaded as `from_version` to the current version.
///
/// Returns `true` when the config changed and therefore has to be written back.
pub fn migrate(config: &mut AppConfig, from_version: u32) -> bool {
    if from_version >= CONFIG_VERSION {
        return false;
    }
    config.config_version = CONFIG_VERSION;
    if from_version < 3 {
        config.expanded_height = (config.expanded_height + 60.0).min(1000.0);
        for entry in &mut config.widget_layout {
            if entry.widget == Some(super::WidgetKind::Settings) && entry.slot == 17 {
                entry.widget = None;
            }
        }
        if !config
            .widget_layout
            .iter()
            .any(|entry| entry.widget == Some(super::WidgetKind::Settings))
        {
            config.widget_layout.push(super::WidgetSlot {
                slot: 23,
                widget: Some(super::WidgetKind::Settings),
            });
        }
        if (config.resource_widget_columns, config.resource_widget_rows) == (2, 1) {
            config.resource_widget_rows = 2;
        }
    }
    if from_version < 4 {
        // The dashboard needs a wide panel; the inherited 360 x 260 default forced the
        // cramped compact layout.
        config.expanded_width = config.expanded_width.max(MIN_DASHBOARD_WIDTH);
        config.expanded_height = config.expanded_height.max(MIN_DASHBOARD_HEIGHT);
    }
    if from_version < 5 {
        // The System tab is new: show it in every existing workspace.
        for workspace in &mut config.workspaces {
            if !workspace
                .modules
                .iter()
                .any(|module| module == "dashboard_system")
            {
                workspace.modules.insert(0, "dashboard_system".to_string());
            }
        }
    }
    if from_version < 6 {
        // New tools tabs: show them in every existing workspace.
        for workspace in &mut config.workspaces {
            for key in [
                "dashboard_converter",
                "dashboard_units",
                "dashboard_message",
            ] {
                if !workspace.modules.iter().any(|module| module == key) {
                    workspace.modules.push(key.to_string());
                }
            }
        }
    }
    if from_version < 7 {
        // No default workspace listed the Clock tab, so it could not be opened.
        for workspace in &mut config.workspaces {
            if !workspace
                .modules
                .iter()
                .any(|module| module == "dashboard_clock")
            {
                workspace.modules.push("dashboard_clock".to_string());
            }
        }
    }
    true
}
