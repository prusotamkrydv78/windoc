//! The top bar that replaces the Windows taskbar: a full-width black strip with Start and the
//! running apps on the left, the notch in the middle, and status plus the clock on the right.
//!
//! The app publishes state with the `set_*` functions; drawing and hit testing read it.

pub mod popover;

use std::cell::RefCell;

use winisland_render::text::{DrawTextCachedParams, FontManager};
use winisland_render::{FontStyle, Painter, Point, Radius, Rect, Rgba};

/// One running app button.
#[derive(Clone, Debug, PartialEq)]
pub struct BarApp {
    pub handle: isize,
    pub process_id: u32,
    pub label: String,
    pub title: String,
    pub foreground: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarHit {
    Start,
    /// Opens the panel with the command palette focused.
    Search,
    App(isize),
    Clock,
    /// Wi-Fi, volume and battery: opens Quick Settings.
    Status,
    /// The thin strip at the far right edge: minimise or restore all windows.
    ShowDesktop,
    /// Hidden tray icons: opens the Windows tray overflow.
    Tray,
    Notifications,
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
        let search = Rect::from_xywh(start.right + 2.0 * scale, item_top, 28.0 * scale, item_h);
        hover_fill(painter, search, BarHit::Search, false);
        draw_glyph(painter, SEARCH, search, scale);
        targets.push((search, BarHit::Search));
        x = search.right + 6.0 * scale;
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
            let icon = icon_for(app.handle);
            if let Some(icon) = &icon {
                painter.draw_image(icon, tile_rect, &winisland_render::ImageOptions::default());
            } else {
                painter.fill_round_rect(
                    tile_rect,
                    Radius::uniform(4.5 * scale),
                    tile_color(&app.label),
                );
            }
            // Apps without an icon get a monogram tile.
            let initial: String = if icon.is_some() {
                String::new()
            } else {
                app.label.chars().take(1).collect()
            };
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

        // ---- Right: notifications, clock, then the Wi-Fi / volume / battery group ----
        // Show-desktop sliver at the very edge, as on the classic taskbar.
        let desktop = Rect::from_xywh(width - 6.0 * scale, 0.0, 6.0 * scale, height);
        if hovered == Some(BarHit::ShowDesktop) {
            painter.fill_rect(desktop, Rgba::WHITE.with_alpha(40));
        }
        targets.push((desktop, BarHit::ShowDesktop));
        let mut right = width - SIDE_PADDING * scale - 4.0 * scale;
        let bell = Rect::from_xywh(right - 28.0 * scale, item_top, 28.0 * scale, item_h);
        hover_fill(painter, bell, BarHit::Notifications, false);
        draw_glyph(painter, BELL, bell, scale);
        targets.push((bell, BarHit::Notifications));
        right = bell.left - 2.0 * scale;
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

        // Status group: one button covering battery, volume and Wi-Fi, opening Quick Settings.
        let group_right = right;
        let mut group_left = right - 8.0 * scale;
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
            group_left = body.left - 6.0 * scale - label_w - 8.0 * scale;
        }
        for glyph in [SPEAKER, WIFI] {
            let rect = Rect::from_xywh(group_left - 20.0 * scale, item_top, 20.0 * scale, item_h);
            draw_glyph(painter, glyph, rect, scale);
            group_left = rect.left - 2.0 * scale;
        }
        let group = Rect::from_ltrb(
            group_left - 4.0 * scale,
            item_top,
            group_right,
            item_top + item_h,
        );
        if hovered == Some(BarHit::Status) {
            painter.fill_round_rect(
                group,
                Radius::uniform(6.0 * scale),
                Rgba::WHITE.with_alpha(26),
            );
        }
        targets.push((group, BarHit::Status));
        let tray = Rect::from_xywh(group.left - 26.0 * scale, item_top, 24.0 * scale, item_h);
        let (microphone, camera) = crate::ui::island::capture_in_use();
        if microphone || camera {
            // Privacy dot left of the tray chevron: orange for the mic, green for the camera.
            let color = if microphone {
                Rgba::from_rgb(255, 159, 10)
            } else {
                Rgba::from_rgb(48, 209, 88)
            };
            painter.fill_circle(
                Point::new(group.left - 34.0 * scale, height / 2.0),
                3.5 * scale,
                color,
            );
        }
        if hovered == Some(BarHit::Tray) {
            painter.fill_round_rect(
                tray,
                Radius::uniform(6.0 * scale),
                Rgba::WHITE.with_alpha(26),
            );
        }
        draw_glyph(painter, CHEVRON, tray, scale);
        targets.push((tray, BarHit::Tray));
        state.targets = targets;
    });
}

/// 24x24 line glyphs for the status area (original artwork).
pub(crate) const WIFI: &str =
    "M2.5 9a14 14 0 0 1 19 0M5.5 12.5a9.5 9.5 0 0 1 13 0M8.8 16a4.7 4.7 0 0 1 6.4 0M12 19.3h.01";
const SPEAKER: &str =
    "M4 9.5h3.5L12 5.5v13l-4.5-4H4zM15.5 9.5a3.5 3.5 0 0 1 0 5M18 7a7 7 0 0 1 0 10";
const BELL: &str = "M6 16.5V11a6 6 0 0 1 12 0v5.5l1.5 1.5h-15zM10 20.5a2.2 2.2 0 0 0 4 0";
const CHEVRON: &str = "M7 14.5l5-5 5 5";
const SEARCH: &str = "M11 4.5a6.5 6.5 0 1 1 0 13 6.5 6.5 0 0 1 0-13zM16 16l4.5 4.5";
pub(crate) const BLUETOOTH: &str = "M7 7l10 10-5 4V3l5 4L7 17";
pub(crate) const MOON: &str = "M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z";
pub(crate) const SUN: &str = "M12 8a4 4 0 1 1 0 8 4 4 0 0 1 0-8zM12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4";

/// Strokes a 24x24 SVG path glyph centred in `rect` at about 15 px.
pub(crate) fn draw_glyph(painter: Painter<'_>, svg: &str, rect: Rect, scale: f32) {
    thread_local! {
        static CACHE: RefCell<Vec<(&'static str, Option<winisland_render::Path>)>> = const { RefCell::new(Vec::new()) };
    }
    let svg: &'static str = match svg {
        WIFI => WIFI,
        SPEAKER => SPEAKER,
        CHEVRON => CHEVRON,
        SEARCH => SEARCH,
        BLUETOOTH => BLUETOOTH,
        MOON => MOON,
        SUN => SUN,
        _ => BELL,
    };
    let path = CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((_, path)) = cache.iter().find(|(key, _)| *key == svg) {
            return path.clone();
        }
        let path = winisland_render::Path::from_svg(svg);
        cache.push((svg, path.clone()));
        path
    });
    let Some(path) = path else {
        return;
    };
    let size = 15.0 * scale;
    painter.save();
    painter.translate(winisland_render::Vec2::new(
        rect.center_x() - size / 2.0,
        rect.center_y() - size / 2.0,
    ));
    painter.scale(winisland_render::Vec2::new(size / 24.0, size / 24.0));
    painter.stroke_path(
        &path,
        1.9,
        Rgba::WHITE,
        winisland_render::StrokeCap::Round,
        winisland_render::StrokeJoin::Round,
    );
    painter.restore();
}

/// Seconds per full pass of the edge light across the bar.
const EDGE_PASS_SECONDS: f32 = 6.0;
/// Length of the travelling light in logical pixels.
const EDGE_COMET: f32 = 260.0;

/// The light that travels around the notch outline, shoulder to shoulder, at a constant speed
/// measured along the line itself, looping with no gap.
pub fn draw_edge_light(
    painter: Painter<'_>,
    notch: Rect,
    radius: f32,
    shoulder: f32,
    scale: f32,
    seconds: f32,
) {
    let top = notch.top.min(0.0);
    let edge = winisland_render::Path::notch_outline(notch, top, radius, shoulder);
    let total = edge.length();
    if total <= 1.0 {
        return;
    }
    let comet = EDGE_COMET * scale;
    let head = (seconds / EDGE_PASS_SECONDS * total).rem_euclid(total);
    // The current comet plus, while it wraps, the previous one leaving on the right.
    let draw_layers = |head: f32| {
        // (length behind the head, stroke width, colour, blur sigma)
        let layers: [(f32, f32, Rgba, f32); 5] = [
            (1.0, 16.0, Rgba::from_argb(110, 143, 123, 255), 8.0),
            (0.8, 6.0, Rgba::from_argb(200, 167, 123, 255), 3.0),
            (1.0, 1.8, Rgba::from_argb(180, 91, 140, 255), 0.0),
            (0.45, 1.8, Rgba::from_argb(240, 255, 143, 207), 0.0),
            (0.12, 1.8, Rgba::WHITE, 0.0),
        ];
        for (fraction, stroke, color, sigma) in layers {
            let segment = edge.segment(head - comet * fraction, head);
            if segment.is_empty() {
                continue;
            }
            if sigma > 0.0 {
                painter.stroke_path_blurred(&segment, stroke * scale, color, sigma * scale);
            } else {
                painter.stroke_path(
                    &segment,
                    stroke * scale,
                    color,
                    winisland_render::StrokeCap::Round,
                    winisland_render::StrokeJoin::Round,
                );
            }
        }
    };
    draw_layers(head);
    if head < comet {
        draw_layers(head + total);
    }
}

/// Seconds since the bar first animated, for time-based effects.
pub fn animation_seconds() -> f32 {
    thread_local! {
        static START: std::time::Instant = std::time::Instant::now();
    }
    START.with(|start| start.elapsed().as_secs_f32())
}

/// Result of the last per-app volume change, handed from the audio worker to the UI thread.
static APP_VOLUME: std::sync::Mutex<Option<(String, f32)>> = std::sync::Mutex::new(None);

/// Scrolling over an app button changes that app's own volume (Windows per-app mixer).
/// Returns `true` when `point` was over an app.
pub fn scroll_app_volume(point: Point, steps: f32) -> bool {
    let Some(BarHit::App(handle)) = hit(point) else {
        return false;
    };
    let Some((process_id, label)) = STATE.with(|state| {
        state
            .borrow()
            .apps
            .iter()
            .find(|app| app.handle == handle)
            .map(|app| (app.process_id, app.label.clone()))
    }) else {
        return false;
    };
    std::thread::spawn(move || {
        if let Some(level) =
            winisland_platform_windows::shell_bar::adjust_app_volume(process_id, steps * 0.04)
            && let Ok(mut slot) = APP_VOLUME.lock()
        {
            *slot = Some((label, level));
        }
    });
    true
}

/// The latest per-app volume change, once.
pub fn take_app_volume() -> Option<(String, f32)> {
    APP_VOLUME.lock().ok().and_then(|mut slot| slot.take())
}

thread_local! {
    /// App icons by window handle; `None` remembers windows without an icon.
    static ICONS: RefCell<std::collections::HashMap<isize, Option<winisland_render::Image>>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Loads icons for windows seen for the first time and forgets closed ones.
pub fn refresh_icons(handles: &[isize]) {
    ICONS.with(|icons| {
        let mut icons = icons.borrow_mut();
        icons.retain(|handle, _| handles.contains(handle));
        for &handle in handles {
            icons.entry(handle).or_insert_with(|| {
                winisland_platform_windows::shell_bar::window_icon_rgba(handle).and_then(
                    |(width, height, rgba)| {
                        winisland_render::Image::from_rgba8(width, height, &rgba)
                    },
                )
            });
        }
    });
}

fn icon_for(handle: isize) -> Option<winisland_render::Image> {
    ICONS.with(|icons| icons.borrow().get(&handle).cloned().flatten())
}

thread_local! {
    /// Card behind the live window preview, in window pixels.
    static PREVIEW_CARD: std::cell::Cell<Option<Rect>> = const { std::cell::Cell::new(None) };
}

pub fn set_preview_card(card: Option<Rect>) {
    PREVIEW_CARD.with(|cell| cell.set(card));
}

/// The button rectangle of the app with `handle`, from the last drawn frame.
pub fn app_rect(handle: isize) -> Option<Rect> {
    STATE.with(|state| {
        state
            .borrow()
            .targets
            .iter()
            .find(|(_, hit)| *hit == BarHit::App(handle))
            .map(|(rect, _)| *rect)
    })
}

/// Draws the card that frames a live window preview; the preview itself is composited on
/// top by Windows.
pub fn draw_preview_card(painter: Painter<'_>, scale: f32) {
    let Some(card) = PREVIEW_CARD.with(std::cell::Cell::get) else {
        return;
    };
    painter.fill_round_rect(
        card,
        Radius::uniform(12.0 * scale),
        Rgba::from_argb(240, 28, 28, 30),
    );
    painter.stroke_round_rect(
        card,
        Radius::uniform(12.0 * scale),
        1.0,
        Rgba::WHITE.with_alpha(30),
    );
}

/// The rectangle of a bar item from the last drawn frame.
pub fn target_rect(target: BarHit) -> Option<Rect> {
    STATE.with(|state| {
        state
            .borrow()
            .targets
            .iter()
            .find(|(_, hit)| *hit == target)
            .map(|(rect, _)| *rect)
    })
}
