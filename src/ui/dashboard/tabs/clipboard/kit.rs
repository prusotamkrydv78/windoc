//! Drawing, text and time helpers shared by the Clipboard and Notes tabs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use winisland_render::text::FontManager;
use winisland_render::tokens::{SemanticColors, TextStyle, typography};
use winisland_render::{
    FontStyle, Painter, Path, Point, Radius, Rect, Rgba, StrokeCap, StrokeJoin, Vec2,
};

use crate::ui::components::text::{Align, draw_text};

/// Line glyphs on a 24×24 grid, stroked like the app's module icons.
pub mod glyphs {
    pub const PIN: &str = "M9 4 H15 M10 4 V10 L7 14 H17 L14 10 V4 M12 14 V20";
    /// The pin's head, filled when an item is pinned.
    pub const PIN_FILL: &str = "M10 4 H14 V10 L17 14 H7 L10 10 Z";
    pub const TRASH: &str = "M4 7 H20 M9 7 V4 H15 V7 M6 7 L7 20 H17 L18 7 M10 11 V16 M14 11 V16";
    pub const COPY: &str = "M9 9 H20 V20 H9 Z M6 15 H4 V4 H15 V6";
    pub const SEARCH: &str = "M11 4 A7 7 0 1 1 10.99 4 M16 16 L20 20";
    pub const PLUS: &str = "M12 5 V19 M5 12 H19";
    pub const CLOSE: &str = "M7 7 L17 17 M17 7 L7 17";
    pub const TEXT_DOC: &str =
        "M6 3 H14 L18 7 V21 H6 Z M14 3 V7 H18 M9 11 H15 M9 14.5 H15 M9 18 H13";
    pub const IMAGE: &str = "M4 5 H20 V19 H4 Z M4 16 L9 11 L13 15 L15 13 L20 18 \
                             M15 8 A1.5 1.5 0 1 1 14.99 8";
    pub const LINK: &str = "M10 14 L14 10 M9 7 L11 5 A4 4 0 0 1 17 11 L15 13 \
                            M15 17 L13 19 A4 4 0 0 1 7 13 L9 11";
    pub const FOLDER: &str = "M3 6 H9 L11 8 H21 V19 H3 Z";
    pub const NOTE: &str = "M5 4 H19 V14 L13 20 H5 Z M13 20 V14 H19";
    pub const CHECK_BOX: &str = "M5 5 H19 V19 H5 Z M8.5 12 L11 14.5 L15.5 9.5";
    pub const BULLETS: &str = "M9 6 H20 M9 12 H20 M9 18 H20";
    pub const DOWNLOAD: &str = "M12 4 V15 M7 10 L12 15 L17 10 M5 19 H19";
    pub const CHECK: &str = "M6 12.5 L10 16.5 L18 8";
    pub const CHEVRON_DOWN: &str = "M8 10 L12 14 L16 10";
}

thread_local! {
    static PATHS: RefCell<HashMap<&'static str, Option<Path>>> = RefCell::new(HashMap::new());
}

/// Strokes `svg` (a 24×24 path) centred in `rect`.
pub fn glyph(painter: Painter<'_>, svg: &'static str, rect: Rect, color: Rgba) {
    glyph_with(painter, svg, rect, color, 1.8, false);
}

/// Fills `svg` (a closed 24×24 path) centred in `rect`.
pub fn glyph_fill(painter: Painter<'_>, svg: &'static str, rect: Rect, color: Rgba) {
    glyph_with(painter, svg, rect, color, 0.0, true);
}

fn glyph_with(
    painter: Painter<'_>,
    svg: &'static str,
    rect: Rect,
    color: Rgba,
    width: f32,
    fill: bool,
) {
    let size = rect.width().min(rect.height());
    if size <= 0.0 {
        return;
    }
    PATHS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let Some(path) = cache
            .entry(svg)
            .or_insert_with(|| Path::from_svg(svg))
            .as_ref()
        else {
            return;
        };
        let scale = size / 24.0;
        let save = painter.save();
        painter.translate(Vec2::new(
            rect.center_x() - 12.0 * scale,
            rect.center_y() - 12.0 * scale,
        ));
        painter.scale(Vec2::new(scale, scale));
        if fill {
            painter.fill_path(path, color);
        } else {
            painter.stroke_path(path, width, color, StrokeCap::Round, StrokeJoin::Round);
        }
        painter.restore_to(save);
    });
}

/// A square of `size` centred in `rect`.
pub fn centered(rect: Rect, size: f32) -> Rect {
    Rect::from_xywh(
        rect.center_x() - size / 2.0,
        rect.center_y() - size / 2.0,
        size,
        size,
    )
}

pub fn font_style(style: TextStyle) -> FontStyle {
    if style.weight >= 600 {
        FontStyle::bold()
    } else {
        FontStyle::normal()
    }
}

pub fn text_width(text: &str, style: TextStyle) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    FontManager::global().measure_text_cached(text, style.size, font_style(style))
}

/// One line of start-aligned text, ellipsized to `rect`.
pub fn label(painter: Painter<'_>, text: &str, style: TextStyle, rect: Rect, color: Rgba) {
    draw_text(painter, text, style, rect, Align::Start, color);
}

/// Text right-aligned against `right`, vertically centred on `rect`. Returns its left edge.
pub fn label_right(
    painter: Painter<'_>,
    text: &str,
    style: TextStyle,
    rect: Rect,
    color: Rgba,
) -> f32 {
    let width = text_width(text, style).min(rect.width());
    let left = rect.right - width;
    draw_text(
        painter,
        text,
        style,
        Rect::from_ltrb(left - 1.0, rect.top, rect.right + 1.0, rect.bottom),
        Align::Start,
        color,
    );
    left
}

/// A round icon button: hover and active states share one soft fill.
pub fn icon_button(
    painter: Painter<'_>,
    rect: Rect,
    svg: &'static str,
    hovered: bool,
    active: bool,
    color: Rgba,
    colors: &SemanticColors,
) {
    if hovered || active {
        let fill = if active {
            colors.surface_interactive
        } else {
            colors.surface_overlay
        };
        painter.fill_round_rect(rect, Radius::uniform(rect.height() / 2.0 - 2.0), fill);
    }
    glyph(painter, svg, centered(rect, 16.0), color);
}

/// Width of one character at `size`, cached by the font manager.
fn char_width(character: char, style: FontStyle, size: f32) -> f32 {
    let mut buffer = [0u8; 4];
    FontManager::global().measure_text_cached(character.encode_utf8(&mut buffer), size, style)
}

/// Greedy word wrap of `chars` to `width`. Returns char ranges, one per visual line; an
/// empty input yields one empty line.
pub fn wrap(chars: &[char], style: TextStyle, width: f32) -> Vec<(usize, usize)> {
    let font = font_style(style);
    let mut lines = Vec::new();
    let (mut start, mut x, mut index) = (0usize, 0.0f32, 0usize);
    let mut last_space: Option<usize> = None;
    while index < chars.len() {
        let advance = char_width(chars[index], font, style.size);
        if x + advance > width && index > start && chars[index] != ' ' {
            let end = match last_space {
                Some(space) if space >= start && space + 1 < index + 1 => space + 1,
                _ => index,
            };
            lines.push((start, end));
            start = end;
            last_space = None;
            x = chars[start..index]
                .iter()
                .map(|character| char_width(*character, font, style.size))
                .sum();
            continue;
        }
        if chars[index] == ' ' {
            last_space = Some(index);
        }
        x += advance;
        index += 1;
    }
    lines.push((start, chars.len()));
    lines
}

/// Draws plain wrapped text from `top` down, clipped to `area`. Returns the bottom.
pub fn draw_wrapped(
    painter: Painter<'_>,
    text: &str,
    style: TextStyle,
    line_height: f32,
    area: Rect,
    color: Rgba,
) -> f32 {
    let mut top = area.top;
    'paragraphs: for paragraph in text.lines() {
        let chars: Vec<char> = paragraph.chars().collect();
        for (start, end) in wrap(&chars, style, area.width()) {
            if top + line_height > area.bottom + 0.5 {
                // Show that more follows on the last visible line.
                break 'paragraphs;
            }
            let line: String = chars[start..end].iter().collect();
            label(
                painter,
                line.trim_end(),
                style,
                Rect::from_xywh(area.left, top, area.width(), line_height),
                color,
            );
            top += line_height;
        }
    }
    top
}

pub fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

pub fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// A capsule search field with a caret. `query` empty shows `placeholder`.
pub fn search_field(
    painter: Painter<'_>,
    rect: Rect,
    query: &str,
    placeholder: &str,
    colors: &SemanticColors,
) {
    painter.fill_round_rect(
        rect,
        Radius::uniform(rect.height() / 2.0),
        colors.surface_raised,
    );
    painter.stroke_round_rect(
        rect,
        Radius::uniform(rect.height() / 2.0),
        1.0,
        colors.border_strong,
    );
    let icon = Rect::from_xywh(rect.left + 8.0, rect.top, 14.0, rect.height());
    glyph(
        painter,
        glyphs::SEARCH,
        centered(icon, 14.0),
        colors.text_secondary,
    );
    let text_left = icon.right + 6.0;
    let text_rect = Rect::from_ltrb(text_left, rect.top, rect.right - 24.0, rect.bottom);
    let style = typography::LABEL;
    if query.is_empty() {
        label(painter, placeholder, style, text_rect, colors.text_disabled);
    } else {
        label(painter, query, style, text_rect, colors.text_primary);
    }
    if caret_visible() {
        let x = (text_left + text_width(query, style) + 1.0).min(text_rect.right);
        painter.stroke_line(
            Point::new(x, rect.center_y() - 7.0),
            Point::new(x, rect.center_y() + 7.0),
            1.2,
            colors.accent_primary,
            StrokeCap::Butt,
        );
    }
}

/// The close "x" inside a search field from [`search_field`].
pub fn search_close(rect: Rect) -> Rect {
    Rect::from_xywh(rect.right - 24.0, rect.top, 22.0, rect.height())
}

pub fn caret_visible() -> bool {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(true, |elapsed| {
            (elapsed.as_millis() / 530).is_multiple_of(2)
        })
}

// ---- Local time -------------------------------------------------------------------------

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn unix_seconds(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs() as i64,
        Err(error) => -(error.duration().as_secs() as i64),
    }
}

/// The local clock's offset from UTC in seconds, rounded to a quarter hour.
fn local_offset() -> i64 {
    let local = crate::platform::shell().local_datetime();
    let local_seconds = crate::ui::dashboard::days_from_civil(
        i64::from(local.year),
        u32::from(local.month),
        u32::from(local.day),
    ) * 86_400
        + i64::from(local.hour) * 3600
        + i64::from(local.minute) * 60
        + i64::from(local.second);
    let offset = local_seconds - unix_seconds(SystemTime::now());
    (offset as f64 / 900.0).round() as i64 * 900
}

/// Local `(days since 1970, seconds into the day)` for `time`.
fn local_day_seconds(time: SystemTime) -> (i64, i64) {
    let seconds = unix_seconds(time) + local_offset();
    (seconds.div_euclid(86_400), seconds.rem_euclid(86_400))
}

/// Local days since 1970-01-01, for grouping by day.
pub fn local_day(time: SystemTime) -> i64 {
    local_day_seconds(time).0
}

pub fn today() -> i64 {
    local_day(SystemTime::now())
}

/// "Oct 28".
pub fn month_day(time: SystemTime) -> String {
    let (_, month, day) = crate::ui::dashboard::civil_from_days(local_day(time));
    format!(
        "{} {day}",
        MONTHS[(month as usize).saturating_sub(1).min(11)]
    )
}

/// "10:13 PM".
pub fn clock(time: SystemTime) -> String {
    let (_, seconds) = local_day_seconds(time);
    let (hour, minute) = (seconds / 3600, seconds / 60 % 60);
    let suffix = if hour < 12 { "AM" } else { "PM" };
    let hour12 = match hour % 12 {
        0 => 12,
        other => other,
    };
    format!("{hour12}:{minute:02} {suffix}")
}

/// "59.4 KB".
pub fn byte_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// "now", "4m ago", "23h ago", "3d ago", then the date.
pub fn ago(time: SystemTime) -> String {
    let seconds = unix_seconds(SystemTime::now()) - unix_seconds(time);
    if seconds < 60 {
        "now".to_string()
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3600)
    } else if seconds < 604_800 {
        format!("{}d ago", seconds / 86_400)
    } else {
        month_day(time)
    }
}
