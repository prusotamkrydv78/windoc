//! Screen time tab: today's total, time left under the daily limit and the number of apps
//! on top, the last seven days below, and the apps behind today on the right.
//!
//! Data comes from what the app already publishes to the dashboard (today's apps and the
//! last seven days). The daily limit lives in the config file, which the tab reads again
//! only when the file changes.

use std::cell::Cell;
use std::time::{Duration, Instant, SystemTime};

use winisland_platform::Key;
use winisland_render::text::FontManager;
use winisland_render::tokens::{SemanticColors, TextStyle, radius, typography};
use winisland_render::{Angle, FontStyle, Painter, Point, Radius, Rect, Rgba, StrokeCap};

use super::TabContext;
use crate::ui::components::icon::{Icon, draw_icon};
use crate::ui::components::text::{Align, draw_text};
use crate::ui::dashboard::{
    DashboardHit, Destination, SCREEN_TIME_APPS, SCREEN_TIME_WEEK, hours_minutes,
};

const PAD: f32 = 14.0;
const GAP: f32 = 8.0;
const APP_ROW: f32 = 36.0;
/// How often the config file is checked for a new daily limit while the tab is open.
const LIMIT_REFRESH: Duration = Duration::from_secs(3);

/// Big figures: tabular so a ticking minute does not shift the layout.
const HERO: TextStyle = TextStyle {
    size: 26.0,
    weight: 650,
    tabular: true,
};
const NAME: TextStyle = TextStyle {
    size: 13.0,
    weight: 500,
    tabular: false,
};
const FIGURE: TextStyle = TextStyle {
    size: 12.0,
    weight: 650,
    tabular: true,
};

// Ids for `DashboardHit::Tab`: 1100..1107 week bars, 1110..1149 app rows, 1150 the pager.
const ID_BAR: u16 = 1100;
const ID_ROW: u16 = 1110;
const MAX_ROWS: usize = 40;
const ID_PAGER: u16 = 1150;

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const DAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
/// Muted tile colours for app monograms, picked by a hash of the name.
const TILE_COLORS: [Rgba; 8] = [
    Rgba::from_rgb(10, 132, 255),
    Rgba::from_rgb(48, 209, 88),
    Rgba::from_rgb(255, 159, 10),
    Rgba::from_rgb(191, 90, 242),
    Rgba::from_rgb(255, 55, 95),
    Rgba::from_rgb(100, 210, 255),
    Rgba::from_rgb(255, 214, 10),
    Rgba::from_rgb(172, 142, 104),
];

thread_local! {
    /// First app shown in the "By app" card when the list has more rows than fit.
    static PAGE_START: Cell<usize> = const { Cell::new(0) };
    /// Daily limit in minutes from the config file; `None` until read.
    static LIMIT: Cell<Option<f32>> = const { Cell::new(None) };
    static LIMIT_MODIFIED: Cell<Option<SystemTime>> = const { Cell::new(None) };
    static LIMIT_CHECKED: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// The three cards: today (top left), the week (bottom left) and apps (right).
struct Cards {
    today: Rect,
    week: Rect,
    apps: Rect,
}

fn cards(grid: Rect) -> Cards {
    let right_width = (grid.width() * 0.34).clamp(180.0, 300.0);
    let split = grid.right - right_width;
    let top_height = (grid.height() * 0.41).clamp(84.0, 104.0);
    let today = Rect::from_ltrb(grid.left, grid.top, split - GAP, grid.top + top_height);
    Cards {
        today,
        week: Rect::from_ltrb(grid.left, today.bottom + GAP, split - GAP, grid.bottom),
        apps: Rect::from_ltrb(split, grid.top, grid.right, grid.bottom),
    }
}

/// Today's total in minutes and the apps behind it, or `None` while tracking is off.
fn today_apps() -> Option<(u64, Vec<(String, u64)>)> {
    SCREEN_TIME_APPS.with(|cell| cell.borrow().clone())
}

/// The last seven days in minutes, oldest first, with today's figure taken from `total`
/// so both cards agree.
fn week_minutes(total: u64) -> Vec<f32> {
    let mut days: Vec<f32> = SCREEN_TIME_WEEK.with(|cell| {
        cell.borrow()
            .iter()
            .map(|(_, minutes)| minutes.max(0.0))
            .collect()
    });
    if days.is_empty() {
        days = vec![0.0; 7];
    }
    if days.len() > 7 {
        days.drain(..days.len() - 7);
    }
    if let Some(today) = days.last_mut() {
        *today = total as f32;
    }
    days
}

/// Weekday index (0 = Sunday) of each of `count` days ending today.
fn weekdays(count: usize) -> Vec<usize> {
    let today = i64::from(crate::platform::shell().local_datetime().day_of_week);
    (0..count)
        .map(|index| (today - (count - 1 - index) as i64).rem_euclid(7) as usize)
        .collect()
}

fn daily_limit() -> Option<u64> {
    LIMIT
        .with(Cell::get)
        .filter(|minutes| *minutes >= 1.0)
        .map(|minutes| minutes.round() as u64)
}

/// The app rows that fit in the "By app" card.
fn rows_top(card: Rect) -> f32 {
    card.top + 34.0
}

fn fitting_rows(card: Rect) -> usize {
    (((card.bottom - 6.0 - rows_top(card)) / APP_ROW)
        .floor()
        .max(0.0) as usize)
        .min(MAX_ROWS)
}

fn app_row(card: Rect, index: usize) -> Rect {
    Rect::from_xywh(
        card.left + 6.0,
        rows_top(card) + index as f32 * APP_ROW,
        card.width() - 12.0,
        APP_ROW - 2.0,
    )
}

/// The pager label and its pill when the apps do not all fit.
fn pager(card: Rect, count: usize) -> Option<(String, Rect)> {
    let fits = fitting_rows(card);
    if fits == 0 || count <= fits {
        return None;
    }
    let start = page_start(count);
    let end = (start + fits).min(count);
    let label = format!("{}–{end} of {count}  ›", start + 1);
    let width = measure(&label, typography::CAPTION) + 16.0;
    Some((
        label,
        Rect::from_xywh(card.right - PAD - width + 6.0, card.top + 8.0, width, 20.0),
    ))
}

fn page_start(count: usize) -> usize {
    let start = PAGE_START.with(Cell::get);
    if start >= count { 0 } else { start }
}

/// Bar columns in the week card, used for drawing and hit testing.
fn week_columns(card: Rect, count: usize) -> Vec<Rect> {
    let inner = Rect::from_ltrb(
        card.left + PAD,
        card.top + 28.0,
        card.right - PAD,
        card.bottom - 4.0,
    );
    let slot = inner.width() / count.max(1) as f32;
    (0..count)
        .map(|index| {
            Rect::from_xywh(
                inner.left + slot * index as f32,
                inner.top,
                slot,
                inner.height(),
            )
        })
        .collect()
}

/// The "Open Settings" button of the off state.
fn settings_button(grid: Rect) -> Rect {
    Rect::from_xywh(grid.center_x() - 66.0, grid.center_y() + 26.0, 132.0, 30.0)
}

/// Draws the tab. Return `false` to fall back to the legacy page.
pub fn draw(ctx: &TabContext<'_>) -> bool {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let Some((total, apps)) = today_apps() else {
        draw_off_state(painter, ctx.grid, ctx.hover, colors);
        return true;
    };
    let cards = cards(ctx.grid);
    for card in [cards.today, cards.week, cards.apps] {
        fill_card(painter, card, colors);
    }
    let week = week_minutes(total);
    draw_today(painter, cards.today, total, apps.len(), &week, colors);
    draw_week(painter, cards.week, &week, ctx.hover, colors);
    draw_apps(painter, cards.apps, total, &apps, ctx.hover, colors);
    true
}

/// `true` when this tab draws its own cards instead of sitting on the shared page card.
pub fn custom_layout() -> bool {
    true
}

/// Hit test in page coordinates. Use `DashboardHit::Tab(id)` with ids private to this tab.
pub fn hit(grid: Rect, point: Point) -> Option<DashboardHit> {
    if !grid.contains(point) {
        return None;
    }
    let Some((_, apps)) = today_apps() else {
        return settings_button(grid)
            .contains(point)
            .then_some(DashboardHit::OpenSettings);
    };
    let cards = cards(grid);
    if cards.apps.contains(point) {
        if let Some((_, pill)) = pager(cards.apps, apps.len())
            && pill.inset(-4.0).contains(point)
        {
            return Some(DashboardHit::Tab(ID_PAGER));
        }
        let start = page_start(apps.len());
        let shown = apps
            .len()
            .saturating_sub(start)
            .min(fitting_rows(cards.apps));
        return (0..shown)
            .find(|index| app_row(cards.apps, *index).contains(point))
            .map(|index| DashboardHit::Tab(ID_ROW + index as u16));
    }
    if cards.week.contains(point) {
        let count = week_minutes(0).len();
        return week_columns(cards.week, count)
            .iter()
            .position(|column| column.contains(point))
            .map(|index| DashboardHit::Tab(ID_BAR + index as u16));
    }
    None
}

/// Applies a click from [`hit`]. `true` when handled.
pub fn apply(hit: DashboardHit) -> bool {
    match hit {
        DashboardHit::Tab(ID_PAGER) => {
            let count = today_apps().map_or(0, |(_, apps)| apps.len());
            let fits = PAGE_FITS.with(Cell::get).max(1);
            let next = page_start(count) + fits;
            PAGE_START.with(|cell| cell.set(if next >= count { 0 } else { next }));
            true
        }
        DashboardHit::Tab(id) => (ID_BAR..ID_PAGER).contains(&id),
        _ => false,
    }
}

thread_local! {
    /// Rows that fit at the last draw, so a pager click advances by one page.
    static PAGE_FITS: Cell<usize> = const { Cell::new(1) };
}

pub fn wants_keyboard() -> bool {
    false
}

pub fn handle_key(_key: &Key) -> bool {
    false
}

/// Every frame, visible or not. Re-reads the daily limit when the config file changes.
pub fn poll(now: Instant) {
    if !crate::ui::dashboard::showing(Destination::ScreenTime) {
        return;
    }
    if LIMIT_CHECKED
        .with(Cell::get)
        .is_some_and(|at| now.saturating_duration_since(at) < LIMIT_REFRESH)
    {
        return;
    }
    LIMIT_CHECKED.with(|cell| cell.set(Some(now)));
    let modified = std::fs::metadata(crate::core::persistence::get_config_path())
        .and_then(|metadata| metadata.modified())
        .ok();
    if LIMIT.with(Cell::get).is_some() && LIMIT_MODIFIED.with(Cell::get) == modified {
        return;
    }
    LIMIT_MODIFIED.with(|cell| cell.set(modified));
    let minutes = crate::core::persistence::load_config().screen_time_limit_minutes;
    LIMIT.with(|cell| cell.set(Some(minutes.max(0.0))));
}

// ---------------------------------------------------------------------------------------
// Cards

fn draw_today(
    painter: Painter<'_>,
    card: Rect,
    total: u64,
    app_count: usize,
    week: &[f32],
    colors: &SemanticColors,
) {
    card_header(painter, card, Icon::Chart, "Today", colors);
    let inner_width = card.width() - PAD * 2.0;
    let column = inner_width / 3.0;
    // Value and caption, centred in the space under the header.
    let block = 46.0;
    let top = card.top + 28.0 + ((card.height() - 36.0 - block) / 2.0).max(0.0);
    let value_baseline = top + 24.0;
    let caption_baseline = top + 42.0;
    for index in 1..3 {
        painter.fill_rect(
            Rect::from_xywh(
                card.left + PAD + column * index as f32,
                top + 2.0,
                1.0,
                block - 4.0,
            ),
            colors.border_subtle,
        );
    }
    let center = |index: usize| card.left + PAD + column * (index as f32 + 0.5);

    // Screen time.
    text_center(
        painter,
        &hours_minutes(total),
        HERO,
        center(0),
        value_baseline,
        column - 12.0,
        colors.text_primary,
    );
    text_center(
        painter,
        "Screen time",
        typography::LABEL,
        center(0),
        caption_baseline,
        column - 12.0,
        colors.text_secondary,
    );

    // Time left under the limit, or today against the daily average.
    let (value, caption, fraction) = match daily_limit() {
        Some(limit) if total <= limit => (
            hours_minutes(limit - total),
            format!("left of {} limit", short_duration(limit)),
            Some(total as f32 / limit as f32),
        ),
        Some(limit) => (
            hours_minutes(total - limit),
            format!("over {} limit", short_duration(limit)),
            Some(1.0),
        ),
        None => match average(&week[..week.len().saturating_sub(1)]) {
            Some(average) => {
                let delta = total as f32 - average;
                let sign = if delta >= 0.0 { "+" } else { "−" };
                (
                    format!("{sign}{}", hours_minutes(delta.abs().round() as u64)),
                    "vs daily average".to_string(),
                    None,
                )
            }
            None => ("—".to_string(), "No daily limit".to_string(), None),
        },
    };
    match fraction {
        Some(fraction) => {
            const RING: f32 = 34.0;
            let text_width = measure(&value, HERO).max(measure(&caption, typography::LABEL));
            let group = (RING + 10.0 + text_width).min(column - 8.0);
            let left = center(1) - group / 2.0;
            let ring_rect = Rect::from_xywh(left, top + (block - RING) / 2.0, RING, RING);
            let color = if fraction >= 1.0 {
                colors.state_error
            } else if fraction >= 0.85 {
                colors.state_warning
            } else {
                colors.text_primary
            };
            draw_ring(painter, ring_rect, fraction, color, colors);
            let text_left = left + RING + 10.0;
            let width = (left + group - text_left).max(0.0);
            text_at(
                painter,
                &value,
                HERO,
                text_left,
                value_baseline,
                width,
                colors.text_primary,
            );
            text_at(
                painter,
                &caption,
                typography::LABEL,
                text_left,
                caption_baseline,
                width,
                colors.text_secondary,
            );
        }
        None => {
            text_center(
                painter,
                &value,
                HERO,
                center(1),
                value_baseline,
                column - 12.0,
                colors.text_primary,
            );
            text_center(
                painter,
                &caption,
                typography::LABEL,
                center(1),
                caption_baseline,
                column - 12.0,
                colors.text_secondary,
            );
        }
    }

    // Apps used.
    text_center(
        painter,
        &app_count.to_string(),
        HERO,
        center(2),
        value_baseline,
        column - 12.0,
        colors.text_primary,
    );
    text_center(
        painter,
        if app_count == 1 {
            "App used"
        } else {
            "Apps used"
        },
        typography::LABEL,
        center(2),
        caption_baseline,
        column - 12.0,
        colors.text_secondary,
    );
}

fn draw_week(
    painter: Painter<'_>,
    card: Rect,
    week: &[f32],
    hover: Option<DashboardHit>,
    colors: &SemanticColors,
) {
    card_header(painter, card, Icon::Calendar, "Last 7 days", colors);
    let names = weekdays(week.len());
    let hovered = match hover {
        Some(DashboardHit::Tab(id)) if (ID_BAR..ID_BAR + 7).contains(&id) => {
            Some(usize::from(id - ID_BAR)).filter(|index| *index < week.len())
        }
        _ => None,
    };
    let note = match hovered {
        Some(index) => format!(
            "{} · {}",
            DAY_NAMES[names[index]],
            hours_minutes(week[index].round() as u64)
        ),
        None => average(week).map_or_else(
            || "No history yet".to_string(),
            |average| format!("avg {}", hours_minutes(average.round() as u64)),
        ),
    };
    text_right(
        painter,
        &note,
        typography::LABEL,
        card.right - PAD,
        card.top + 22.0,
        if hovered.is_some() {
            colors.text_primary
        } else {
            colors.text_secondary
        },
    );

    let label_baseline = card.top + 44.0;
    let day_baseline = card.bottom - 10.0;
    let plot_top = card.top + 52.0;
    let plot_bottom = card.bottom - 24.0;
    let plot_height = (plot_bottom - plot_top).max(4.0);
    let max = week.iter().copied().fold(1.0f32, f32::max);
    let last = week.len().saturating_sub(1);
    for (index, column) in week_columns(card, week.len()).into_iter().enumerate() {
        let minutes = week[index];
        let today = index == last;
        let is_hovered = hovered == Some(index);
        if is_hovered {
            painter.fill_round_rect(
                column.inset(2.0),
                Radius::uniform(10.0),
                colors.text_primary.with_alpha(12),
            );
        }
        let label = if minutes >= 1.0 {
            hours_minutes(minutes.round() as u64)
        } else {
            "—".to_string()
        };
        text_center(
            painter,
            &label,
            if today { FIGURE } else { typography::CAPTION },
            column.center_x(),
            label_baseline,
            column.width(),
            if today || is_hovered {
                colors.text_primary
            } else if minutes >= 1.0 {
                colors.text_secondary
            } else {
                colors.text_disabled
            },
        );
        let bar_width = (column.width() * 0.44).clamp(6.0, 30.0);
        let height = if minutes >= 1.0 {
            (minutes / max * plot_height).max(8.0)
        } else {
            4.0
        };
        let bar = Rect::from_xywh(
            column.center_x() - bar_width / 2.0,
            plot_bottom - height,
            bar_width,
            height,
        );
        let fill = if today {
            colors.text_primary
        } else if is_hovered {
            colors.text_primary.with_alpha(70)
        } else if minutes >= 1.0 {
            colors.text_primary.with_alpha(34)
        } else {
            colors.text_primary.with_alpha(16)
        };
        painter.fill_round_rect(
            bar,
            Radius::uniform((bar_width / 2.0).min(8.0).min(height / 2.0)),
            fill,
        );
        text_center(
            painter,
            DAYS[names[index]],
            if today { FIGURE } else { typography::CAPTION },
            column.center_x(),
            day_baseline,
            column.width(),
            if today || is_hovered {
                colors.text_primary
            } else {
                colors.text_secondary
            },
        );
    }
}

fn draw_apps(
    painter: Painter<'_>,
    card: Rect,
    total: u64,
    apps: &[(String, u64)],
    hover: Option<DashboardHit>,
    colors: &SemanticColors,
) {
    card_header(painter, card, Icon::Chart, "By app", colors);
    let fits = fitting_rows(card);
    PAGE_FITS.with(|cell| cell.set(fits.max(1)));
    if let Some((label, pill)) = pager(card, apps.len()) {
        let hovered = hover == Some(DashboardHit::Tab(ID_PAGER));
        painter.fill_round_rect(
            pill,
            Radius::uniform(pill.height() / 2.0),
            colors
                .text_primary
                .with_alpha(if hovered { 36 } else { 16 }),
        );
        draw_text(
            painter,
            &label,
            typography::CAPTION,
            pill,
            Align::Center,
            if hovered {
                colors.text_primary
            } else {
                colors.text_secondary
            },
        );
    }
    if apps.is_empty() {
        draw_text(
            painter,
            "No app time yet today",
            typography::LABEL,
            Rect::from_ltrb(card.left, card.top + 30.0, card.right, card.bottom - 6.0),
            Align::Center,
            colors.text_secondary,
        );
        return;
    }
    let start = page_start(apps.len());
    let column_width = 58.0;
    for (index, (name, minutes)) in apps.iter().skip(start).take(fits).enumerate() {
        let row = app_row(card, index);
        if hover == Some(DashboardHit::Tab(ID_ROW + index as u16)) {
            painter.fill_round_rect(
                row,
                Radius::uniform(10.0),
                colors.text_primary.with_alpha(14),
            );
        }
        let tile = Rect::from_xywh(row.left + 8.0, row.center_y() - 13.0, 26.0, 26.0);
        draw_monogram(painter, tile, name);
        let text_left = tile.right + 10.0;
        let right = row.right - 8.0;
        let column_left = right - column_width;
        let time = hours_minutes(*minutes);
        text_right(
            painter,
            &time,
            FIGURE,
            right,
            row.top + 15.0,
            colors.text_primary,
        );
        let share = if total == 0 {
            0.0
        } else {
            (*minutes as f32 / total as f32).clamp(0.0, 1.0)
        };
        text_right(
            painter,
            &format!("{:.0}%", share * 100.0),
            typography::CAPTION,
            right,
            row.top + 30.0,
            colors.text_secondary,
        );
        let time_width = measure(&time, FIGURE);
        text_at(
            painter,
            name,
            NAME,
            text_left,
            row.top + 15.0,
            (right - time_width - 8.0 - text_left).max(0.0),
            colors.text_primary,
        );
        let bar = Rect::from_xywh(
            text_left,
            row.top + 23.0,
            (column_left - 6.0 - text_left).max(0.0),
            3.0,
        );
        painter.fill_round_rect(
            bar,
            Radius::uniform(1.5),
            colors.text_primary.with_alpha(22),
        );
        if share > 0.0 {
            painter.fill_round_rect(
                Rect::from_xywh(
                    bar.left,
                    bar.top,
                    (bar.width() * share).max(bar.height()),
                    bar.height(),
                ),
                Radius::uniform(1.5),
                colors.text_primary,
            );
        }
    }
}

/// Tracking is off: one card explaining it, with a button to Settings.
fn draw_off_state(
    painter: Painter<'_>,
    grid: Rect,
    hover: Option<DashboardHit>,
    colors: &SemanticColors,
) {
    fill_card(painter, grid, colors);
    let cx = grid.center_x();
    let cy = grid.center_y();
    draw_icon(
        painter,
        Icon::Chart,
        Rect::from_xywh(cx - 12.0, cy - 62.0, 24.0, 24.0),
        colors.text_secondary,
    );
    text_center(
        painter,
        "Screen time is off",
        typography::SECTION_TITLE,
        cx,
        cy - 18.0,
        grid.width() - PAD * 2.0,
        colors.text_primary,
    );
    text_center(
        painter,
        "Turn it on in Settings to see where your day goes. It stays on this PC.",
        typography::LABEL,
        cx,
        cy + 4.0,
        grid.width() - PAD * 2.0,
        colors.text_secondary,
    );
    let button = settings_button(grid);
    let hovered = hover == Some(DashboardHit::OpenSettings);
    painter.fill_round_rect(
        button,
        Radius::uniform(button.height() / 2.0),
        colors
            .text_primary
            .with_alpha(if hovered { 44 } else { 24 }),
    );
    draw_text(
        painter,
        "Open Settings",
        typography::LABEL,
        button,
        Align::Center,
        colors.text_primary,
    );
}

// ---------------------------------------------------------------------------------------
// Drawing helpers

fn fill_card(painter: Painter<'_>, card: Rect, colors: &SemanticColors) {
    painter.fill_round_rect(
        card,
        Radius::uniform(radius::CARD + 6.0),
        colors.surface_raised,
    );
}

/// Icon and title at the card's top left.
fn card_header(painter: Painter<'_>, card: Rect, icon: Icon, title: &str, colors: &SemanticColors) {
    draw_icon(
        painter,
        icon,
        Rect::from_xywh(card.left + PAD, card.top + 11.0, 13.0, 13.0),
        colors.text_secondary,
    );
    text_at(
        painter,
        title,
        typography::LABEL,
        card.left + PAD + 19.0,
        card.top + 22.0,
        card.width() / 2.0,
        colors.text_secondary,
    );
}

/// A track ring with `fraction` of it drawn clockwise from the top.
fn draw_ring(
    painter: Painter<'_>,
    rect: Rect,
    fraction: f32,
    color: Rgba,
    colors: &SemanticColors,
) {
    let stroke = 3.5;
    let ring = rect.inset(stroke / 2.0);
    painter.stroke_arc(
        ring,
        Angle::from_degrees(0.0),
        Angle::from_degrees(360.0),
        stroke,
        colors.text_primary.with_alpha(30),
        StrokeCap::Round,
    );
    let sweep = 360.0 * fraction.clamp(0.0, 1.0);
    if sweep > 0.5 {
        painter.stroke_arc(
            ring,
            Angle::from_degrees(0.0),
            Angle::from_degrees(sweep),
            stroke,
            color,
            StrokeCap::Round,
        );
    }
}

/// A rounded tile with the app's first letter, tinted by its name.
fn draw_monogram(painter: Painter<'_>, tile: Rect, name: &str) {
    let hash = name.bytes().fold(5381u32, |hash, byte| {
        hash.wrapping_mul(33) ^ u32::from(byte)
    });
    let tint = TILE_COLORS[hash as usize % TILE_COLORS.len()];
    painter.fill_round_rect(tile, Radius::uniform(7.0), tint.with_alpha(52));
    let letter: String = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".to_string());
    draw_text(
        painter,
        &letter,
        TextStyle {
            size: 13.0,
            weight: 650,
            tabular: false,
        },
        tile,
        Align::Center,
        tint,
    );
}

fn measure(text: &str, style: TextStyle) -> f32 {
    let font = if style.weight >= 600 {
        FontStyle::bold()
    } else {
        FontStyle::normal()
    };
    FontManager::global().measure_text_cached(text, style.size, font)
}

/// One line of text whose baseline sits at `baseline`, starting at `x`.
fn text_at(
    painter: Painter<'_>,
    text: &str,
    style: TextStyle,
    x: f32,
    baseline: f32,
    width: f32,
    color: Rgba,
) {
    let center = baseline - style.size * 0.35;
    draw_text(
        painter,
        text,
        style,
        Rect::from_xywh(x, center - style.size * 0.75, width, style.size * 1.5),
        Align::Start,
        color,
    );
}

/// One line of text ending at `right`.
fn text_right(
    painter: Painter<'_>,
    text: &str,
    style: TextStyle,
    right: f32,
    baseline: f32,
    color: Rgba,
) {
    let width = measure(text, style);
    text_at(
        painter,
        text,
        style,
        right - width - 1.0,
        baseline,
        width + 2.0,
        color,
    );
}

/// One line of text centred on `center_x`, at most `width` wide.
fn text_center(
    painter: Painter<'_>,
    text: &str,
    style: TextStyle,
    center_x: f32,
    baseline: f32,
    width: f32,
    color: Rgba,
) {
    let center = baseline - style.size * 0.35;
    draw_text(
        painter,
        text,
        style,
        Rect::from_xywh(
            center_x - width / 2.0,
            center - style.size * 0.75,
            width,
            style.size * 1.5,
        ),
        Align::Center,
        color,
    );
}

// ---------------------------------------------------------------------------------------
// Figures

/// Average of the days that have any time, in minutes.
fn average(days: &[f32]) -> Option<f32> {
    let active: Vec<f32> = days.iter().copied().filter(|m| *m >= 1.0).collect();
    (!active.is_empty()).then(|| active.iter().sum::<f32>() / active.len() as f32)
}

/// `6h`, `6h 30m` or `45m`, for the limit.
fn short_duration(minutes: u64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h {minutes}m"),
    }
}
