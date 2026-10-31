//! Notch, Top bar and Live Activities pages: everything about the notch itself, grouped by
//! what the user is trying to do rather than by implementation area.

use winisland_core::config::AppConfigField;
use winisland_core::i18n::tr;

use super::{PageInput, SettingsApp, SettingsPage};

/// These pages only hold settings rows, which `handle_setting_click` applies directly.
#[derive(Clone, Copy)]
pub(crate) enum NoAction {}

impl SettingsApp {
    pub(crate) fn build_notch_page(&self) -> SettingsPage<NoAction> {
        let mut page = SettingsPage::new();
        let group = |page: &mut SettingsPage<NoAction>, title: &str, fields: &[AppConfigField]| {
            page.section(tr(title));
            page.group_start();
            for field in fields {
                page.setting(&self.config, *field, true);
            }
            page.group_end();
        };
        group(
            &mut page,
            "section_notch_look",
            &[
                AppConfigField::NotchShape,
                AppConfigField::UiTheme,
                AppConfigField::EdgeLight,
            ],
        );
        group(
            &mut page,
            "section_notch_open",
            &[
                AppConfigField::HoverToOpen,
                AppConfigField::HotkeyOpensIsland,
                AppConfigField::RightClickDrag,
            ],
        );
        group(
            &mut page,
            "section_notch_size",
            &[
                AppConfigField::CompactScale,
                AppConfigField::BaseWidth,
                AppConfigField::BaseHeight,
                AppConfigField::ExpandedScale,
                AppConfigField::ExpandedWidth,
                AppConfigField::ExpandedHeight,
            ],
        );
        group(
            &mut page,
            "section_notch_position",
            &[
                AppConfigField::PositionXOffset,
                AppConfigField::PositionYOffset,
            ],
        );
        group(
            &mut page,
            "section_notch_hiding",
            &[
                AppConfigField::AutoHide,
                AppConfigField::AutoHideDelay,
                AppConfigField::FullscreenAutoHide,
            ],
        );
        page
    }

    pub(crate) fn build_top_bar_page(&self) -> SettingsPage<NoAction> {
        let mut page = SettingsPage::new();
        page.section(tr("section_top_bar"));
        page.group_start();
        page.setting(&self.config, AppConfigField::TopBar, true);
        page.group_end();
        page.section(tr("section_top_bar_about"));
        page.group_start();
        for key in [
            "top_bar_hint_space",
            "top_bar_hint_items",
            "top_bar_hint_restore",
            "top_bar_hint_safe_mode",
        ] {
            page.row_label(tr(key));
        }
        page.group_end();
        page
    }

    pub(crate) fn build_activities_page(&self) -> SettingsPage<NoAction> {
        let mut page = SettingsPage::new();
        page.section(tr("section_activities"));
        page.group_start();
        page.setting(&self.config, AppConfigField::LiveActivities, true);
        let caps = crate::platform::capabilities();
        page.setting(
            &self.config,
            AppConfigField::NotificationDisplay,
            caps.toast_events,
        );
        page.group_end();
        page.section(tr("section_activities_system"));
        page.group_start();
        page.setting(
            &self.config,
            AppConfigField::ReplaceNativeVolumeFlyout,
            caps.input_hooks && caps.volume_control,
        );
        page.setting(
            &self.config,
            AppConfigField::BrightnessOverlayEnabled,
            caps.brightness_control,
        );
        page.group_end();
        page.section(tr("section_activities_reminders"));
        page.group_start();
        page.setting(&self.config, AppConfigField::WaterReminderMinutes, true);
        page.setting(&self.config, AppConfigField::StretchReminderMinutes, true);
        page.group_end();
        page
    }

    pub(crate) fn handle_notch_page_click(&mut self, input: PageInput, which: usize) {
        let page = match which {
            0 => self.build_notch_page(),
            1 => self.build_top_bar_page(),
            _ => self.build_activities_page(),
        };
        let result = input.hit_test(&page);
        self.handle_setting_click(&page, &result, input);
    }
}
