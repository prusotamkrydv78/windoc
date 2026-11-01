//! Finding your way around Settings: a search box that searches every setting on every page,
//! and a sidebar of short, focused pages.
//!
//! The many sections of the General page are split into five views (Appearance, Behaviour,
//! Robot, Panel tools, Data & updates). A view shows some of General's sections; clicks are
//! mapped back to the full General page, so every existing control keeps working as is.

use std::ops::Range;
use std::time::Instant;

use winisland_core::i18n::tr;
use winisland_platform::Key;
use winisland_render::{Painter, Point, Radius, Rect, Rgba, StrokeCap};

use crate::ui::components::icon::{Icon, draw_icon};
use crate::utils::color::SettingsTheme;
use crate::utils::settings_ui::items::{SIDEBAR_PAD, SIDEBAR_SEL_RADIUS, SettingsItem};
use crate::utils::settings_ui::{SettingsPainter, settings_color};

use super::pages::PageInput;
use super::{
    ACTIVITIES_PAGE_INDEX, GENERAL_PAGE_INDEX, MUSIC_PAGE_INDEX, NOTCH_PAGE_INDEX,
    PLUGINS_PAGE_INDEX, SETTINGS_HEADER_H, SIDEBAR_ROW_GAP, SIDEBAR_ROW_H, SIDEBAR_START_Y,
    SIDEBAR_W, SettingsApp, TOP_BAR_PAGE_INDEX, WIDGETS_PAGE_INDEX,
};

/// A focused slice of the General page.
pub(crate) struct GeneralView {
    title: &'static str,
    subtitle: &'static str,
    icon: Icon,
    /// Section header keys shown in this view.
    sections: &'static [&'static str],
}

pub(crate) const GENERAL_VIEWS: [GeneralView; 5] = [
    GeneralView {
        title: "nav_appearance",
        subtitle: "nav_appearance_hint",
        icon: Icon::Sparkle,
        sections: &["section_appearance", "section_effects"],
    },
    GeneralView {
        title: "nav_behaviour",
        subtitle: "nav_behaviour_hint",
        icon: Icon::Settings,
        sections: &[
            "section_behavior",
            "section_modules",
            "section_you",
            "section_status_message",
        ],
    },
    GeneralView {
        title: "nav_robot",
        subtitle: "nav_robot_hint",
        icon: Icon::Pet,
        sections: &["section_robot"],
    },
    GeneralView {
        title: "nav_panel_tools",
        subtitle: "nav_panel_tools_hint",
        icon: Icon::Clipboard,
        sections: &[
            "section_screen_time",
            "section_clipboard",
            "section_weather",
            "section_ai_usage",
            "section_wellness",
            "section_shelves",
            "section_calendar_feeds",
            "section_links",
            "section_world_clocks",
        ],
    },
    GeneralView {
        title: "nav_data",
        subtitle: "nav_data_hint",
        icon: Icon::Shelf,
        sections: &[
            "section_backup",
            "section_local_data",
            "section_updates",
            "section_start_fresh",
        ],
    },
];

/// The view that takes General sections no view lists, so nothing is ever hidden.
const FALLBACK_VIEW: usize = 1;

/// One row of the sidebar's built-in part.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SidebarRow {
    General(usize),
    Page(usize),
}

/// The Plugins page is left out: Notchy's features are built in.
pub(crate) const BUILTIN_ROWS: [SidebarRow; 10] = [
    SidebarRow::General(0),
    SidebarRow::General(1),
    SidebarRow::General(2),
    SidebarRow::General(3),
    SidebarRow::General(4),
    SidebarRow::Page(NOTCH_PAGE_INDEX),
    SidebarRow::Page(TOP_BAR_PAGE_INDEX),
    SidebarRow::Page(ACTIVITIES_PAGE_INDEX),
    SidebarRow::Page(MUSIC_PAGE_INDEX),
    SidebarRow::Page(WIDGETS_PAGE_INDEX),
];

/// `sidebar_hover` values for built-in rows start here, clear of page indices.
pub(crate) const BUILTIN_HOVER_BASE: i32 = 10_000;

/// The search box in the sidebar.
const SEARCH_Y: f32 = 50.0;
const SEARCH_H: f32 = 32.0;
const SEARCH_MAX_CHARS: usize = 40;
/// How long a found setting glows after jumping to it.
const FLASH_SECONDS: f32 = 1.6;

fn row_y(index: usize) -> f32 {
    SIDEBAR_START_Y + index as f32 * (SIDEBAR_ROW_H + SIDEBAR_ROW_GAP)
}

fn search_rect() -> Rect {
    Rect::from_xywh(SIDEBAR_PAD, SEARCH_Y, SIDEBAR_W - SIDEBAR_PAD * 2.0, SEARCH_H)
}

/// The label a row shows, for search; `None` for rows without text.
fn item_label(item: &SettingsItem) -> Option<&str> {
    match item {
        SettingsItem::SectionHeader { label }
        | SettingsItem::RowSwitch { label, .. }
        | SettingsItem::RowStepper { label, .. }
        | SettingsItem::RowSourceSelect { label, .. }
        | SettingsItem::RowButton { label, .. }
        | SettingsItem::RowTextField { label, .. }
        | SettingsItem::RowFontPicker { label, .. }
        | SettingsItem::RowLabel { label }
        | SettingsItem::RowFolderPicker { label, .. } => Some(label),
        _ => None,
    }
}

/// Where a search result lives and how to get there.
#[derive(Clone)]
pub(crate) struct SearchHit {
    page: usize,
    view: usize,
    /// The row's index in the page's full item list.
    item: usize,
}

/// Section ranges of the full General list, as `(section key, range)`.
fn general_sections(items: &[SettingsItem]) -> Vec<(Option<&'static str>, Range<usize>)> {
    let keys: Vec<(&'static str, String)> = GENERAL_VIEWS
        .iter()
        .flat_map(|view| view.sections.iter())
        .map(|key| (*key, tr(key)))
        .collect();
    let mut sections = Vec::new();
    let mut start = 0;
    let mut key = None;
    for (index, item) in items.iter().enumerate() {
        if let SettingsItem::SectionHeader { label } = item {
            if index > start {
                sections.push((key, start..index));
            }
            start = index;
            key = keys
                .iter()
                .find(|(_, text)| text == label)
                .map(|(key, _)| *key);
        }
    }
    if items.len() > start {
        sections.push((key, start..items.len()));
    }
    sections
}

/// Which view a General section belongs to.
fn view_of(key: Option<&'static str>) -> usize {
    key.and_then(|key| {
        GENERAL_VIEWS
            .iter()
            .position(|view| view.sections.contains(&key))
    })
    .unwrap_or(FALLBACK_VIEW)
}

impl SettingsApp {
    // ---- The General views ------------------------------------------------------------

    /// The ranges of the full General list shown in the current view, in order.
    fn general_view_ranges(&self, items: &[SettingsItem]) -> Vec<Range<usize>> {
        general_sections(items)
            .into_iter()
            .filter(|(key, _)| view_of(*key) == self.general_view)
            .map(|(_, range)| range)
            .collect()
    }

    /// The current view's rows, from the full General list.
    pub(crate) fn build_general_view_items(&self) -> Vec<SettingsItem> {
        let items = self.build_general_items();
        let ranges = self.general_view_ranges(&items);
        let mut items: Vec<Option<SettingsItem>> = items.into_iter().map(Some).collect();
        let mut shown: Vec<SettingsItem> = ranges
            .into_iter()
            .flat_map(|range| range.collect::<Vec<_>>())
            .filter_map(|index| items[index].take())
            .collect();
        // The page title already names the first section ("Appearance" under "Appearance"):
        // keep its spacing but drop the repeated words.
        let title = tr(GENERAL_VIEWS[self.general_view.min(GENERAL_VIEWS.len() - 1)].title);
        if let Some(SettingsItem::SectionHeader { label }) = shown.first_mut()
            && *label == title
        {
            label.clear();
        }
        shown
    }

    /// A click in a General view, mapped to the full General page: the scroll is offset for
    /// the section clicked so hit tests, popups and text fields all land on the right row.
    pub(crate) fn handle_general_view_click(&mut self, input: PageInput) {
        let items = self.build_general_items();
        let heights: Vec<f32> = items.iter().map(SettingsItem::height).collect();
        let full_top = |index: usize| heights[..index].iter().sum::<f32>();
        let mut shown_top = 0.0;
        for range in self.general_view_ranges(&items) {
            let height: f32 = heights[range.clone()].iter().sum();
            let local = input.y - input.start_y - shown_top;
            if (0.0..height).contains(&local) {
                let shift = full_top(range.start) - shown_top;
                let saved = self.scroll_y;
                self.scroll_y += shift;
                self.handle_general_click(PageInput {
                    y: input.y + shift,
                    ..input
                });
                self.scroll_y = saved;
                return;
            }
            shown_top += height;
        }
    }

    // ---- The sidebar --------------------------------------------------------------------

    pub(crate) fn builtin_row_count() -> usize {
        BUILTIN_ROWS.len()
    }

    fn active_row(&self) -> Option<SidebarRow> {
        if self.search_active() {
            return None;
        }
        Some(if self.active_page == GENERAL_PAGE_INDEX {
            SidebarRow::General(self.general_view)
        } else {
            SidebarRow::Page(self.active_page)
        })
    }

    /// The built-in row under `(x, y)`.
    pub(crate) fn sidebar_row_at(&self, x: f32, y: f32) -> Option<usize> {
        if !(SIDEBAR_PAD..=SIDEBAR_W - SIDEBAR_PAD).contains(&x) {
            return None;
        }
        (0..BUILTIN_ROWS.len()).find(|index| (row_y(*index)..=row_y(*index) + SIDEBAR_ROW_H).contains(&y))
    }

    pub(crate) fn visit_row(&mut self, index: usize) {
        self.end_search();
        match BUILTIN_ROWS[index] {
            SidebarRow::General(view) => {
                let changed = self.general_view != view;
                self.general_view = view;
                if self.active_page == GENERAL_PAGE_INDEX && changed {
                    self.scroll_y = 0.0;
                    self.target_scroll_y = 0.0;
                    self.mark_items_dirty();
                    self.request_redraw();
                } else {
                    self.visit_page(GENERAL_PAGE_INDEX);
                }
            }
            SidebarRow::Page(page) => self.visit_page(page),
        }
    }

    /// The built-in rows: an icon and a name each, the current one highlighted.
    pub(crate) fn draw_builtin_rows(&self, painter: Painter<'_>, theme: &SettingsTheme) {
        let active = self.active_row();
        for (index, row) in BUILTIN_ROWS.iter().enumerate() {
            let y = row_y(index);
            // A little gap and a divider between the General views and the other pages.
            if index == GENERAL_VIEWS.len() {
                painter.fill_rect(
                    Rect::from_xywh(SIDEBAR_PAD + 8.0, y - 2.0, SIDEBAR_W - SIDEBAR_PAD * 2.0 - 16.0, 0.5),
                    settings_color(theme.separator),
                );
            }
            let rect = Rect::from_xywh(SIDEBAR_PAD, y, SIDEBAR_W - SIDEBAR_PAD * 2.0, SIDEBAR_ROW_H);
            let selected = active == Some(*row);
            let hovered = self.sidebar_hover == BUILTIN_HOVER_BASE + index as i32;
            let text = if selected {
                painter.fill_round_rect(
                    rect,
                    Radius::uniform(SIDEBAR_SEL_RADIUS),
                    settings_color(if self.focused {
                        theme.selection_bg
                    } else {
                        theme.card_highlight
                    }),
                );
                if self.focused && !self.is_light {
                    Rgba::WHITE
                } else if self.focused {
                    settings_color(theme.selection_text)
                } else {
                    settings_color(theme.text_pri)
                }
            } else {
                if hovered {
                    painter.fill_round_rect(
                        rect,
                        Radius::uniform(SIDEBAR_SEL_RADIUS),
                        settings_color(theme.sidebar_hover),
                    );
                }
                settings_color(if hovered { theme.text_pri } else { theme.text_sec })
            };
            let (icon, label) = match row {
                SidebarRow::General(view) => (GENERAL_VIEWS[*view].icon, tr(GENERAL_VIEWS[*view].title)),
                SidebarRow::Page(page) => page_icon_and_label(*page),
            };
            let icon_rect = Rect::from_xywh(SIDEBAR_PAD + 9.0, y + 7.0, 20.0, 20.0);
            if icon == Icon::Pet {
                // The robot's own little face for its page.
                draw_robot_glyph(painter, icon_rect, text);
            } else {
                draw_icon(painter, icon, icon_rect.inset(1.5), text);
            }
            SettingsPainter::new(painter).text(&label, (SIDEBAR_PAD + 38.0, y + 22.0), 13.0, selected, text);
        }
    }

    // ---- Search -------------------------------------------------------------------------

    pub(crate) fn search_active(&self) -> bool {
        !self.search_query.trim().is_empty()
    }

    fn end_search(&mut self) {
        if self.search_focused || !self.search_query.is_empty() {
            self.search_focused = false;
            self.search_query.clear();
            self.search_hits.clear();
            self.mark_items_dirty();
        }
    }

    /// Whether `(x, y)` is on the search box.
    pub(crate) fn search_box_at(x: f32, y: f32) -> bool {
        search_rect().contains(Point::new(x, y))
    }

    pub(crate) fn focus_search(&mut self) {
        self.commit_number_input();
        self.search_focused = true;
        self.request_redraw();
    }

    /// Typing while the search box has focus. `true` when the key was used.
    pub(crate) fn handle_search_key(&mut self, key: &Key) -> bool {
        if !self.search_focused {
            // Typing anywhere (outside a field) starts a search.
            if let Key::Character(text) = key
                && self.number_input.is_none()
                && self.popup.is_none()
                && !text.trim().is_empty()
            {
                self.search_focused = true;
            } else {
                return false;
            }
        }
        match key {
            Key::Character(text) => {
                if self.search_query.chars().count() < SEARCH_MAX_CHARS {
                    self.search_query.push_str(text);
                }
            }
            Key::Backspace => {
                self.search_query.pop();
            }
            Key::Escape => {
                self.end_search();
            }
            Key::Enter => {
                // Enter opens the first result.
                if let Some(hit) = self.search_hits.first().cloned() {
                    self.open_search_hit(&hit);
                }
                return true;
            }
            _ => return false,
        }
        self.scroll_y = 0.0;
        self.target_scroll_y = 0.0;
        self.mark_items_dirty();
        self.request_redraw();
        true
    }

    /// Every built-in page's rows, with the page and (for General) view they belong to.
    fn searchable_pages(&self) -> Vec<(usize, Vec<SettingsItem>)> {
        vec![
            (GENERAL_PAGE_INDEX, self.build_general_items()),
            (NOTCH_PAGE_INDEX, self.build_notch_page().into_items()),
            (TOP_BAR_PAGE_INDEX, self.build_top_bar_page().into_items()),
            (ACTIVITIES_PAGE_INDEX, self.build_activities_page().into_items()),
            (MUSIC_PAGE_INDEX, self.build_music_items()),
        ]
    }

    /// The result rows for the current query: each matching setting as a button naming
    /// where it lives. Also records where each leads.
    pub(crate) fn build_search_items(&mut self) -> Vec<SettingsItem> {
        let query = self.search_query.trim().to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        let mut hits = Vec::new();
        let mut rows = Vec::new();
        for (page, items) in self.searchable_pages() {
            let sections = if page == GENERAL_PAGE_INDEX {
                general_sections(&items)
            } else {
                vec![(None, 0..items.len())]
            };
            let mut section_label = String::new();
            for (index, item) in items.iter().enumerate() {
                if let SettingsItem::SectionHeader { label } = item {
                    section_label.clone_from(label);
                    continue;
                }
                let Some(label) = item_label(item) else {
                    continue;
                };
                let haystack = format!("{label} {section_label}").to_lowercase();
                if !words.iter().all(|word| haystack.contains(word)) {
                    continue;
                }
                let view = sections
                    .iter()
                    .find(|(_, range)| range.contains(&index))
                    .map_or(FALLBACK_VIEW, |(key, _)| view_of(*key));
                let place = if page == GENERAL_PAGE_INDEX {
                    tr(GENERAL_VIEWS[view].title)
                } else {
                    page_icon_and_label(page).1
                };
                // "Robot", not "Robot › Robot", when the section is named like its page.
                let place = if section_label.is_empty() || section_label == place {
                    place
                } else {
                    format!("{place} \u{203a} {section_label}")
                };
                rows.push(SettingsItem::RowButton {
                    label: label.to_string(),
                    btn_label: place,
                    enabled: true,
                });
                hits.push(SearchHit {
                    page,
                    view,
                    item: index,
                });
                if hits.len() >= 60 {
                    break;
                }
            }
        }
        self.search_hits = hits;
        let mut items = vec![SettingsItem::SectionHeader {
            label: if rows.is_empty() {
                tr("search_no_results")
            } else {
                winisland_core::i18n::tr_args("search_results", &[&rows.len().to_string()])
            },
        }];
        if !rows.is_empty() {
            items.push(SettingsItem::GroupStart);
            items.extend(rows);
            items.push(SettingsItem::GroupEnd);
        }
        items
    }

    /// A click on the results list.
    pub(crate) fn handle_search_click(&mut self, input: PageInput) {
        let mut top = input.start_y;
        let mut hit_index = 0;
        for item in &self.cached_items {
            let height = item.height();
            if matches!(item, SettingsItem::RowButton { .. }) {
                if (top..top + height).contains(&input.y) {
                    if let Some(hit) = self.search_hits.get(hit_index).cloned() {
                        self.open_search_hit(&hit);
                    }
                    return;
                }
                hit_index += 1;
            }
            top += height;
        }
    }

    /// Goes to a found setting, scrolls it into view and makes it glow for a moment.
    fn open_search_hit(&mut self, hit: &SearchHit) {
        self.end_search();
        if hit.page == GENERAL_PAGE_INDEX {
            self.general_view = hit.view;
        }
        if self.active_page != hit.page {
            self.visit_page(hit.page);
        }
        self.mark_items_dirty();
        self.rebuild_items_cache();
        // Where the row sits on the page now shown.
        let shown_index = if hit.page == GENERAL_PAGE_INDEX {
            let items = self.build_general_items();
            let mut shown = 0;
            let mut found = None;
            for range in self.general_view_ranges(&items) {
                if range.contains(&hit.item) {
                    found = Some(shown + hit.item - range.start);
                    break;
                }
                shown += range.len();
            }
            found.unwrap_or(0)
        } else {
            hit.item
        };
        let row_top: f32 = self.cached_items[..shown_index.min(self.cached_items.len())]
            .iter()
            .map(SettingsItem::height)
            .sum();
        let target = (row_top - 80.0).clamp(0.0, self.cached_max_scroll);
        self.target_scroll_y = target;
        self.flash = Some((shown_index, Instant::now()));
        self.request_redraw();
    }

    /// The glow over a row just found by search, in content coordinates.
    pub(crate) fn draw_search_flash(&self, painter: Painter<'_>, width: f32, theme: &SettingsTheme) -> bool {
        let Some((index, at)) = self.flash else {
            return false;
        };
        let age = at.elapsed().as_secs_f32() / FLASH_SECONDS;
        if age >= 1.0 || index >= self.cached_items.len() {
            return false;
        }
        let top: f32 = SETTINGS_HEADER_H
            + self.cached_items[..index]
                .iter()
                .map(SettingsItem::height)
                .sum::<f32>();
        let height = self.cached_items[index].height();
        let glow = (1.0 - age) * (1.0 - age);
        let accent = settings_color(theme.selection_bg);
        painter.fill_round_rect(
            Rect::from_xywh(20.0, top, width - 40.0, height),
            Radius::uniform(10.0),
            accent.with_alpha((70.0 * glow) as u8),
        );
        true
    }

    /// The search box at the top of the sidebar.
    pub(crate) fn draw_search_box(&self, painter: Painter<'_>, theme: &SettingsTheme) {
        let rect = search_rect();
        painter.fill_round_rect(
            rect,
            Radius::uniform(9.0),
            settings_color(if self.search_focused {
                theme.card_highlight
            } else {
                theme.sidebar_hover
            }),
        );
        if self.search_focused {
            painter.stroke_round_rect(
                rect.inset(0.5),
                Radius::uniform(9.0),
                1.0,
                settings_color(theme.selection_bg),
            );
        }
        draw_icon(
            painter,
            Icon::Search,
            Rect::from_xywh(rect.left + 9.0, rect.center_y() - 7.5, 15.0, 15.0),
            settings_color(theme.text_sec),
        );
        let text_x = rect.left + 31.0;
        let baseline = rect.center_y() + 4.5;
        if self.search_query.is_empty() {
            SettingsPainter::new(painter).text(
                &tr("search_settings"),
                (text_x, baseline),
                12.5,
                false,
                settings_color(theme.text_sec),
            );
        } else {
            SettingsPainter::new(painter).text(
                &self.search_query,
                (text_x, baseline),
                12.5,
                false,
                settings_color(theme.text_pri),
            );
        }
        if self.search_focused && self.frame_count % 60 < 30 {
            let width = winisland_render::text::FontManager::global().measure_text_cached(
                &self.search_query,
                12.5,
                winisland_render::FontStyle::normal(),
            );
            painter.stroke_line(
                Point::new(text_x + width + 1.0, rect.center_y() - 7.0),
                Point::new(text_x + width + 1.0, rect.center_y() + 7.0),
                1.2,
                settings_color(theme.text_pri),
                StrokeCap::Butt,
            );
        }
    }

    /// The title and one-line description for the page header.
    pub(crate) fn general_view_title(&self) -> (String, String) {
        if self.search_active() {
            return (tr("search_title"), String::new());
        }
        let view = &GENERAL_VIEWS[self.general_view.min(GENERAL_VIEWS.len() - 1)];
        (tr(view.title), tr(view.subtitle))
    }
}

fn page_icon_and_label(page: usize) -> (Icon, String) {
    match page {
        NOTCH_PAGE_INDEX => (Icon::Home, tr("tab_notch")),
        TOP_BAR_PAGE_INDEX => (Icon::More, tr("tab_top_bar")),
        ACTIVITIES_PAGE_INDEX => (Icon::Bolt, tr("tab_activities")),
        MUSIC_PAGE_INDEX => (Icon::Music, tr("tab_music")),
        WIDGETS_PAGE_INDEX => (Icon::Grid, tr("tab_widgets")),
        PLUGINS_PAGE_INDEX => (Icon::Link, tr("tab_plugins")),
        _ => (Icon::Settings, tr("tab_general")),
    }
}

/// A tiny robot face for the Robot page's row.
fn draw_robot_glyph(painter: Painter<'_>, rect: Rect, colour: Rgba) {
    let head = Rect::from_xywh(rect.left + 2.0, rect.top + 3.0, rect.width() - 4.0, rect.height() - 6.0);
    painter.stroke_round_rect(head, Radius::uniform(4.0), 1.6, colour);
    for side in [-1.0f32, 1.0] {
        painter.fill_circle(
            Point::new(head.center_x() + side * 3.2, head.center_y()),
            1.4,
            colour,
        );
    }
}
