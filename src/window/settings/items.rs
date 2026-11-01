use crate::utils::settings_ui::content_height;
use crate::utils::settings_ui::items::SettingsItem;

use super::{
    ACTIVITIES_PAGE_INDEX, GENERAL_PAGE_INDEX, MUSIC_PAGE_INDEX, NOTCH_PAGE_INDEX,
    PLUGINS_PAGE_INDEX, SETTINGS_HEADER_H, SettingsApp, TOP_BAR_PAGE_INDEX, WIDGETS_PAGE_INDEX,
};

impl SettingsApp {
    pub(crate) fn build_current_items(&self) -> Vec<SettingsItem> {
        match self.active_page {
            GENERAL_PAGE_INDEX => self.build_general_view_items(),
            NOTCH_PAGE_INDEX => self.build_notch_page().into_items(),
            TOP_BAR_PAGE_INDEX => self.build_top_bar_page().into_items(),
            ACTIVITIES_PAGE_INDEX => self.build_activities_page().into_items(),
            MUSIC_PAGE_INDEX => self.build_music_items(),
            WIDGETS_PAGE_INDEX => self.build_widget_items(),
            PLUGINS_PAGE_INDEX => self.build_plugin_items(),
            page if page == self.about_page_index() => self.build_about_items(),
            _ => self.build_plugin_settings_items(),
        }
    }

    pub(crate) fn rebuild_items_cache(&mut self) {
        // Descriptions wrap to the page's current width.
        crate::utils::settings_ui::items::set_label_wrap_width(self.content_width());
        // A search shows its results on any page.
        self.cached_items = if self.search_active() {
            self.build_search_items()
        } else {
            self.build_current_items()
        };
        let switch_states: Vec<bool> = self
            .cached_items
            .iter()
            .filter_map(|item| match item {
                SettingsItem::RowSwitch { on, .. } => Some(*on),
                _ => None,
            })
            .collect();
        // Each General view and the search results animate their own switches.
        let switch_context = (
            self.active_page,
            if self.search_active() { usize::MAX } else { self.general_view },
        );
        if self.switch_anim_context != switch_context
            || self.switch_anim.len() != switch_states.len()
        {
            self.switch_anim = crate::utils::settings_ui::SwitchAnimator::new(&switch_states);
            self.switch_anim_context = switch_context;
        } else {
            self.switch_anim.set_targets(&switch_states);
        }
        self.cached_content_height = content_height(&self.cached_items, SETTINGS_HEADER_H);
        let view_h = self.logical_window_size().1;
        self.cached_max_scroll = (self.cached_content_height - view_h + 20.0).max(0.0);
        self.items_dirty = false;
    }

    pub(crate) fn ensure_items_cache(&mut self) {
        if self.items_dirty {
            self.rebuild_items_cache();
        }
    }

    pub(crate) fn mark_items_dirty(&mut self) {
        self.items_dirty = true;
    }
}
