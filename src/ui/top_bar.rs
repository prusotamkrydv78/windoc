//! The top bar that replaces the Windows taskbar: a full-width black strip with Start and the
//! running apps on the left, the notch in the middle, and status plus the clock on the right.
//!
//! The app publishes state with the `set_*` functions; drawing and hit testing read it.

use std::cell::RefCell;

use winisland_render::text::{DrawTextCachedParams, FontManager};
use winisland_render::{FontStyle, Painter, Point, Radius, Rect, Rgba};

/// One running app button.
#[derive(Clone, Debug, PartialEq)]
pub struct BarApp {
    pub handle: isize,
    pub label: String,
    pub title: String,
    pub foreground: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarHit {
    Start,
    App(isize),
    Clock,
}

#[derive(Default)]
struct BarState {
    active: bool,
    apps: Vec<BarApp>,
    battery: Option<(u8, bool)>,
    hovered: Option<BarHit>,
    /// Hit rectangles from the last draw, in window pixels.
    targets: Vec<(Rect, BarHit)>,
}

thread_local! {
    static STATE: RefCell<BarState> = RefCell::new(BarState::default());
}

const TEXT: f32 = 13.0;
const ITEM_HEIGHT: f32 = 24.0;
const SIDE_PADDING: f32 = 10.0;
const NOTCH_GAP: f32 = 16.0;
const MAX_TITLE: f32 = 180.0;

pub fn set_active(active: bool) {
    STATE.with(|state| state.borrow_mut().active = active);
}

pub fn is_active() -> bool {
    STATE.with(|state| state.borrow().active)
}

pub fn set_apps(apps: Vec<BarApp>) {
    STATE.with(|state| state.borrow_mut().apps = apps);
}

pub fn set_battery(battery: Option<(u8, bool)>) {
    STATE.with(|state| state.borrow_mut().battery = battery);
}

/// Returns `true` when the hovered item changed and the bar needs a redraw.
pub fn set_hover(point: Option<Point>) -> bool {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let hovered = point.and_then(|point| hit_in(&state.targets, point));
        let changed = state.hovered != hovered;
        state.hovered = hovered;
        changed
    })
}

/// The item under `point` (window pixels), from the last drawn frame.
pub fn hit(point: Point) -> Option<BarHit> {
    STATE.with(|state| hit_in(&state.borrow().targets, point))
}

fn hit_in(targets: &[(Rect, BarHit)], point: Point) -> Option<BarHit> {
    targets
        .iter()
        .find(|(rect, _)| rect.contains(point))
        .map(|(_, hit)| *hit)
}

/// Short label for an executable: `Code.exe` → `Code`.
pub fn app_label(executable: &str, title: &str) -> String {
    let stem = executable
        .rsplit_once('.')
        .map_or(executable, |(stem, _)| stem)
        .trim();
    if stem.is_empty() {
        title.chars().take(12).collect()
    } else {
        let mut chars = stem.chars();
        chars
            .next()
            .map(|first| first.to_uppercase().chain(chars).collect())
            .unwrap_or_default()
    }
}

/// Stable, pleasant colour for an app's monogram tile.
fn tile_color(label: &str) -> Rgba {
    const COLORS: [(u8, u8, u8); 8] = [
        (10, 132, 255),
        (48, 209, 88),
        (255, 159, 10),
        (191, 90, 242),
        (255, 69, 58),
        (100, 210, 255),
        (255, 55, 95),
        (94, 92, 230),
    ];
    let hash = label.bytes().fold(0u32, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(u32::from(byte))
    });
    let (r, g, b) = COLORS[hash as usize % COLORS.len()];
    Rgba::from_rgb(r, g, b)
}

/// Draws the bar across `width` x `height` window pixels. `notch` is the notch's horizontal
/// extent so the side items never run underneath it.
pub fn draw(painter: Painter<'_>, width: f32, height: f32, scale: f32, notch: (f32, f32)) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.targets.clear();
        if !state.active {
            return;
        }
        painter.fill_rect(Rect::from_xywh(0.0, 0.0, width, height), Rgba::BLACK);
        let fonts = FontManager::global();
        let text_size = TEXT * scale;
        let item_h = ITEM_HEIGHT * scale;
        let item_top = (height - item_h) / 2.0;
        let baseline = height / 2.0 + text_size * 0.36;
        let hovered = state.hovered;
        let mut targets = Vec::new();

        let hover_fill = |painter: Painter<'_>, rect: Rect, hit: BarHit, active: bool| {
            if active {
                painter.fill_round_rect(
                    rect,
                    Radius::uniform(6.0 * scale),
                    Rgba::WHITE.with_alpha(40),
                );
            } else if hovered == Some(hit) {
                painter.fill_round_rect(
                    rect,
                    Radius::uniform(6.0 * scale),
                    Rgba::WHITE.with_alpha(26),
                );
            }
        };

        // ---- Left: Start, then running apps ----
        let mut x = SIDE_PADDING * scale;
        let start = Rect::from_xywh(x, item_top, 30.0 * scale, item_h);
        hover_fill(painter, start, BarHit::Start, false);
        let cell = 6.4 * scale;
        let gap = 1.2 * scale;
        let origin_x = start.center_x() - cell - gap / 2.0;
        let origin_y = start.center_y() - cell - gap / 2.0;
        for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
            painter.fill_round_rect(
                Rect::from_xywh(
                    origin_x + dx * (cell + gap),
                    origin_y + dy * (cell + gap),
                    cell,
                    cell,
                ),
                Radius::uniform(1.2 * scale),
                Rgba::WHITE,
            );
        }
        targets.push((start, BarHit::Start));
        x = start.right + 6.0 * scale;
        painter.fill_rect(
            Rect::from_xywh(x, height / 2.0 - 7.0 * scale, 1.0, 14.0 * scale),
            Rgba::WHITE.with_alpha(40),
        );
        x += 7.0 * scale;

        let left_limit = notch.0 - NOTCH_GAP * scale;
        for app in &state.apps {
            let tile = 17.0 * scale;
            let title_width = if app.foreground {
                let title = crate::utils::settings_ui::ellipsize_text(
                    fonts,
                    &app.label,
                    text_size,
                    FontStyle::bold(),
                    MAX_TITLE * scale,
                );
                fonts.measure_text_cached(&title, text_size, FontStyle::bold()) + 7.0 * scale
            } else {
                0.0
            };
            let button_w = 7.0 * scale * 2.0 + tile + title_width;
            if x + button_w > left_limit {
                break;
            }
            let button = Rect::from_xywh(x, item_top, button_w, item_h);
            hover_fill(painter, button, BarHit::App(app.handle), false);
            let tile_rect = Rect::from_xywh(x + 7.0 * scale, height / 2.0 - tile / 2.0, tile, tile);
            painter.fill_round_rect(
                tile_rect,
                Radius::uniform(4.5 * scale),
                tile_color(&app.label),
            );
            let initial: String = app.label.chars().take(1).collect();
            let initial_size = 9.5 * scale;
            let initial_w = fonts.measure_text_cached(&initial, initial_size, FontStyle::bold());
            fonts.draw_text_cached(DrawTextCachedParams {
                painter,
                text: &initial,
                x: tile_rect.center_x() - initial_w / 2.0,
                y: tile_rect.center_y() + initial_size * 0.36,
                size: initial_size,
                bold: true,
                color: Rgba::WHITE,
                blur: None,
            });
            if app.foreground {
                let title = crate::utils::settings_ui::ellipsize_text(
                    fonts,
                    &app.label,
                    text_size,
                    FontStyle::bold(),
                    MAX_TITLE * scale,
                );
                fonts.draw_text_cached(DrawTextCachedParams {
                    painter,
                    text: &title,
                    x: tile_rect.right + 7.0 * scale,
                    y: baseline,
                    size: text_size,
                    bold: true,
                    color: Rgba::WHITE,
                    blur: None,
                });
            }
            // Focus underline: a wide white pill for the active app, a dot for the rest.
            let underline_w = if app.foreground {
                12.0 * scale
            } else {
                3.0 * scale
            };
            painter.fill_round_rect(
                Rect::from_xywh(
                    tile_rect.center_x() - underline_w / 2.0,
                    button.bottom - 2.0 * scale,
                    underline_w,
                    2.0 * scale,
                ),
                Radius::uniform(1.0 * scale),
                if app.foreground {
                    Rgba::WHITE
                } else {
                    Rgba::WHITE.with_alpha(100)
                },
            );
            targets.push((button, BarHit::App(app.handle)));
            x = button.right + 1.0 * scale;
        }

        // ---- Right: battery and clock ----
        let mut right = width - SIDE_PADDING * scale;
        let now = crate::platform::shell().local_datetime();
        const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let hour12 = match now.hour % 12 {
            0 => 12,
            hour => hour,
        };
        let clock = format!(
            "{} {} {}  {}:{:02} {}",
            DAYS[usize::from(now.day_of_week % 7)],
            now.day,
            MONTHS[usize::from(now.month.clamp(1, 12) - 1)],
            hour12,
            now.minute,
            if now.hour < 12 { "AM" } else { "PM" }
        );
        let clock_w = fonts.measure_text_cached(&clock, text_size, FontStyle::normal());
        let clock_rect = Rect::from_xywh(
            right - clock_w - 18.0 * scale,
            item_top,
            clock_w + 18.0 * scale,
            item_h,
        );
        hover_fill(painter, clock_rect, BarHit::Clock, false);
        fonts.draw_text_cached(DrawTextCachedParams {
            painter,
            text: &clock,
            x: clock_rect.left + 9.0 * scale,
            y: baseline,
            size: text_size,
            bold: false,
            color: Rgba::WHITE,
            blur: None,
        });
        targets.push((clock_rect, BarHit::Clock));
        right = clock_rect.left - 4.0 * scale;

        if let Some((level, charging)) = state.battery {
            let body = Rect::from_xywh(
                right - 26.0 * scale,
                height / 2.0 - 6.0 * scale,
                22.0 * scale,
                12.0 * scale,
            );
            painter.stroke_round_rect(
                body,
                Radius::uniform(3.2 * scale),
                1.2 * scale,
                Rgba::WHITE.with_alpha(140),
            );
            painter.fill_round_rect(
                Rect::from_xywh(
                    body.right + 1.0 * scale,
                    body.center_y() - 2.5 * scale,
                    2.0 * scale,
                    5.0 * scale,
                ),
                Radius::uniform(1.0 * scale),
                Rgba::WHITE.with_alpha(140),
            );
            let fill = if charging {
                Rgba::from_rgb(52, 199, 89)
            } else if level <= 15 {
                Rgba::from_rgb(255, 69, 58)
            } else {
                Rgba::WHITE
            };
            painter.fill_round_rect(
                Rect::from_xywh(
                    body.left + 1.8 * scale,
                    body.top + 1.8 * scale,
                    ((body.width() - 3.6 * scale) * f32::from(level.min(100)) / 100.0).max(1.0),
                    body.height() - 3.6 * scale,
                ),
                Radius::uniform(1.8 * scale),
                fill,
            );
            let label = format!("{level}%");
            let label_w = fonts.measure_text_cached(&label, text_size, FontStyle::normal());
            fonts.draw_text_cached(DrawTextCachedParams {
                painter,
                text: &label,
                x: body.left - 6.0 * scale - label_w,
                y: baseline,
                size: text_size,
                bold: false,
                color: Rgba::WHITE,
                blur: None,
            });
        }
        state.targets = targets;
    });
}
