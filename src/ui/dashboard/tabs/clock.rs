//! Clock tab: the local time large on the left with its display settings, and world
//! clocks on the right.
//!
//! The settings (12/24-hour, seconds, date style) belong to this tab and are saved to
//! `clock_tab.json`. World clocks use the zones chosen in Settings, or four default cities
//! when none are chosen. Each zone's offset comes from Windows, so daylight saving is
//! honoured; only if Windows does not know a zone do we fall back to its fixed standard
//! offset, which is then off by an hour during daylight saving.

pub(super) mod widgets;

use std::cell::RefCell;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use winisland_platform::Key;
use winisland_render::tokens::{SemanticColors, TextStyle, radius, space, typography};
use winisland_render::{Painter, Point, Radius, Rect};

use super::TabContext;
use crate::ui::components::icon::{Icon, draw_icon};
use crate::ui::components::text::{Align, draw_text};
use crate::ui::dashboard::DashboardHit;
use widgets::{
    CARD_PAD, GLOBE, draw_glyph, draw_right, draw_segmented, draw_toggle, segment_rects,
    text_width, toggle_rect,
};

const FILE_NAME: &str = "clock_tab.json";
const MAX_FILE_BYTES: u64 = 4 * 1024;

const ID_12H: u16 = 100;
const ID_24H: u16 = 101;
const ID_SECONDS: u16 = 102;
const ID_DATE_FULL: u16 = 103;
const ID_DATE_SHORT: u16 = 104;
const ID_DATE_COMPACT: u16 = 105;
const ID_ADD_CITY: u16 = 106;

const HOUR_LABELS: [&str; 2] = ["12h", "24h"];
const DATE_LABELS: [&str; 3] = ["Full", "Short", "Compact"];

const HERO: TextStyle = TextStyle {
    size: 42.0,
    weight: 650,
    tabular: true,
};
const ROW_TIME: TextStyle = TextStyle {
    size: 17.0,
    weight: 600,
    tabular: true,
};
const CITY: TextStyle = TextStyle {
    size: 14.0,
    weight: 500,
    tabular: false,
};
const SETTING_ROW_H: f32 = 30.0;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DateStyle {
    #[default]
    Full,
    Short,
    Compact,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(default)]
struct Settings {
    twelve_hour: bool,
    show_seconds: bool,
    date_style: DateStyle,
}

/// The local zone and the world clock rows, refreshed about once a minute.
struct Zones {
    refreshed: Option<Instant>,
    keys: Vec<String>,
    local_name: String,
    local_abbr: String,
    /// Minutes east of UTC.
    local_offset: i32,
    cities: Vec<City>,
}

struct City {
    name: String,
    abbr: String,
    offset: i32,
}

thread_local! {
    static SETTINGS: RefCell<Settings> = RefCell::new(load_settings());
    static ZONES: RefCell<Zones> = const {
        RefCell::new(Zones {
            refreshed: None,
            keys: Vec::new(),
            local_name: String::new(),
            local_abbr: String::new(),
            local_offset: 0,
            cities: Vec::new(),
        })
    };
}

fn load_settings() -> Settings {
    match crate::core::json_store::read::<Settings>(FILE_NAME, MAX_FILE_BYTES) {
        Ok(Some(settings)) => settings,
        Ok(None) => Settings::default(),
        Err(error) => {
            log::warn!("Ignoring saved Clock settings: {error}");
            Settings::default()
        }
    }
}

fn update_settings(change: impl FnOnce(&mut Settings)) {
    let settings = SETTINGS.with(|cell| {
        let mut settings = cell.borrow_mut();
        change(&mut settings);
        *settings
    });
    if let Err(error) = crate::core::json_store::write(FILE_NAME, &settings) {
        log::warn!("Cannot save Clock settings: {error}");
    }
}

/// Known Windows zones: key, city, standard and daylight abbreviations, standard offset.
const KNOWN: [(&str, &str, &str, &str, i32); 14] = [
    ("Eastern Standard Time", "New York", "EST", "EDT", -300),
    ("GMT Standard Time", "London", "GMT", "BST", 0),
    ("Tokyo Standard Time", "Tokyo", "JST", "JST", 540),
    ("AUS Eastern Standard Time", "Sydney", "AEST", "AEDT", 600),
    ("Pacific Standard Time", "Los Angeles", "PST", "PDT", -480),
    ("Central Standard Time", "Chicago", "CST", "CDT", -360),
    ("Mountain Standard Time", "Denver", "MST", "MDT", -420),
    ("W. Europe Standard Time", "Berlin", "CET", "CEST", 60),
    ("Romance Standard Time", "Paris", "CET", "CEST", 60),
    ("India Standard Time", "Kolkata", "IST", "IST", 330),
    ("China Standard Time", "Shanghai", "CST", "CST", 480),
    ("Singapore Standard Time", "Singapore", "SGT", "SGT", 480),
    ("Arabian Standard Time", "Dubai", "GST", "GST", 240),
    ("UTC", "UTC", "UTC", "UTC", 0),
];

/// Shown when no world clocks are configured.
const DEFAULT_KEYS: [&str; 4] = [
    "Eastern Standard Time",
    "GMT Standard Time",
    "Tokyo Standard Time",
    "AUS Eastern Standard Time",
];

/// "India Standard Time" → "IST".
fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|word| word.chars().next())
        .filter(|c| c.is_alphabetic())
        .flat_map(char::to_uppercase)
        .collect()
}

fn utc_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Minutes east of UTC for a wall-clock time read now, rounded to 15 minutes.
fn offset_of(time: &winisland_platform::LocalDateTime, utc: i64) -> i32 {
    let wall = days_from_civil(
        i64::from(time.year),
        i64::from(time.month),
        i64::from(time.day),
    ) * 86_400
        + i64::from(time.hour) * 3600
        + i64::from(time.minute) * 60
        + i64::from(time.second);
    ((wall - utc) as f64 / 900.0).round() as i32 * 15
}

/// The local zone's display name, from Windows.
fn local_zone_name() -> Option<String> {
    use windows::Win32::System::Time::{
        DYNAMIC_TIME_ZONE_INFORMATION, GetDynamicTimeZoneInformation,
    };
    let mut info = DYNAMIC_TIME_ZONE_INFORMATION::default();
    // SAFETY: `info` is a valid, writable out-parameter.
    let id = unsafe { GetDynamicTimeZoneInformation(&mut info) };
    let name = match id {
        1 => &info.StandardName[..],
        2 => &info.DaylightName[..],
        // No daylight saving rules: the standard name applies.
        0 => &info.StandardName[..],
        _ => return None,
    };
    let end = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    let text = String::from_utf16_lossy(&name[..end]);
    (!text.trim().is_empty()).then_some(text)
}

fn refresh_zones(zones: &mut Zones, now: Instant) {
    let configured = super::super::WORLD_CLOCKS.with(|cell| cell.borrow().clone());
    let fresh = zones
        .refreshed
        .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_secs(60));
    if fresh && configured == zones.keys {
        return;
    }
    let utc = utc_seconds();
    let local = crate::platform::shell().local_datetime();
    zones.local_offset = offset_of(&local, utc);
    zones.local_name = local_zone_name().unwrap_or_else(|| "Local time".to_string());
    zones.local_abbr = if zones.local_offset == 0 && zones.local_name.contains("Coordinated") {
        "UTC".to_string()
    } else {
        initials(&zones.local_name)
    };
    let keys: Vec<String> = if configured.is_empty() {
        DEFAULT_KEYS.iter().map(|key| (*key).to_string()).collect()
    } else {
        configured.clone()
    };
    zones.cities = keys
        .iter()
        .filter_map(|key| {
            let known = KNOWN.iter().find(|entry| entry.0 == key.as_str());
            // Windows applies daylight saving; the fixed offset is only a fallback.
            let offset = winisland_platform_windows::now_in_zone(key)
                .map(|time| offset_of(&time, utc))
                .or(known.map(|entry| entry.4))?;
            let (name, abbr) = match known {
                Some(&(_, city, standard, daylight, standard_offset)) => (
                    city.to_string(),
                    if offset == standard_offset {
                        standard
                    } else {
                        daylight
                    }
                    .to_string(),
                ),
                None => (
                    key.trim_end_matches(" Standard Time").to_string(),
                    initials(key),
                ),
            };
            Some(City { name, abbr, offset })
        })
        .collect();
    zones.keys = configured;
    zones.refreshed = Some(now);
}

/// A wall-clock time at `offset` minutes east of UTC.
struct ZoneTime {
    /// Days since 1970-01-01 in that zone, to tell today from tomorrow.
    day: i64,
    hour: u32,
    minute: u32,
    second: u32,
}

fn zone_time(utc: i64, offset: i32) -> ZoneTime {
    let wall = utc + i64::from(offset) * 60;
    let seconds = wall.rem_euclid(86_400) as u32;
    ZoneTime {
        day: wall.div_euclid(86_400),
        hour: seconds / 3600,
        minute: seconds / 60 % 60,
        second: seconds % 60,
    }
}

/// "9:41" / "09:41" (plus seconds), and "AM"/"PM" in 12-hour mode.
fn format_time(time: &ZoneTime, settings: Settings, seconds: bool) -> (String, &'static str) {
    let (hour, suffix) = if settings.twelve_hour {
        (
            (time.hour + 11) % 12 + 1,
            if time.hour < 12 { "AM" } else { "PM" },
        )
    } else {
        (time.hour, "")
    };
    let text = match (settings.twelve_hour, seconds) {
        (true, true) => format!("{hour}:{:02}:{:02}", time.minute, time.second),
        (true, false) => format!("{hour}:{:02}", time.minute),
        (false, true) => format!("{hour:02}:{:02}:{:02}", time.minute, time.second),
        (false, false) => format!("{hour:02}:{:02}", time.minute),
    };
    (text, suffix)
}

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn format_date(time: &winisland_platform::LocalDateTime, style: DateStyle) -> String {
    let weekday = WEEKDAYS[usize::from(time.day_of_week % 7)];
    let month = MONTHS[usize::from(time.month.clamp(1, 12) - 1)];
    match style {
        DateStyle::Full => format!("{weekday}, {} {month} {}", time.day, time.year),
        DateStyle::Short => format!(
            "{}, {} {} {}",
            &weekday[..3],
            time.day,
            &month[..3],
            time.year
        ),
        DateStyle::Compact => format!("{}-{:02}-{:02}", time.year, time.month, time.day),
    }
}

/// "UTC+05:30", "UTC−04:00", "UTC".
fn format_utc_offset(offset: i32) -> String {
    if offset == 0 {
        return "UTC".to_string();
    }
    let sign = if offset > 0 { '+' } else { '−' };
    let minutes = offset.unsigned_abs();
    format!("UTC{sign}{:02}:{:02}", minutes / 60, minutes % 60)
}

/// "−9h 30m", "+3h", "same time".
fn format_difference(minutes: i32) -> String {
    if minutes == 0 {
        return "same time".to_string();
    }
    let sign = if minutes > 0 { '+' } else { '−' };
    let total = minutes.unsigned_abs();
    match (total / 60, total % 60) {
        (0, m) => format!("{sign}{m}m"),
        (h, 0) => format!("{sign}{h}h"),
        (h, m) => format!("{sign}{h}h {m}m"),
    }
}

// ---------------------------------------------------------------------------------------
// Layout

struct Layout {
    left: Rect,
    right: Rect,
    hour_segments: (Rect, Vec<Rect>),
    seconds_row: Rect,
    date_row: Rect,
    date_segments: (Rect, Vec<Rect>),
    add_city: Rect,
}

fn layout(grid: Rect) -> Layout {
    let gap = space::S3;
    let left_w = ((grid.width() - gap) * 0.56).round();
    let left = Rect::from_xywh(grid.left, grid.top, left_w, grid.height());
    let right = Rect::from_ltrb(left.right + gap, grid.top, grid.right, grid.bottom);
    let inner_left = left.left + CARD_PAD;
    let inner_right = left.right - CARD_PAD;
    let header_center = left.top + CARD_PAD + 6.0;
    let hour_segments = segment_rects(inner_right, header_center, &HOUR_LABELS);
    let rows_top = left.bottom - 10.0 - SETTING_ROW_H * 2.0;
    let seconds_row = Rect::from_ltrb(inner_left, rows_top, inner_right, rows_top + SETTING_ROW_H);
    let date_row = Rect::from_ltrb(
        inner_left,
        seconds_row.bottom,
        inner_right,
        seconds_row.bottom + SETTING_ROW_H,
    );
    let date_segments = segment_rects(inner_right, date_row.center_y(), &DATE_LABELS);
    let add_label = "+ Add city";
    let add_w = text_width(add_label, typography::LABEL) + 16.0;
    let add_city = Rect::from_xywh(
        right.right - CARD_PAD - add_w + 8.0,
        right.top + CARD_PAD - 6.0,
        add_w,
        24.0,
    );
    Layout {
        left,
        right,
        hour_segments,
        seconds_row,
        date_row,
        date_segments,
        add_city,
    }
}

// ---------------------------------------------------------------------------------------
// Tab entry points

/// Draws the tab. Return `false` to fall back to the legacy page.
pub fn draw(ctx: &TabContext<'_>) -> bool {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let layout = layout(ctx.grid);
    let settings = SETTINGS.with(|cell| *cell.borrow());
    let hovered = match ctx.hover {
        Some(DashboardHit::Tab(id)) => Some(id),
        _ => None,
    };
    for card in [layout.left, layout.right] {
        painter.fill_round_rect(
            card,
            Radius::uniform(radius::CARD + 6.0),
            colors.surface_raised,
        );
    }
    ZONES.with(|cell| {
        let mut zones = cell.borrow_mut();
        refresh_zones(&mut zones, ctx.now);
        draw_local(painter, &layout, &zones, settings, hovered, colors);
        draw_world(painter, &layout, &zones, settings, hovered, colors);
    });
    true
}

fn card_title(
    painter: Painter<'_>,
    card: Rect,
    icon: Option<Icon>,
    title: &str,
    colors: &SemanticColors,
) {
    let top = card.top + CARD_PAD - 3.0;
    let icon_rect = Rect::from_xywh(card.left + CARD_PAD, top + 1.0, 16.0, 16.0);
    match icon {
        Some(icon) => draw_icon(painter, icon, icon_rect, colors.text_secondary),
        None => draw_glyph(painter, GLOBE, icon_rect, colors.text_secondary, 1.8),
    }
    draw_text(
        painter,
        title,
        typography::LABEL,
        Rect::from_xywh(icon_rect.right + 8.0, top, 160.0, 18.0),
        Align::Start,
        colors.text_secondary,
    );
}

fn draw_local(
    painter: Painter<'_>,
    layout: &Layout,
    zones: &Zones,
    settings: Settings,
    hovered: Option<u16>,
    colors: &SemanticColors,
) {
    let card = layout.left;
    let inner = Rect::from_ltrb(
        card.left + CARD_PAD,
        card.top,
        card.right - CARD_PAD,
        card.bottom,
    );
    card_title(painter, card, Some(Icon::Clock), "Clock", colors);
    let hour_hover = match hovered {
        Some(ID_12H) => Some(0),
        Some(ID_24H) => Some(1),
        _ => None,
    };
    draw_segmented(
        painter,
        layout.hour_segments.0,
        &layout.hour_segments.1,
        &HOUR_LABELS,
        usize::from(!settings.twelve_hour),
        hour_hover,
        colors,
    );

    let local = crate::platform::shell().local_datetime();
    let now = ZoneTime {
        day: 0,
        hour: u32::from(local.hour),
        minute: u32::from(local.minute),
        second: u32::from(local.second),
    };
    let (time, suffix) = format_time(&now, settings, settings.show_seconds);
    let time_rect = Rect::from_xywh(inner.left, card.top + 36.0, inner.width(), 50.0);
    draw_text(
        painter,
        &time,
        HERO,
        time_rect,
        Align::Start,
        colors.text_primary,
    );
    if !suffix.is_empty() {
        let width = text_width(&time, HERO);
        // Sits on the hero's baseline: same baseline formula as `draw_text`.
        let suffix_style = TextStyle {
            size: 17.0,
            weight: 600,
            tabular: false,
        };
        let center = time_rect.center_y() + (HERO.size - suffix_style.size) * 0.35;
        draw_text(
            painter,
            suffix,
            suffix_style,
            Rect::from_xywh(inner.left + width + 6.0, center - 10.0, 40.0, 20.0),
            Align::Start,
            colors.text_secondary,
        );
    }
    draw_text(
        painter,
        &format_date(&local, settings.date_style),
        typography::BODY,
        Rect::from_xywh(inner.left, time_rect.bottom, inner.width(), 20.0),
        Align::Start,
        colors.text_primary.with_alpha(215),
    );
    let zone_line = format!(
        "{} · {} · {}",
        zones.local_abbr,
        format_utc_offset(zones.local_offset),
        zones.local_name
    );
    draw_text(
        painter,
        &zone_line,
        typography::LABEL,
        Rect::from_xywh(inner.left, time_rect.bottom + 21.0, inner.width(), 16.0),
        Align::Start,
        colors.text_secondary,
    );

    // Settings rows under a hairline.
    painter.fill_rect(
        Rect::from_xywh(inner.left, layout.seconds_row.top - 4.0, inner.width(), 1.0),
        colors.border_subtle,
    );
    let seconds_hot = hovered == Some(ID_SECONDS);
    draw_text(
        painter,
        "Show seconds",
        typography::BODY,
        layout.seconds_row,
        Align::Start,
        if seconds_hot {
            colors.text_primary
        } else {
            colors.text_secondary
        },
    );
    draw_toggle(
        painter,
        toggle_rect(layout.seconds_row),
        settings.show_seconds,
        seconds_hot,
        colors,
    );
    draw_text(
        painter,
        "Date style",
        typography::BODY,
        layout.date_row,
        Align::Start,
        colors.text_secondary,
    );
    let date_hover = match hovered {
        Some(ID_DATE_FULL) => Some(0),
        Some(ID_DATE_SHORT) => Some(1),
        Some(ID_DATE_COMPACT) => Some(2),
        _ => None,
    };
    draw_segmented(
        painter,
        layout.date_segments.0,
        &layout.date_segments.1,
        &DATE_LABELS,
        settings.date_style as usize,
        date_hover,
        colors,
    );
}

fn draw_world(
    painter: Painter<'_>,
    layout: &Layout,
    zones: &Zones,
    settings: Settings,
    hovered: Option<u16>,
    colors: &SemanticColors,
) {
    let card = layout.right;
    card_title(painter, card, None, "World clocks", colors);
    let add_hot = hovered == Some(ID_ADD_CITY);
    if add_hot {
        painter.fill_round_rect(
            layout.add_city,
            Radius::uniform(layout.add_city.height() / 2.0),
            colors.surface_interactive,
        );
    }
    draw_text(
        painter,
        "+ Add city",
        typography::LABEL,
        layout.add_city,
        Align::Center,
        if add_hot {
            colors.text_primary
        } else {
            colors.text_secondary
        },
    );

    let utc = utc_seconds();
    let local = zone_time(utc, zones.local_offset);
    let list = Rect::from_ltrb(
        card.left + CARD_PAD,
        card.top + CARD_PAD + 22.0,
        card.right - CARD_PAD,
        card.bottom - 8.0,
    );
    let count = zones.cities.len() + 1;
    let row_h = (list.height() / count as f32).clamp(32.0, 44.0);
    let rows = ((list.height() / row_h).floor() as usize).min(count);
    for index in 0..rows {
        let row = Rect::from_xywh(
            list.left,
            list.top + index as f32 * row_h,
            list.width(),
            row_h,
        );
        if index > 0 {
            painter.fill_rect(
                Rect::from_xywh(row.left, row.top, row.width(), 1.0),
                colors.border_subtle,
            );
        }
        let (name, detail, time) = if index == 0 {
            (
                "Local".to_string(),
                zones.local_abbr.clone(),
                local_time_text(&local, settings),
            )
        } else {
            let city = &zones.cities[index - 1];
            let time = zone_time(utc, city.offset);
            let mut detail = format!(
                "{} · {}",
                city.abbr,
                format_difference(city.offset - zones.local_offset)
            );
            match time.day - local.day {
                1 => detail.push_str(", tomorrow"),
                -1 => detail.push_str(", yesterday"),
                _ => {}
            }
            (city.name.clone(), detail, local_time_text(&time, settings))
        };
        let time_w = text_width(&time, ROW_TIME) + 4.0;
        let text_right = row.right - time_w - space::S2;
        let mid = row.center_y();
        draw_text(
            painter,
            &name,
            CITY,
            Rect::from_ltrb(row.left, mid - 17.0, text_right, mid + 1.0),
            Align::Start,
            colors.text_primary,
        );
        draw_text(
            painter,
            &detail,
            typography::CAPTION,
            Rect::from_ltrb(row.left, mid + 1.0, text_right, mid + 16.0),
            Align::Start,
            colors.text_secondary,
        );
        draw_right(
            painter,
            &time,
            ROW_TIME,
            Rect::from_ltrb(text_right, row.top, row.right, row.bottom),
            colors.text_primary,
        );
    }
}

fn local_time_text(time: &ZoneTime, settings: Settings) -> String {
    let (text, suffix) = format_time(time, settings, false);
    if suffix.is_empty() {
        text
    } else {
        format!("{text} {suffix}")
    }
}

/// `true` when this tab draws its own cards instead of sitting on the shared page card.
pub fn custom_layout() -> bool {
    true
}

/// Hit test in page coordinates. Use `DashboardHit::Tab(id)` with ids private to this tab.
pub fn hit(grid: Rect, point: Point) -> Option<DashboardHit> {
    let layout = layout(grid);
    let id = if let Some(index) = layout
        .hour_segments
        .1
        .iter()
        .position(|rect| rect.contains(point))
    {
        if index == 0 { ID_12H } else { ID_24H }
    } else if let Some(index) = layout
        .date_segments
        .1
        .iter()
        .position(|rect| rect.contains(point))
    {
        ID_DATE_FULL + index as u16
    } else if layout.seconds_row.contains(point) {
        ID_SECONDS
    } else if layout.add_city.contains(point) {
        ID_ADD_CITY
    } else {
        return None;
    };
    Some(DashboardHit::Tab(id))
}

/// Applies a click from [`hit`]. `true` when handled.
pub fn apply(hit: DashboardHit) -> bool {
    let DashboardHit::Tab(id) = hit else {
        return false;
    };
    match id {
        ID_12H => update_settings(|settings| settings.twelve_hour = true),
        ID_24H => update_settings(|settings| settings.twelve_hour = false),
        ID_SECONDS => update_settings(|settings| settings.show_seconds = !settings.show_seconds),
        ID_DATE_FULL => update_settings(|settings| settings.date_style = DateStyle::Full),
        ID_DATE_SHORT => update_settings(|settings| settings.date_style = DateStyle::Short),
        ID_DATE_COMPACT => update_settings(|settings| settings.date_style = DateStyle::Compact),
        // World clocks are chosen in Settings: ask the app to open it.
        ID_ADD_CITY => {
            super::super::HEADER_REQUEST.with(|cell| cell.set(Some(DashboardHit::OpenSettings)))
        }
        _ => return false,
    }
    true
}

pub fn wants_keyboard() -> bool {
    false
}

pub fn handle_key(_key: &Key) -> bool {
    false
}

/// Every frame, visible or not.
pub fn poll(_now: Instant) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_days_match_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    }

    #[test]
    fn differences_read_naturally() {
        assert_eq!(format_difference(-570), "−9h 30m");
        assert_eq!(format_difference(180), "+3h");
        assert_eq!(format_difference(30), "+30m");
        assert_eq!(format_difference(0), "same time");
        assert_eq!(format_utc_offset(330), "UTC+05:30");
    }

    #[test]
    fn twelve_hour_clock_wraps() {
        let settings = Settings {
            twelve_hour: true,
            ..Settings::default()
        };
        let midnight = ZoneTime {
            day: 0,
            hour: 0,
            minute: 5,
            second: 0,
        };
        assert_eq!(
            format_time(&midnight, settings, false),
            ("12:05".to_string(), "AM")
        );
    }
}
