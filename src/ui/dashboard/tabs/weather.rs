//! Weather tab: three cards. "Now" (big glyph, temperature, condition, high and low, feels
//! like, place and update time), "Hourly" (the next six hours with glyphs, a smooth line
//! and area chart, and a humidity, wind and UV footer) and "5-Day" (low, range bar, high).
//!
//! The app publishes only a short summary to the dashboard, so the full forecast is read
//! from the weather service's cache file (`weather.json`) while the tab is visible. The
//! summary still decides whether weather is on: no summary means off (or expired).

mod icons;

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant, SystemTime};

use winisland_core::i18n::tr;
use winisland_core::weather::{Condition, Forecast, HourlyPoint};
use winisland_platform::Key;
use winisland_render::text::FontManager;
use winisland_render::tokens::{SemanticColors, TextStyle, radius, space, typography};
use winisland_render::{
    FontStyle, GradientStop, Painter, PathBuilder, Point, Radius, Rect, Rgba, StrokeCap,
    StrokeJoin, TileMode,
};

use super::TabContext;
use crate::ui::components::text::{Align, draw_text};
use crate::ui::dashboard::{DashboardHit, Destination};
use icons::Glyph;

const PAD: f32 = 16.0;
const HERO: TextStyle = TextStyle {
    size: 40.0,
    weight: 650,
    tabular: true,
};
const TEMP: TextStyle = TextStyle {
    size: 13.0,
    weight: 650,
    tabular: true,
};
const VALUE: TextStyle = TextStyle {
    size: 13.0,
    weight: 650,
    tabular: false,
};
const TABULAR_CAPTION: TextStyle = TextStyle {
    size: 11.0,
    weight: 400,
    tabular: true,
};
const COLD: Rgba = Rgba::from_rgb(90, 170, 255);
const WARM: Rgba = Rgba::from_rgb(255, 170, 60);

/// Hit ids (300-399 belong to this tab).
const ID_SETTINGS: u16 = 300;
const ID_HOUR: u16 = 310;
const HOURS: usize = 6;
const DAYS: usize = 5;
const RELOAD_EVERY: Duration = Duration::from_secs(10);
const MAX_FILE_BYTES: u64 = 256 * 1024;

thread_local! {
    static FORECAST: RefCell<Option<Forecast>> = const { RefCell::new(None) };
    static LOADED_AT: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Re-reads the cached forecast while the tab is visible.
pub fn poll(now: Instant) {
    if !crate::ui::dashboard::showing(Destination::Weather) {
        return;
    }
    let due = LOADED_AT
        .with(Cell::get)
        .is_none_or(|at| now.saturating_duration_since(at) >= RELOAD_EVERY);
    if !due {
        return;
    }
    LOADED_AT.with(|cell| cell.set(Some(now)));
    let forecast = match crate::core::json_store::read::<Forecast>(
        crate::core::weather_service::FILE_NAME,
        MAX_FILE_BYTES,
    ) {
        Ok(forecast) => forecast,
        Err(error) => {
            log::debug!("Weather tab cannot read the forecast: {error}");
            None
        }
    };
    FORECAST.with(|cell| *cell.borrow_mut() = forecast);
}

pub fn custom_layout() -> bool {
    true
}

pub fn wants_keyboard() -> bool {
    false
}

pub fn handle_key(_key: &Key) -> bool {
    false
}

/// What the tab can show right now.
enum State {
    Off,
    Loading,
    Ready(Box<Forecast>),
}

fn state() -> State {
    let summary = crate::ui::dashboard::WEATHER.with(|cell| cell.borrow().clone());
    let Some(summary) = summary else {
        return State::Off;
    };
    match FORECAST.with(|cell| cell.borrow().clone()) {
        Some(forecast) if forecast.place.name == summary.place => State::Ready(Box::new(forecast)),
        _ => State::Loading,
    }
}

struct Cards {
    now: Rect,
    hourly: Rect,
    days: Rect,
}

fn cards(grid: Rect) -> Cards {
    let gap = space::S3;
    let usable = grid.width() - gap * 2.0;
    let side = (usable * 0.3).floor();
    let middle = usable - side * 2.0;
    let now = Rect::from_xywh(grid.left, grid.top, side, grid.height());
    let hourly = Rect::from_xywh(now.right + gap, grid.top, middle, grid.height());
    let days = Rect::from_xywh(hourly.right + gap, grid.top, side, grid.height());
    Cards { now, hourly, days }
}

/// The "Open Settings" pill of the empty state.
fn settings_button(grid: Rect) -> Rect {
    Rect::from_xywh(grid.center_x() - 64.0, grid.center_y() + 30.0, 128.0, 30.0)
}

/// Geometry of the hourly card.
struct HourlyLayout {
    columns: Vec<Rect>,
    chart: Rect,
    footer: Rect,
}

fn hourly_layout(card: Rect, count: usize) -> HourlyLayout {
    let inner = Rect::from_ltrb(
        card.left + PAD,
        card.top + PAD,
        card.right - PAD,
        card.bottom - PAD,
    );
    let count = count.max(1);
    let width = inner.width() / count as f32;
    let columns = (0..count)
        .map(|index| {
            Rect::from_xywh(
                inner.left + width * index as f32,
                inner.top + 20.0,
                width,
                54.0,
            )
        })
        .collect();
    let footer = Rect::from_ltrb(inner.left, inner.bottom - 32.0, inner.right, inner.bottom);
    let chart = Rect::from_ltrb(inner.left, inner.top + 80.0, inner.right, footer.top - 10.0);
    HourlyLayout {
        columns,
        chart,
        footer,
    }
}

pub fn hit(grid: Rect, point: Point) -> Option<DashboardHit> {
    match state() {
        State::Off => settings_button(grid)
            .contains(point)
            .then_some(DashboardHit::Tab(ID_SETTINGS)),
        State::Loading => None,
        State::Ready(forecast) => {
            let card = cards(grid).hourly;
            let count = upcoming_hours(&forecast).len();
            if count < 2 || !card.contains(point) {
                return None;
            }
            let layout = hourly_layout(card, count);
            layout
                .columns
                .iter()
                .position(|column| {
                    point.x >= column.left
                        && point.x < column.right
                        && point.y >= column.top
                        && point.y < layout.chart.bottom
                })
                .map(|index| DashboardHit::Tab(ID_HOUR + index as u16))
        }
    }
}

pub fn apply(hit: DashboardHit) -> bool {
    match hit {
        DashboardHit::Tab(ID_SETTINGS) => {
            crate::ui::dashboard::HEADER_REQUEST
                .with(|cell| cell.set(Some(DashboardHit::OpenSettings)));
            true
        }
        // Hovering a column shows its details; a click has nothing more to do.
        DashboardHit::Tab(id) if (ID_HOUR..ID_HOUR + HOURS as u16).contains(&id) => true,
        _ => false,
    }
}

pub fn draw(ctx: &TabContext<'_>) -> bool {
    let painter = ctx.painter;
    let colors = ctx.colors;
    match state() {
        State::Off => draw_off(painter, ctx.grid, ctx.hover, colors),
        State::Loading => {
            let cards = cards(ctx.grid);
            for card in [cards.now, cards.hourly, cards.days] {
                fill_card(painter, card, colors);
            }
            draw_text(
                painter,
                "Loading the forecast…",
                typography::BODY,
                cards.hourly,
                Align::Center,
                colors.text_secondary,
            );
        }
        State::Ready(forecast) => {
            let cards = cards(ctx.grid);
            for card in [cards.now, cards.hourly, cards.days] {
                fill_card(painter, card, colors);
            }
            let hovered = match ctx.hover {
                Some(DashboardHit::Tab(id)) if (ID_HOUR..ID_HOUR + HOURS as u16).contains(&id) => {
                    Some(usize::from(id - ID_HOUR))
                }
                _ => None,
            };
            draw_now(painter, cards.now, &forecast, colors);
            draw_hourly(painter, cards.hourly, &forecast, hovered, colors);
            draw_days(painter, cards.days, &forecast, colors);
        }
    }
    true
}

fn fill_card(painter: Painter<'_>, card: Rect, colors: &SemanticColors) {
    painter.fill_round_rect(
        card,
        Radius::uniform(radius::CARD + 6.0),
        colors.surface_raised,
    );
}

fn draw_off(
    painter: Painter<'_>,
    grid: Rect,
    hover: Option<DashboardHit>,
    colors: &SemanticColors,
) {
    fill_card(painter, grid, colors);
    icons::draw(
        painter,
        Glyph::SunCloud,
        Point::new(grid.center_x(), grid.center_y() - 56.0),
        36.0,
        colors,
        colors.surface_raised,
    );
    draw_text(
        painter,
        "Weather is off",
        typography::SECTION_TITLE,
        Rect::from_xywh(grid.left, grid.center_y() - 26.0, grid.width(), 22.0),
        Align::Center,
        colors.text_primary,
    );
    draw_text(
        painter,
        "Turn on Weather and choose a city in Settings",
        typography::LABEL,
        Rect::from_xywh(grid.left, grid.center_y() + 2.0, grid.width(), 18.0),
        Align::Center,
        colors.text_secondary,
    );
    let button = settings_button(grid);
    let hovered = hover == Some(DashboardHit::Tab(ID_SETTINGS));
    painter.fill_round_rect(
        button,
        Radius::uniform(button.height() / 2.0),
        if hovered {
            colors.text_primary.with_alpha(220)
        } else {
            colors.text_primary
        },
    );
    draw_text(
        painter,
        "Open Settings",
        typography::LABEL,
        button,
        Align::Center,
        colors.surface_base,
    );
}

fn degrees(value: f64) -> String {
    format!("{:.0}°", value.round())
}

fn glyph_for(condition: Condition, night: bool) -> Glyph {
    match condition {
        Condition::Clear if night => Glyph::Moon,
        Condition::Clear => Glyph::Sun,
        Condition::PartlyCloudy if night => Glyph::MoonCloud,
        Condition::PartlyCloudy => Glyph::SunCloud,
        Condition::Cloudy | Condition::Unknown => Glyph::Cloud,
        Condition::Fog => Glyph::Fog,
        Condition::Drizzle => Glyph::Drizzle,
        Condition::Rain | Condition::Showers => Glyph::Rain,
        Condition::Snow => Glyph::Snow,
        Condition::Thunderstorm => Glyph::Thunder,
    }
}

/// Hour of a provider time such as `2026-10-30T14:00`.
fn hour_of(time: &str) -> Option<u32> {
    time.get(11..13)?.parse().ok()
}

fn is_night(hour: Option<u32>) -> bool {
    hour.is_some_and(|hour| !(6..19).contains(&hour))
}

/// The next hours, skipping those that have passed since the forecast was fetched.
fn upcoming_hours(forecast: &Forecast) -> Vec<HourlyPoint> {
    let skip = (forecast.age(SystemTime::now()).as_secs() / 3600) as usize;
    forecast
        .hourly
        .iter()
        .skip(skip)
        .take(HOURS)
        .cloned()
        .collect()
}

/// The provider gives no per-hour condition, so an hour uses its day's condition, nudged by
/// the chance of rain.
fn hour_glyph(forecast: &Forecast, index: usize, hour: &HourlyPoint) -> Glyph {
    let base = if index == 0 {
        forecast.condition
    } else {
        forecast
            .daily
            .iter()
            .find(|day| hour.time.starts_with(&day.date))
            .map_or(forecast.condition, |day| day.condition)
    };
    let wet = matches!(
        base,
        Condition::Drizzle
            | Condition::Rain
            | Condition::Showers
            | Condition::Snow
            | Condition::Thunderstorm
    );
    let condition = match hour.precipitation_probability {
        Some(chance) if chance >= 60 && !wet => Condition::Rain,
        Some(chance) if chance < 20 && wet && base != Condition::Snow => Condition::PartlyCloudy,
        _ => base,
    };
    glyph_for(condition, is_night(hour_of(&hour.time)))
}

/// Minutes to add to UTC for local time, from the shell's local clock.
fn local_offset_minutes() -> i64 {
    let local = crate::platform::shell().local_datetime();
    let days = crate::ui::dashboard::days_from_civil(
        i64::from(local.year),
        u32::from(local.month),
        u32::from(local.day),
    );
    let local_minutes = days * 1440 + i64::from(local.hour) * 60 + i64::from(local.minute);
    let utc_minutes = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| (elapsed.as_secs() / 60) as i64)
        .unwrap_or_default();
    ((local_minutes - utc_minutes) as f64 / 15.0).round() as i64 * 15
}

fn updated_text(forecast: &Forecast) -> String {
    let age = forecast.age(SystemTime::now());
    let fetched = forecast
        .fetched_at
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| (elapsed.as_secs() / 60) as i64)
        .unwrap_or_default()
        + local_offset_minutes();
    let clock = format!(
        "{:02}:{:02}",
        fetched.rem_euclid(1440) / 60,
        fetched.rem_euclid(60)
    );
    let minutes = age.as_secs() / 60;
    if minutes < 1 {
        format!("Updated {clock} · just now")
    } else if minutes < 60 {
        format!("Updated {clock} · {minutes} min ago")
    } else {
        format!("Updated {clock} · {} h ago", minutes / 60)
    }
}

fn text_width(text: &str, style: TextStyle) -> f32 {
    let font = if style.weight >= 600 {
        FontStyle::bold()
    } else {
        FontStyle::normal()
    };
    FontManager::global().measure_text_cached(text, style.size, font)
}

/// Text right-aligned against `right`.
fn draw_right(
    painter: Painter<'_>,
    text: &str,
    style: TextStyle,
    right: f32,
    row: Rect,
    color: Rgba,
) {
    let width = text_width(text, style) + 1.0;
    draw_text(
        painter,
        text,
        style,
        Rect::from_ltrb(right - width, row.top, right, row.bottom),
        Align::Start,
        color,
    );
}

fn card_title(painter: Painter<'_>, card: Rect, title: &str, colors: &SemanticColors) {
    draw_text(
        painter,
        title,
        typography::LABEL,
        Rect::from_xywh(
            card.left + PAD,
            card.top + PAD - 2.0,
            card.width() - PAD * 2.0,
            16.0,
        ),
        Align::Start,
        colors.text_secondary,
    );
}

fn draw_now(painter: Painter<'_>, card: Rect, forecast: &Forecast, colors: &SemanticColors) {
    let left = card.left + PAD;
    let width = card.width() - PAD * 2.0;
    let hour = upcoming_hours(forecast)
        .first()
        .and_then(|hour| hour_of(&hour.time));
    icons::draw(
        painter,
        glyph_for(forecast.condition, is_night(hour)),
        Point::new(left + 20.0, card.top + PAD + 18.0),
        40.0,
        colors,
        colors.surface_raised,
    );
    let mut y = card.top + PAD + 42.0;
    draw_text(
        painter,
        &degrees(forecast.temperature),
        HERO,
        Rect::from_xywh(left, y, width, 46.0),
        Align::Start,
        colors.text_primary,
    );
    y += 46.0;
    draw_text(
        painter,
        &tr(forecast.condition.label_key()),
        typography::BODY,
        Rect::from_xywh(left, y, width, 18.0),
        Align::Start,
        colors.text_primary,
    );
    y += 20.0;
    if let Some(today) = forecast.daily.first() {
        draw_text(
            painter,
            &format!("H {}   L {}", degrees(today.max), degrees(today.min)),
            typography::LABEL,
            Rect::from_xywh(left, y, width, 16.0),
            Align::Start,
            colors.text_primary,
        );
        y += 17.0;
    }
    draw_text(
        painter,
        &format!("Feels like {}", degrees(forecast.apparent_temperature)),
        typography::LABEL,
        Rect::from_xywh(left, y, width, 16.0),
        Align::Start,
        colors.text_secondary,
    );
    // Footer: place and freshness.
    let footer = card.bottom - PAD - 30.0;
    if footer > y + 22.0 {
        painter.fill_rect(
            Rect::from_xywh(left, footer - 6.0, width, 1.0),
            colors.border_subtle,
        );
        let place = if forecast.place.detail.is_empty() {
            forecast.place.name.clone()
        } else {
            format!("{} · {}", forecast.place.name, forecast.place.detail)
        };
        draw_text(
            painter,
            &place,
            typography::CAPTION,
            Rect::from_xywh(left, footer, width, 15.0),
            Align::Start,
            colors.text_secondary,
        );
        let stale = forecast.age(SystemTime::now()) > winisland_core::weather::FRESH_FOR;
        draw_text(
            painter,
            &updated_text(forecast),
            typography::CAPTION,
            Rect::from_xywh(left, footer + 15.0, width, 15.0),
            Align::Start,
            if stale {
                colors.state_warning
            } else {
                colors.text_secondary
            },
        );
    }
}

fn draw_hourly(
    painter: Painter<'_>,
    card: Rect,
    forecast: &Forecast,
    hovered: Option<usize>,
    colors: &SemanticColors,
) {
    card_title(painter, card, "Hourly", colors);
    let hours = upcoming_hours(forecast);
    if hours.len() < 2 {
        draw_text(
            painter,
            "No hourly forecast yet",
            typography::LABEL,
            card,
            Align::Center,
            colors.text_secondary,
        );
        return;
    }
    let layout = hourly_layout(card, hours.len());
    for (index, (hour, column)) in hours.iter().zip(&layout.columns).enumerate() {
        let selected = hovered == Some(index);
        let label = if index == 0 {
            "Now".to_string()
        } else {
            hour.time.get(11..16).unwrap_or(&hour.time).to_string()
        };
        draw_text(
            painter,
            &label,
            if index == 0 {
                TextStyle {
                    size: 11.0,
                    weight: 650,
                    tabular: true,
                }
            } else {
                TABULAR_CAPTION
            },
            Rect::from_xywh(column.left, column.top, column.width(), 14.0),
            Align::Center,
            if index == 0 || selected {
                colors.text_primary
            } else {
                colors.text_secondary
            },
        );
        icons::draw(
            painter,
            hour_glyph(forecast, index, hour),
            Point::new(column.center_x(), column.top + 26.0),
            18.0,
            colors,
            colors.surface_raised,
        );
        draw_text(
            painter,
            &degrees(hour.temperature),
            TEMP,
            Rect::from_xywh(column.left, column.top + 38.0, column.width(), 16.0),
            Align::Center,
            colors.text_primary,
        );
    }
    let centers: Vec<f32> = layout.columns.iter().map(Rect::center_x).collect();
    let values: Vec<f32> = hours.iter().map(|hour| hour.temperature as f32).collect();
    draw_chart(painter, layout.chart, &centers, &values, hovered, colors);
    if let Some(index) = hovered
        && let Some(hour) = hours.get(index)
    {
        let text = match hour.precipitation_probability {
            Some(chance) => format!("Rain {chance}%"),
            None => "No rain data".to_string(),
        };
        let width = text_width(&text, typography::CAPTION) + 14.0;
        let center = centers[index];
        let pill = Rect::from_xywh(
            (center - width / 2.0).clamp(
                layout.chart.left,
                (layout.chart.right - width).max(layout.chart.left),
            ),
            layout.chart.bottom - 18.0,
            width,
            18.0,
        );
        painter.fill_round_rect(pill, Radius::uniform(9.0), colors.surface_overlay);
        draw_text(
            painter,
            &text,
            typography::CAPTION,
            pill,
            Align::Center,
            colors.text_primary,
        );
    }
    draw_hourly_footer(painter, layout.footer, forecast, colors);
}

/// A smooth temperature line through the column centres with a soft fill beneath and
/// dotted guides up to the temperature labels.
fn draw_chart(
    painter: Painter<'_>,
    rect: Rect,
    centers: &[f32],
    values: &[f32],
    hovered: Option<usize>,
    colors: &SemanticColors,
) {
    if rect.height() < 12.0 || values.len() < 2 {
        return;
    }
    let (min, max) = values
        .iter()
        .fold((f32::MAX, f32::MIN), |(min, max), value| {
            (min.min(*value), max.max(*value))
        });
    let range = (max - min).max(3.0);
    let middle = (max + min) / 2.0;
    let plot_top = rect.top + 6.0;
    let plot_bottom = rect.bottom - 22.0;
    let plot_height = (plot_bottom - plot_top).max(4.0);
    let points: Vec<Point> = centers
        .iter()
        .zip(values)
        .map(|(x, value)| {
            let fraction = 0.5 + (value - middle) / range;
            Point::new(*x, plot_bottom - fraction * plot_height)
        })
        .collect();
    let line_color = colors.text_primary;
    // Dotted guides.
    for point in &points {
        let mut y = rect.top - 4.0;
        while y < point.y - 4.0 {
            painter.fill_rect(
                Rect::from_xywh(point.x - 0.5, y, 1.0, 2.0),
                colors.border_strong,
            );
            y += 5.0;
        }
    }
    let mut line = PathBuilder::new();
    line.move_to(points[0]);
    for index in 1..points.len() {
        let previous = points[index - 1];
        let current = points[index];
        let before = points[index.saturating_sub(2)];
        let after = points[(index + 1).min(points.len() - 1)];
        line.cubic_to(
            Point::new(
                previous.x + (current.x - before.x) / 6.0,
                previous.y + (current.y - before.y) / 6.0,
            ),
            Point::new(
                current.x - (after.x - previous.x) / 6.0,
                current.y - (after.y - previous.y) / 6.0,
            ),
            current,
        );
    }
    let mut area = PathBuilder::new();
    area.move_to(Point::new(points[0].x, rect.bottom));
    area.line_to(points[0]);
    for index in 1..points.len() {
        let previous = points[index - 1];
        let current = points[index];
        let before = points[index.saturating_sub(2)];
        let after = points[(index + 1).min(points.len() - 1)];
        area.cubic_to(
            Point::new(
                previous.x + (current.x - before.x) / 6.0,
                previous.y + (current.y - before.y) / 6.0,
            ),
            Point::new(
                current.x - (after.x - previous.x) / 6.0,
                current.y - (after.y - previous.y) / 6.0,
            ),
            current,
        );
    }
    area.line_to(Point::new(points[points.len() - 1].x, rect.bottom));
    area.close();
    painter.fill_path_with_gradient(
        &area.detach(),
        Point::new(rect.left, plot_top),
        Point::new(rect.left, rect.bottom),
        &[
            GradientStop {
                offset: 0.0,
                color: line_color.with_alpha(64),
            },
            GradientStop {
                offset: 1.0,
                color: line_color.with_alpha(8),
            },
        ],
        TileMode::Clamp,
    );
    painter.stroke_path(
        &line.detach(),
        2.0,
        line_color,
        StrokeCap::Round,
        StrokeJoin::Round,
    );
    for (index, point) in points.iter().enumerate() {
        let selected = hovered == Some(index);
        let size = if selected { 5.0 } else { 3.0 };
        if selected {
            painter.fill_circle(*point, size + 3.0, line_color.with_alpha(50));
        }
        painter.fill_circle(*point, size, line_color);
    }
}

fn uv_level(uv: f64) -> &'static str {
    match uv {
        value if value < 3.0 => "Low",
        value if value < 6.0 => "Moderate",
        value if value < 8.0 => "High",
        value if value < 11.0 => "Very high",
        _ => "Extreme",
    }
}

fn draw_hourly_footer(
    painter: Painter<'_>,
    footer: Rect,
    forecast: &Forecast,
    colors: &SemanticColors,
) {
    painter.fill_rect(
        Rect::from_xywh(footer.left, footer.top - 6.0, footer.width(), 1.0),
        colors.border_subtle,
    );
    // The forecast request never asks for another wind unit, so speeds are km/h.
    let wind_unit = "km/h";
    let items = [
        (
            "Humidity",
            forecast
                .humidity_percent
                .map(|value| format!("{value:.0}%")),
        ),
        (
            "Wind",
            forecast
                .wind_speed
                .map(|value| format!("{value:.0} {wind_unit}")),
        ),
        (
            "UV index",
            forecast
                .uv_index
                .map(|value| format!("{value:.0} {}", uv_level(value))),
        ),
    ];
    let width = footer.width() / items.len() as f32;
    for (index, (label, value)) in items.iter().enumerate() {
        let cell = Rect::from_xywh(
            footer.left + width * index as f32,
            footer.top,
            width,
            footer.height(),
        );
        let icon = Point::new(cell.left + 7.0, cell.center_y() + 1.0);
        let tint = colors.text_secondary;
        match index {
            0 => icons::drop(painter, icon, 14.0, tint),
            1 => icons::wind(painter, icon, 14.0, tint),
            _ => icons::uv(painter, icon, 14.0, tint),
        }
        let text_left = cell.left + 18.0;
        let text_width = (cell.right - text_left - 4.0).max(0.0);
        draw_text(
            painter,
            label,
            typography::CAPTION,
            Rect::from_xywh(text_left, cell.top, text_width, 14.0),
            Align::Start,
            colors.text_secondary,
        );
        draw_text(
            painter,
            value.as_deref().unwrap_or("—"),
            VALUE,
            Rect::from_xywh(text_left, cell.top + 15.0, text_width, 17.0),
            Align::Start,
            colors.text_primary,
        );
    }
}

fn weekday_short(date: &str) -> Option<&'static str> {
    const NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let year: i64 = date.get(0..4)?.parse().ok()?;
    let month: u32 = date.get(5..7)?.parse().ok()?;
    let day: u32 = date.get(8..10)?.parse().ok()?;
    let days = crate::ui::dashboard::days_from_civil(year, month, day);
    NAMES.get((days + 4).rem_euclid(7) as usize).copied()
}

fn mix(from: Rgba, to: Rgba, amount: f32) -> Rgba {
    let amount = amount.clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * amount) as u8;
    Rgba::from_rgb(
        channel(from.r(), to.r()),
        channel(from.g(), to.g()),
        channel(from.b(), to.b()),
    )
}

fn draw_days(painter: Painter<'_>, card: Rect, forecast: &Forecast, colors: &SemanticColors) {
    card_title(painter, card, "5-Day", colors);
    let days: Vec<_> = forecast.daily.iter().take(DAYS).collect();
    if days.is_empty() {
        draw_text(
            painter,
            "No daily forecast yet",
            typography::LABEL,
            card,
            Align::Center,
            colors.text_secondary,
        );
        return;
    }
    let today = upcoming_hours(forecast)
        .first()
        .and_then(|hour| hour.time.get(0..10).map(str::to_string));
    let (low, high) = days.iter().fold((f64::MAX, f64::MIN), |(low, high), day| {
        (low.min(day.min), high.max(day.max))
    });
    let span = (high - low).max(1.0);
    let top = card.top + PAD + 20.0;
    let bottom = card.bottom - PAD + 4.0;
    let row_height = ((bottom - top) / DAYS as f32).min(40.0);
    let left = card.left + PAD;
    let right = card.right - PAD;
    let number_width = 28.0;
    for (index, day) in days.iter().enumerate() {
        let row = Rect::from_xywh(
            left,
            top + row_height * index as f32,
            right - left,
            row_height,
        );
        if index > 0 {
            painter.fill_rect(
                Rect::from_xywh(row.left, row.top, row.width(), 1.0),
                colors.border_subtle,
            );
        }
        let is_today =
            today.as_deref() == Some(day.date.as_str()) || (today.is_none() && index == 0);
        let name = if is_today {
            "Today"
        } else {
            weekday_short(&day.date).unwrap_or("")
        };
        draw_text(
            painter,
            name,
            typography::LABEL,
            Rect::from_xywh(row.left, row.top, 38.0, row.height()),
            Align::Start,
            colors.text_primary,
        );
        icons::draw(
            painter,
            glyph_for(day.condition, false),
            Point::new(row.left + 46.0, row.center_y()),
            16.0,
            colors,
            colors.surface_raised,
        );
        let low_right = row.left + 54.0 + number_width;
        draw_right(
            painter,
            &degrees(day.min),
            TEMP,
            low_right,
            row,
            colors.text_secondary,
        );
        draw_right(
            painter,
            &degrees(day.max),
            TEMP,
            row.right,
            row,
            colors.text_primary,
        );
        let track = Rect::from_ltrb(
            low_right + 6.0,
            row.center_y() - 2.0,
            row.right - number_width - 6.0,
            row.center_y() + 2.0,
        );
        if track.width() < 12.0 {
            continue;
        }
        painter.fill_round_rect(track, Radius::uniform(2.0), colors.surface_overlay);
        let start = ((day.min - low) / span) as f32;
        let end = ((day.max - low) / span) as f32;
        let fill = Rect::from_ltrb(
            track.left + track.width() * start,
            track.top,
            (track.left + track.width() * end).max(track.left + track.width() * start + 4.0),
            track.bottom,
        );
        painter.fill_round_rect_with_gradient(
            fill,
            Radius::uniform(2.0),
            Point::new(fill.left, fill.top),
            Point::new(fill.right, fill.top),
            &[
                GradientStop {
                    offset: 0.0,
                    color: mix(COLD, WARM, start),
                },
                GradientStop {
                    offset: 1.0,
                    color: mix(COLD, WARM, end),
                },
            ],
            TileMode::Clamp,
        );
        if is_today {
            let now = ((forecast.temperature - low) / span).clamp(0.0, 1.0) as f32;
            let center = Point::new(track.left + track.width() * now, track.center_y());
            painter.fill_circle(center, 4.5, colors.surface_raised);
            painter.fill_circle(center, 3.0, colors.text_primary);
        }
    }
}
