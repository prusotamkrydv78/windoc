//! Pop-up panels under the top bar: Control Centre, Calendar and Notifications, drawn in the
//! bar's own window below the strip.

use std::cell::RefCell;

use winisland_render::text::{DrawTextCachedParams, FontManager};
use winisland_render::{FontStyle, Painter, Point, Radius, Rect, Rgba};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Popover {
    ControlCentre,
    Calendar,
    Notifications,
}

/// Something clickable inside a popover.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PopHit {
    /// A tile that opens a Windows settings page.
    Settings(&'static str),
    /// The volume slider, at a fraction 0..1 of its width.
    Volume(f32),
    PlayPause,
    /// Opens the Windows notification centre.
    WindowsNotifications,
    /// Inside the panel but on nothing actionable.
    Panel,
}

/// Data the app publishes for the panels.
#[derive(Clone, Debug, Default)]
pub struct PopoverData {
    pub volume: Option<f32>,
    pub battery: Option<(u8, bool)>,
    pub track: Option<(String, String, bool)>,
    pub agenda: Vec<(String, String)>,
}

#[derive(Default)]
struct State {
    open: Option<(Popover, f32)>,
    data: PopoverData,
    history: Vec<(String, String)>,
    rect: Option<Rect>,
    targets: Vec<(Rect, PopHit)>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

const WIDTH: f32 = 320.0;
const PAD: f32 = 12.0;
const GLASS: Rgba = Rgba::from_argb(244, 30, 30, 34);
const TILE: Rgba = Rgba::from_argb(255, 44, 44, 46);
const BLUE: Rgba = Rgba::from_rgb(10, 132, 255);
const MUTED: Rgba = Rgba::from_argb(160, 245, 245, 247);

/// Opens `popover` anchored under the bar item whose right edge is at `anchor_right`, or
/// closes it when it is already open.
pub fn toggle(popover: Popover, anchor_right: f32) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.open = match state.open {
            Some((open, _)) if open == popover => None,
            _ => Some((popover, anchor_right)),
        };
    });
}

pub fn close() {
    STATE.with(|state| state.borrow_mut().open = None);
}

pub fn is_open() -> bool {
    STATE.with(|state| state.borrow().open.is_some())
}

pub fn set_data(data: PopoverData) {
    STATE.with(|state| state.borrow_mut().data = data);
}

/// Remembers a notch activity for the Notifications panel (newest first, at most 8).
pub fn record(primary: &str, secondary: &str) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        // Updates of the same activity (a volume drag, a call timer) replace the newest entry.
        if state
            .history
            .first()
            .is_some_and(|(first, _)| first == primary)
        {
            state.history.remove(0);
        }
        state
            .history
            .insert(0, (primary.to_string(), secondary.to_string()));
        state.history.truncate(8);
    });
}

/// The open panel's rectangle in window pixels, for hit testing.
pub fn rect() -> Option<Rect> {
    STATE.with(|state| state.borrow().rect)
}

pub fn hit(point: Point) -> Option<PopHit> {
    STATE.with(|state| {
        let state = state.borrow();
        let panel = state.rect?;
        if !panel.contains(point) {
            return None;
        }
        Some(
            state
                .targets
                .iter()
                .find(|(rect, _)| rect.contains(point))
                .map_or(PopHit::Panel, |(rect, hit)| match hit {
                    PopHit::Volume(_) => {
                        PopHit::Volume(((point.x - rect.left) / rect.width()).clamp(0.0, 1.0))
                    }
                    other => *other,
                }),
        )
    })
}

fn text(painter: Painter<'_>, value: &str, x: f32, y: f32, size: f32, bold: bool, color: Rgba) {
    FontManager::global().draw_text_cached(DrawTextCachedParams {
        painter,
        text: value,
        x,
        y,
        size,
        bold,
        color,
        blur: None,
    });
}

/// Draws the open popover under the bar. `top` is the bar's height and `width` the window's.
pub fn draw(painter: Painter<'_>, top: f32, width: f32, scale: f32) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.targets.clear();
        let Some((popover, anchor)) = state.open else {
            state.rect = None;
            return;
        };
        let s = scale;
        let w = WIDTH * s;
        let height = match popover {
            Popover::ControlCentre => 300.0,
            Popover::Calendar => 290.0,
            Popover::Notifications => 76.0 + 46.0 * state.history.len().max(1) as f32,
        } * s;
        let left = (anchor - w).clamp(8.0 * s, width - w - 8.0 * s);
        let panel = Rect::from_xywh(left, top + 6.0 * s, w, height);
        draw_glass(painter, panel, s);
        painter.stroke_round_rect(
            panel,
            Radius::uniform(16.0 * s),
            1.0,
            Rgba::WHITE.with_alpha(28),
        );
        let mut targets = Vec::new();
        let inner = Rect::from_ltrb(
            panel.left + PAD * s,
            panel.top + PAD * s,
            panel.right - PAD * s,
            panel.bottom - PAD * s,
        );
        match popover {
            Popover::ControlCentre => {
                draw_control_centre(painter, inner, s, &state.data, &mut targets)
            }
            Popover::Calendar => draw_calendar(painter, inner, s, &state.data),
            Popover::Notifications => {
                draw_notifications(painter, inner, s, &state.history, &mut targets)
            }
        }
        state.rect = Some(panel);
        state.targets = targets;
    });
}

fn draw_control_centre(
    painter: Painter<'_>,
    area: Rect,
    s: f32,
    data: &PopoverData,
    targets: &mut Vec<(Rect, PopHit)>,
) {
    // Four tiles in a 2x2 grid: each opens its Windows settings page.
    const TILES: [(&str, &str, &str); 4] = [
        ("Wi-Fi", "Networks", "ms-settings:network-wifi"),
        ("Bluetooth", "Devices", "ms-settings:bluetooth"),
        ("Focus", "Notifications", "ms-settings:quiethours"),
        ("Night light", "Display", "ms-settings:nightlight"),
    ];
    let gap = 8.0 * s;
    let tile_w = (area.width() - gap) / 2.0;
    let tile_h = 52.0 * s;
    for (index, (title, sub, uri)) in TILES.iter().enumerate() {
        let x = area.left + (index % 2) as f32 * (tile_w + gap);
        let y = area.top + (index / 2) as f32 * (tile_h + gap);
        let tile = Rect::from_xywh(x, y, tile_w, tile_h);
        painter.fill_round_rect(tile, Radius::uniform(12.0 * s), TILE);
        let bubble = Point::new(tile.left + 22.0 * s, tile.center_y());
        painter.fill_circle(
            bubble,
            13.0 * s,
            if index == 0 {
                BLUE
            } else {
                Rgba::WHITE.with_alpha(36)
            },
        );
        let glyph = [super::WIFI, super::BLUETOOTH, super::MOON, super::SUN][index];
        super::draw_glyph(
            painter,
            glyph,
            Rect::from_xywh(bubble.x - 11.0 * s, bubble.y - 11.0 * s, 22.0 * s, 22.0 * s),
            s * 0.8,
        );
        text(
            painter,
            title,
            tile.left + 42.0 * s,
            tile.top + 22.0 * s,
            12.5 * s,
            true,
            Rgba::WHITE,
        );
        text(
            painter,
            sub,
            tile.left + 42.0 * s,
            tile.top + 38.0 * s,
            11.0 * s,
            false,
            MUTED,
        );
        targets.push((tile, PopHit::Settings(uri)));
    }
    let mut y = area.top + 2.0 * (tile_h + gap);
    // Volume slider (real, sets the default output).
    let block = Rect::from_xywh(area.left, y, area.width(), 58.0 * s);
    painter.fill_round_rect(block, Radius::uniform(12.0 * s), TILE);
    text(
        painter,
        "Sound",
        block.left + 12.0 * s,
        block.top + 20.0 * s,
        12.0 * s,
        true,
        Rgba::WHITE,
    );
    let track = Rect::from_xywh(
        block.left + 12.0 * s,
        block.top + 30.0 * s,
        block.width() - 24.0 * s,
        18.0 * s,
    );
    painter.fill_round_rect(track, Radius::uniform(9.0 * s), Rgba::WHITE.with_alpha(40));
    let level = data.volume.unwrap_or(0.0).clamp(0.0, 1.0);
    painter.fill_round_rect(
        Rect::from_xywh(
            track.left,
            track.top,
            (track.width() * level).max(track.height()),
            track.height(),
        ),
        Radius::uniform(9.0 * s),
        Rgba::WHITE,
    );
    targets.push((track, PopHit::Volume(level)));
    y = block.bottom + gap;
    // Battery
    let row = Rect::from_xywh(area.left, y, area.width(), 34.0 * s);
    painter.fill_round_rect(row, Radius::uniform(10.0 * s), TILE);
    let battery = match data.battery {
        Some((level, true)) => format!("Battery {level}% · Charging"),
        Some((level, false)) => format!("Battery {level}%"),
        None => "Plugged in".to_string(),
    };
    text(
        painter,
        &battery,
        row.left + 12.0 * s,
        row.center_y() + 4.5 * s,
        12.5 * s,
        false,
        Rgba::WHITE,
    );
    targets.push((row, PopHit::Settings("ms-settings:batterysaver")));
    y = row.bottom + gap;
    // Now playing
    let row = Rect::from_xywh(area.left, y, area.width(), (area.bottom - y).max(30.0 * s));
    painter.fill_round_rect(row, Radius::uniform(10.0 * s), TILE);
    match &data.track {
        Some((title, artist, playing)) => {
            let fonts = FontManager::global();
            let line = crate::utils::settings_ui::ellipsize_text(
                fonts,
                &format!("{title} · {artist}"),
                12.5 * s,
                FontStyle::normal(),
                row.width() - 60.0 * s,
            );
            text(
                painter,
                &line,
                row.left + 12.0 * s,
                row.center_y() + 4.5 * s,
                12.5 * s,
                false,
                Rgba::WHITE,
            );
            let button = Rect::from_xywh(
                row.right - 40.0 * s,
                row.center_y() - 13.0 * s,
                28.0 * s,
                26.0 * s,
            );
            painter.fill_round_rect(
                button,
                Radius::uniform(13.0 * s),
                Rgba::WHITE.with_alpha(36),
            );
            text(
                painter,
                if *playing { "❚❚" } else { "▶" },
                button.left + 8.0 * s,
                button.center_y() + 4.5 * s,
                12.0 * s,
                true,
                Rgba::WHITE,
            );
            targets.push((button, PopHit::PlayPause));
        }
        None => text(
            painter,
            "Nothing playing",
            row.left + 12.0 * s,
            row.center_y() + 4.5 * s,
            12.5 * s,
            false,
            MUTED,
        ),
    }
}

fn draw_calendar(painter: Painter<'_>, area: Rect, s: f32, data: &PopoverData) {
    let now = crate::platform::shell().local_datetime();
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
    let month = usize::from(now.month.clamp(1, 12) - 1);
    text(
        painter,
        &format!("{} {}", MONTHS[month], now.year),
        area.left,
        area.top + 16.0 * s,
        15.0 * s,
        true,
        Rgba::WHITE,
    );
    // Month grid: weekday headers, then up to six weeks.
    let days_in_month = match now.month {
        2 if (now.year % 4 == 0 && now.year % 100 != 0) || now.year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    // Weekday of the 1st: today's weekday minus the days since the 1st.
    let first = (i32::from(now.day_of_week) - (i32::from(now.day) - 1)).rem_euclid(7);
    let cell_w = area.width() / 7.0;
    let cell_h = 24.0 * s;
    let grid_top = area.top + 30.0 * s;
    for (index, label) in ["S", "M", "T", "W", "T", "F", "S"].iter().enumerate() {
        text(
            painter,
            label,
            area.left + index as f32 * cell_w + cell_w / 2.0 - 3.5 * s,
            grid_top + 12.0 * s,
            10.5 * s,
            true,
            MUTED,
        );
    }
    for day in 1..=days_in_month {
        let slot = first + day as i32 - 1;
        let (col, row) = ((slot % 7) as f32, (slot / 7) as f32);
        let cx = area.left + col * cell_w + cell_w / 2.0;
        let cy = grid_top + 18.0 * s + row * cell_h + cell_h / 2.0;
        let today = day == i32::from(now.day);
        if today {
            painter.fill_circle(Point::new(cx, cy), 11.0 * s, BLUE);
        }
        let label = day.to_string();
        let offset = if day < 10 { 3.5 } else { 7.0 } * s;
        text(
            painter,
            &label,
            cx - offset,
            cy + 4.5 * s,
            12.0 * s,
            today,
            Rgba::WHITE,
        );
    }
    // Next events
    let mut y = grid_top + 18.0 * s + 6.0 * cell_h + 8.0 * s;
    if data.agenda.is_empty() {
        text(
            painter,
            "No upcoming events",
            area.left,
            y + 12.0 * s,
            12.0 * s,
            false,
            MUTED,
        );
    }
    for (title, detail) in data.agenda.iter().take(2) {
        painter.fill_round_rect(
            Rect::from_xywh(area.left, y + 2.0 * s, 3.0 * s, 26.0 * s),
            Radius::uniform(1.5 * s),
            BLUE,
        );
        text(
            painter,
            title,
            area.left + 10.0 * s,
            y + 13.0 * s,
            12.5 * s,
            true,
            Rgba::WHITE,
        );
        text(
            painter,
            detail,
            area.left + 10.0 * s,
            y + 27.0 * s,
            11.0 * s,
            false,
            MUTED,
        );
        y += 34.0 * s;
    }
}

fn draw_notifications(
    painter: Painter<'_>,
    area: Rect,
    s: f32,
    history: &[(String, String)],
    targets: &mut Vec<(Rect, PopHit)>,
) {
    text(
        painter,
        "Recent",
        area.left,
        area.top + 14.0 * s,
        12.0 * s,
        true,
        MUTED,
    );
    let mut y = area.top + 24.0 * s;
    if history.is_empty() {
        text(
            painter,
            "Nothing yet",
            area.left,
            y + 18.0 * s,
            12.5 * s,
            false,
            MUTED,
        );
        y += 46.0 * s;
    }
    let fonts = FontManager::global();
    for (primary, secondary) in history {
        let row = Rect::from_xywh(area.left, y, area.width(), 40.0 * s);
        painter.fill_round_rect(row, Radius::uniform(10.0 * s), TILE);
        text(
            painter,
            primary,
            row.left + 12.0 * s,
            row.top + 17.0 * s,
            12.5 * s,
            true,
            Rgba::WHITE,
        );
        let detail = crate::utils::settings_ui::ellipsize_text(
            fonts,
            secondary,
            11.0 * s,
            FontStyle::normal(),
            row.width() - 24.0 * s,
        );
        text(
            painter,
            &detail,
            row.left + 12.0 * s,
            row.top + 32.0 * s,
            11.0 * s,
            false,
            MUTED,
        );
        y += 46.0 * s;
    }
    let button = Rect::from_xywh(area.left, area.bottom - 28.0 * s, area.width(), 28.0 * s);
    painter.fill_round_rect(button, Radius::uniform(9.0 * s), BLUE.with_alpha(60));
    text(
        painter,
        "Windows notification centre",
        button.left + 12.0 * s,
        button.center_y() + 4.5 * s,
        12.5 * s,
        true,
        Rgba::WHITE,
    );
    targets.push((button, PopHit::WindowsNotifications));
}

/// A frosted-glass look without a live backdrop blur: a soft shadow, a dark translucent body
/// with a lighter top, and a fine highlight along the upper edge.
fn draw_glass(painter: Painter<'_>, panel: Rect, s: f32) {
    use winisland_render::{BlurSpec, GradientStop, Path, TileMode};
    let radius = Radius::uniform(16.0 * s);
    painter.fill_path_blurred(
        &Path::continuous_rounded_rect(
            panel.offset(winisland_render::Vec2::new(0.0, 10.0 * s)),
            16.0 * s,
        ),
        Rgba::from_argb(110, 0, 0, 0),
        BlurSpec {
            sigma: (18.0 * s, 18.0 * s),
            tile: None,
        },
    );
    painter.fill_round_rect(panel, radius, GLASS);
    painter.fill_round_rect_with_gradient(
        panel,
        radius,
        Point::new(panel.left, panel.top),
        Point::new(panel.left, panel.top + 120.0 * s),
        &[
            GradientStop {
                offset: 0.0,
                color: Rgba::from_argb(26, 255, 255, 255),
            },
            GradientStop {
                offset: 1.0,
                color: Rgba::from_argb(0, 255, 255, 255),
            },
        ],
        TileMode::Clamp,
    );
    // Fine top highlight that catches the light like a glass edge.
    painter.fill_round_rect(
        Rect::from_xywh(
            panel.left + 16.0 * s,
            panel.top + 0.5,
            panel.width() - 32.0 * s,
            1.0,
        ),
        Radius::uniform(0.5),
        Rgba::WHITE.with_alpha(46),
    );
}
