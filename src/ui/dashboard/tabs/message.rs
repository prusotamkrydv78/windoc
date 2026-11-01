//! Message tab: write a short banner and flash it in the collapsed notch.
//!
//! Layout, top to bottom: a live preview of the banner (drawn by the notch widget itself,
//! so it matches), the text field with Flash and Clear, then three cards: Animation
//! (style chips and a speed slider), Colour (presets, a custom hue and the finish) and
//! Duration (3s / 5s / 10s / custom, and "repeat until taken down").
//!
//! Flash hands the banner to `widget::compact::message::flash`, which wins over the
//! configured status message until it expires or Clear takes it down.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use winisland_platform::Key;
use winisland_render::text::FontManager;
use winisland_render::tokens::{SemanticColors, TextStyle, radius, typography};
use winisland_render::{
    FontStyle, GradientStop, Painter, PathBuilder, Point, Radius, Rect, Rgba, StrokeCap,
    StrokeJoin, TileMode, Vec2,
};

use super::TabContext;
use crate::ui::components::text::{Align, draw_text};
use crate::ui::dashboard::{DashboardHit, Destination};
use crate::ui::widget::compact::message::{
    self as banner, Animation, BannerStyle, Finish, banner_width, draw_banner, intro_seconds,
};

const MAX_CHARS: usize = 60;
const DEFAULT_TEXT: &str = "Back in 10 \u{2014} grabbing chai \u{2615}";

const GAP: f32 = 8.0;
const PAD: f32 = 12.0;
const PREVIEW_H: f32 = 48.0;
const ROW_H: f32 = 36.0;
const FLASH_W: f32 = 100.0;
const CLEAR_W: f32 = 84.0;
const CHIP_H: f32 = 24.0;
const CHIP_GAP: f32 = 6.0;
/// The preview draws the banner a little larger than the notch so it reads at a glance.
const PREVIEW_SCALE: f32 = 1.25;
/// Notch pill height at 1x, around the 11 px banner text.
const PILL_H: f32 = 24.0;

const CHIP: TextStyle = TextStyle {
    size: 13.0,
    weight: 500,
    tabular: false,
};
const CHIP_ON: TextStyle = TextStyle {
    size: 13.0,
    weight: 650,
    tabular: false,
};
const FIELD: TextStyle = TextStyle {
    size: 14.0,
    weight: 500,
    tabular: false,
};
const BUTTON: TextStyle = TextStyle {
    size: 14.0,
    weight: 650,
    tabular: false,
};
const COUNTER: TextStyle = TextStyle {
    size: 12.0,
    weight: 500,
    tabular: true,
};

const GREEN: Rgba = Rgba::from_rgb(48, 209, 88);
const PRESETS: [Rgba; 7] = [
    Rgba::from_rgb(255, 255, 255),
    Rgba::from_rgb(246, 222, 196),
    Rgba::from_rgb(245, 166, 35),
    Rgba::from_rgb(240, 112, 86),
    Rgba::from_rgb(240, 64, 100),
    Rgba::from_rgb(160, 92, 240),
    Rgba::from_rgb(92, 192, 245),
];
/// Seconds offered by "Custom", in click order.
const CUSTOM_SECONDS: [u64; 5] = [15, 30, 60, 120, 300];
const DURATIONS: [u64; 3] = [3, 5, 10];

// Hit ids (1000-1099).
const ID_FIELD: u16 = 1000;
const ID_FLASH: u16 = 1001;
const ID_CLEAR: u16 = 1002;
const ID_PREVIEW: u16 = 1003;
const ID_ANIMATION: u16 = 1010; // + index (6)
const ID_COLOR: u16 = 1020; // + index (7 presets), 1027 custom
const ID_CUSTOM_COLOR: u16 = 1027;
const ID_FINISH: u16 = 1030; // + index (4)
const ID_DURATION: u16 = 1040; // + index (4: 3s, 5s, 10s, custom)
const ID_REPEAT: u16 = 1045;
const ID_SPEED: u16 = 1070; // + step (0..=20)
const SPEED_STEPS: u16 = 20;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ColorChoice {
    Preset(usize),
    Custom,
}

struct State {
    text: String,
    /// Caret position in characters.
    caret: usize,
    editing: bool,
    /// When the caret last moved, so it stays solid while typing.
    caret_moved: Instant,
    animation: Animation,
    /// Slider position 0..=1; 0.5 is normal speed.
    speed: f32,
    color: ColorChoice,
    custom_hue: f32,
    finish: Finish,
    /// 0..=2 for 3s / 5s / 10s, 3 for custom.
    duration: usize,
    custom_index: usize,
    repeat: bool,
    /// Restarts the looping preview whenever the look changes.
    preview_started: Instant,
}

impl State {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            text: DEFAULT_TEXT.to_string(),
            caret: DEFAULT_TEXT.chars().count(),
            editing: false,
            caret_moved: now,
            animation: Animation::Slide,
            speed: 0.5,
            color: ColorChoice::Preset(0),
            custom_hue: 160.0,
            finish: Finish::Aurora,
            duration: 2,
            custom_index: 1,
            repeat: false,
            preview_started: now,
        }
    }

    fn color(&self) -> Rgba {
        match self.color {
            ColorChoice::Preset(index) => PRESETS[index.min(PRESETS.len() - 1)],
            ColorChoice::Custom => hue_color(self.custom_hue),
        }
    }

    fn style(&self) -> BannerStyle {
        BannerStyle {
            animation: self.animation,
            finish: self.finish,
            color: self.color(),
            speed: speed_multiplier(self.speed),
        }
    }

    fn hold_seconds(&self) -> u64 {
        if self.duration < DURATIONS.len() {
            DURATIONS[self.duration]
        } else {
            CUSTOM_SECONDS[self.custom_index % CUSTOM_SECONDS.len()]
        }
    }

    fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    fn byte_at(&self, chars: usize) -> usize {
        self.text
            .char_indices()
            .nth(chars)
            .map_or(self.text.len(), |(index, _)| index)
    }

    fn restart_preview(&mut self) {
        self.preview_started = Instant::now();
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::new());
}

/// 0..1 slider to a 0.5x..2x multiplier, 1x in the middle.
fn speed_multiplier(slider: f32) -> f32 {
    0.5 * 4f32.powf(slider.clamp(0.0, 1.0))
}

fn speed_label(slider: f32) -> &'static str {
    if slider < 0.3 {
        "Slow"
    } else if slider <= 0.7 {
        "Normal"
    } else {
        "Fast"
    }
}

/// "10s", "1m", "2m 30s".
fn format_seconds(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds % 60 == 0 {
        format!("{}m", seconds / 60)
    } else {
        format!("{}m {}s", seconds / 60, seconds % 60)
    }
}

/// A saturated, light colour for `hue` degrees.
fn hue_color(hue: f32) -> Rgba {
    let h = hue.rem_euclid(360.0) / 60.0;
    let (s, v) = (0.62_f32, 0.98_f32);
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |value: f32| ((value + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgba::from_rgb(to(r), to(g), to(b))
}

// ---------------------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------------------

struct Layout {
    preview: Rect,
    field: Rect,
    flash: Rect,
    clear: Rect,
    animation: Rect,
    colour: Rect,
    duration: Rect,
}

fn layout(grid: Rect) -> Layout {
    let preview = Rect::from_xywh(grid.left, grid.top, grid.width(), PREVIEW_H);
    let row_top = preview.bottom + GAP;
    let clear = Rect::from_xywh(grid.right - CLEAR_W, row_top, CLEAR_W, ROW_H);
    let flash = Rect::from_xywh(clear.left - GAP - FLASH_W, row_top, FLASH_W, ROW_H);
    let field = Rect::from_ltrb(grid.left, row_top, flash.left - GAP, row_top + ROW_H);
    let cards_top = row_top + ROW_H + GAP;
    let card_w = (grid.width() - GAP * 2.0) / 3.0;
    let card_h = (grid.bottom - cards_top).max(118.0);
    let card = |index: f32| {
        Rect::from_xywh(
            grid.left + index * (card_w + GAP),
            cards_top,
            card_w,
            card_h,
        )
    };
    Layout {
        preview,
        field,
        flash,
        clear,
        animation: card(0.0),
        colour: card(1.0),
        duration: card(2.0),
    }
}

fn inner(card: Rect) -> Rect {
    Rect::from_ltrb(
        card.left + PAD,
        card.top + PAD,
        card.right - PAD,
        card.bottom - PAD,
    )
}

/// The six animation chips, three per row.
fn animation_chips(card: Rect) -> [Rect; 6] {
    let inner = inner(card);
    let w = (inner.width() - CHIP_GAP * 2.0) / 3.0;
    std::array::from_fn(|index| {
        let (col, row) = ((index % 3) as f32, (index / 3) as f32);
        Rect::from_xywh(
            inner.left + col * (w + CHIP_GAP),
            card.top + 32.0 + row * (CHIP_H + CHIP_GAP),
            w,
            CHIP_H,
        )
    })
}

/// The speed label, slider track and value label.
fn speed_row(card: Rect) -> (Rect, Rect, Rect) {
    let inner = inner(card);
    let top = card.top + 32.0 + 2.0 * CHIP_H + CHIP_GAP + 8.0;
    let label = Rect::from_xywh(inner.left, top, 44.0, 18.0);
    let value = Rect::from_xywh(inner.right - 48.0, top, 48.0, 18.0);
    let track = Rect::from_ltrb(label.right + 4.0, top, value.left - 10.0, top + 18.0);
    (label, track, value)
}

/// Seven preset dots and the custom "+" (centres and radius).
fn colour_dots(card: Rect) -> [(Point, f32); 8] {
    let inner = inner(card);
    let step = inner.width() / 8.0;
    let y = card.top + 44.0;
    std::array::from_fn(|index| (Point::new(inner.left + step * (index as f32 + 0.5), y), 8.0))
}

fn colour_divider(card: Rect) -> f32 {
    card.top + 62.0
}

/// The four finish swatches (centres and radius) after the "Finish" label.
fn finish_swatches(card: Rect) -> [(Point, f32); 4] {
    let inner = inner(card);
    let left = inner.left + 46.0;
    let step = (inner.right - left) / 4.0;
    let y = card.top + 81.0;
    std::array::from_fn(|index| (Point::new(left + step * (index as f32 + 0.5), y), 10.0))
}

/// The segmented control and its four segments.
fn duration_segments(card: Rect) -> (Rect, [Rect; 4]) {
    let inner = inner(card);
    let control = Rect::from_xywh(inner.left, card.top + 32.0, inner.width(), 28.0);
    // "Custom" (and "1m 30s ›" once chosen) needs more room than "10s".
    let w = (control.width() - 4.0) / 4.6;
    let segments = std::array::from_fn(|index| {
        Rect::from_xywh(
            control.left + 2.0 + index as f32 * w,
            control.top + 2.0,
            if index == 3 { w * 1.6 } else { w },
            control.height() - 4.0,
        )
    });
    (control, segments)
}

fn repeat_row(card: Rect) -> (Rect, Rect) {
    let inner = inner(card);
    let y = card.top + 91.0;
    let toggle = Rect::from_xywh(inner.right - 36.0, y - 11.0, 36.0, 22.0);
    let label = Rect::from_ltrb(inner.left, y - 10.0, toggle.left - 8.0, y + 10.0);
    (label, toggle)
}

fn circle_hit(center: Point, radius: f32, point: Point) -> bool {
    let (dx, dy) = (point.x - center.x, point.y - center.y);
    dx * dx + dy * dy <= radius * radius
}

// ---------------------------------------------------------------------------------------
// Tab entry points
// ---------------------------------------------------------------------------------------

/// Draws the tab.
pub fn draw(ctx: &TabContext<'_>) -> bool {
    let layout = layout(ctx.grid);
    STATE.with(|state| {
        let state = state.borrow();
        draw_preview(ctx, &layout, &state);
        draw_input_row(ctx, &layout, &state);
        draw_animation_card(ctx, layout.animation, &state);
        draw_colour_card(ctx, layout.colour, &state);
        draw_duration_card(ctx, layout.duration, &state);
    });
    true
}

/// The tab draws its own cards.
pub fn custom_layout() -> bool {
    true
}

/// Hit test in page coordinates.
pub fn hit(grid: Rect, point: Point) -> Option<DashboardHit> {
    let layout = layout(grid);
    let id = |id: u16| Some(DashboardHit::Tab(id));
    if layout.field.contains(point) {
        return id(ID_FIELD);
    }
    if layout.flash.contains(point) {
        return id(ID_FLASH);
    }
    if layout.clear.contains(point) {
        return id(ID_CLEAR);
    }
    if layout.preview.contains(point) {
        return id(ID_PREVIEW);
    }
    if layout.animation.contains(point) {
        if let Some(index) = animation_chips(layout.animation)
            .iter()
            .position(|chip| chip.contains(point))
        {
            return id(ID_ANIMATION + index as u16);
        }
        let (_, track, _) = speed_row(layout.animation);
        if Rect::from_ltrb(
            track.left - 8.0,
            track.top - 4.0,
            track.right + 8.0,
            track.bottom + 4.0,
        )
        .contains(point)
        {
            let fraction = ((point.x - track.left) / track.width()).clamp(0.0, 1.0);
            let step = (fraction * f32::from(SPEED_STEPS)).round() as u16;
            return id(ID_SPEED + step);
        }
        return None;
    }
    if layout.colour.contains(point) {
        if let Some(index) = colour_dots(layout.colour)
            .iter()
            .position(|(center, radius)| circle_hit(*center, radius + 4.0, point))
        {
            return id(ID_COLOR + index as u16);
        }
        if let Some(index) = finish_swatches(layout.colour)
            .iter()
            .position(|(center, radius)| circle_hit(*center, radius + 4.0, point))
        {
            return id(ID_FINISH + index as u16);
        }
        return None;
    }
    if layout.duration.contains(point) {
        let (_, segments) = duration_segments(layout.duration);
        if let Some(index) = segments.iter().position(|segment| segment.contains(point)) {
            return id(ID_DURATION + index as u16);
        }
        let (label, toggle) = repeat_row(layout.duration);
        if label.contains(point) || toggle.inset(-4.0).contains(point) {
            return id(ID_REPEAT);
        }
    }
    None
}

/// Applies a click from [`hit`].
pub fn apply(hit: DashboardHit) -> bool {
    let DashboardHit::Tab(id) = hit else {
        return false;
    };
    if !(1000..1100).contains(&id) {
        return false;
    }
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let state = &mut *state;
        if id != ID_FIELD {
            state.editing = false;
        }
        match id {
            ID_FIELD => {
                state.editing = true;
                state.caret = state.char_count();
                state.caret_moved = Instant::now();
            }
            ID_FLASH => flash(state),
            ID_CLEAR => {
                // Take the live banner down; with none showing, empty the field instead.
                if !banner::clear_flash() {
                    state.text.clear();
                    state.caret = 0;
                    state.editing = true;
                    state.caret_moved = Instant::now();
                }
            }
            ID_PREVIEW => state.restart_preview(),
            _ if (ID_ANIMATION..ID_ANIMATION + 6).contains(&id) => {
                state.animation = Animation::ALL[usize::from(id - ID_ANIMATION)];
                state.restart_preview();
            }
            ID_CUSTOM_COLOR => {
                // Each click steps to a new hue (golden angle keeps them distinct).
                if state.color == ColorChoice::Custom {
                    state.custom_hue = (state.custom_hue + 137.5).rem_euclid(360.0);
                }
                state.color = ColorChoice::Custom;
                state.finish = Finish::Solid;
            }
            _ if (ID_COLOR..ID_COLOR + 7).contains(&id) => {
                state.color = ColorChoice::Preset(usize::from(id - ID_COLOR));
                // Picking a colour means a solid fill in that colour.
                state.finish = Finish::Solid;
            }
            _ if (ID_FINISH..ID_FINISH + 4).contains(&id) => {
                state.finish = Finish::ALL[usize::from(id - ID_FINISH)];
            }
            _ if (ID_DURATION..ID_DURATION + 4).contains(&id) => {
                let index = usize::from(id - ID_DURATION);
                if index == 3 && state.duration == 3 {
                    state.custom_index = (state.custom_index + 1) % CUSTOM_SECONDS.len();
                }
                state.duration = index;
            }
            ID_REPEAT => state.repeat = !state.repeat,
            _ if (ID_SPEED..=ID_SPEED + SPEED_STEPS).contains(&id) => {
                state.speed = f32::from(id - ID_SPEED) / f32::from(SPEED_STEPS);
                state.restart_preview();
            }
            _ => return false,
        }
        true
    })
}

fn flash(state: &mut State) {
    let text = state.text.trim();
    if text.is_empty() {
        return;
    }
    banner::flash(
        text,
        state.style(),
        Duration::from_secs(state.hold_seconds()),
        state.repeat,
    );
    state.editing = false;
    state.restart_preview();
}

pub fn wants_keyboard() -> bool {
    STATE.with(|state| state.borrow().editing)
}

pub fn handle_key(key: &Key) -> bool {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if !state.editing {
            return false;
        }
        let count = state.char_count();
        match key {
            Key::Character(typed) => {
                let room = MAX_CHARS.saturating_sub(count);
                let insert: String = typed
                    .chars()
                    .filter(|character| !character.is_control())
                    .take(room)
                    .collect();
                if !insert.is_empty() {
                    let at = state.byte_at(state.caret);
                    state.text.insert_str(at, &insert);
                    state.caret += insert.chars().count();
                }
            }
            Key::Backspace => {
                if state.caret > 0 {
                    let start = state.byte_at(state.caret - 1);
                    let end = state.byte_at(state.caret);
                    state.text.replace_range(start..end, "");
                    state.caret -= 1;
                }
            }
            Key::Delete => {
                if state.caret < count {
                    let start = state.byte_at(state.caret);
                    let end = state.byte_at(state.caret + 1);
                    state.text.replace_range(start..end, "");
                }
            }
            Key::ArrowLeft => state.caret = state.caret.saturating_sub(1),
            Key::ArrowRight => state.caret = (state.caret + 1).min(count),
            Key::Home | Key::ArrowUp => state.caret = 0,
            Key::End | Key::ArrowDown => state.caret = count,
            Key::Enter => flash(&mut state),
            Key::Escape | Key::Tab => state.editing = false,
            _ => return false,
        }
        state.caret_moved = Instant::now();
        true
    })
}

/// Every frame, visible or not.
pub fn poll(_now: Instant) {
    // Leaving the tab ends editing so keys go back to the rest of the panel.
    if !crate::ui::dashboard::showing(Destination::Message) {
        STATE.with(|state| {
            if let Ok(mut state) = state.try_borrow_mut() {
                state.editing = false;
            }
        });
    }
}

// ---------------------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------------------

fn hovered(ctx: &TabContext<'_>, id: u16) -> bool {
    ctx.hover == Some(DashboardHit::Tab(id))
}

fn card(painter: Painter<'_>, rect: Rect, colors: &SemanticColors) {
    painter.fill_round_rect(
        rect,
        Radius::uniform(radius::CARD + 4.0),
        colors.surface_raised,
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

/// Icon and title at the top left of a card.
fn card_title(
    painter: Painter<'_>,
    card: Rect,
    icon: fn(Painter<'_>, Rect, Rgba),
    title: &str,
    colors: &SemanticColors,
) {
    let icon_rect = Rect::from_xywh(card.left + PAD, card.top + PAD + 1.0, 14.0, 14.0);
    icon(painter, icon_rect, colors.text_secondary);
    draw_text(
        painter,
        title,
        typography::LABEL,
        Rect::from_xywh(icon_rect.right + 6.0, card.top + PAD - 1.0, 140.0, 18.0),
        Align::Start,
        colors.text_secondary,
    );
}

fn draw_preview(ctx: &TabContext<'_>, layout: &Layout, state: &State) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let rect = layout.preview;
    let hover = hovered(ctx, ID_PREVIEW);
    painter.fill_round_rect(
        rect,
        Radius::uniform(radius::CARD + 4.0),
        if hover {
            colors.surface_overlay
        } else {
            colors.surface_raised
        },
    );
    // "Live preview" with a small notch glyph, vertically centred.
    let glyph = Rect::from_xywh(rect.left + PAD + 2.0, rect.center_y() - 4.0, 12.0, 8.0);
    painter.fill_round_rect(glyph, Radius::uniform(2.5), colors.text_secondary);
    draw_text(
        painter,
        "Live preview",
        typography::LABEL,
        Rect::from_xywh(glyph.right + 8.0, rect.center_y() - 9.0, 120.0, 18.0),
        Align::Start,
        colors.text_secondary,
    );

    // Right: what Flash will do, or the live banner's countdown.
    let (note, note_color) = match banner::flash_remaining(ctx.now) {
        Some(Some(left)) => (
            format!("Live in notch \u{00b7} {}s left", left.as_secs() + 1),
            GREEN,
        ),
        Some(None) => ("Live in notch \u{00b7} until cleared".to_string(), GREEN),
        None if state.repeat => ("Shows until cleared".to_string(), colors.text_secondary),
        None => (
            format!("Shows for {}", format_seconds(state.hold_seconds())),
            colors.text_secondary,
        ),
    };
    let note_w = measure(&note, typography::LABEL) + 2.0;
    let note_rect = Rect::from_xywh(
        rect.right - PAD - note_w,
        rect.center_y() - 9.0,
        note_w,
        18.0,
    );
    if note_color == GREEN {
        painter.fill_circle(
            Point::new(note_rect.left - 9.0, rect.center_y()),
            3.0,
            GREEN,
        );
    }
    draw_text(
        painter,
        &note,
        typography::LABEL,
        note_rect,
        Align::Start,
        note_color,
    );

    // The notch pill hanging from the card's top edge, with the banner inside, drawn by
    // the notch widget itself.
    let text = state.text.trim();
    let style = state.style();
    let shown = if text.is_empty() {
        "Your message"
    } else {
        text
    };
    let content_w = banner_width(shown, style.animation) * PREVIEW_SCALE;
    let pill_h = PILL_H * PREVIEW_SCALE;
    let pill_w = content_w + 26.0;
    let pill = Rect::from_xywh(rect.center_x() - pill_w / 2.0, rect.top, pill_w, pill_h);
    let corner = Vec2::new(pill_h * 0.5, pill_h * 0.5);
    let pill_radius = Radius {
        top_left: Vec2::ZERO,
        top_right: Vec2::ZERO,
        bottom_right: corner,
        bottom_left: corner,
    };
    let save = painter.save();
    painter.clip_round_rect(rect, Radius::uniform(radius::CARD + 4.0));
    painter.fill_round_rect(pill, pill_radius, Rgba::from_rgb(4, 4, 6));
    painter.stroke_round_rect(pill, pill_radius, 1.0, Rgba::from_argb(26, 255, 255, 255));
    let content = Rect::from_xywh(
        pill.center_x() - content_w / 2.0,
        pill.top,
        content_w,
        pill_h,
    );
    let elapsed = ctx
        .now
        .saturating_duration_since(state.preview_started)
        .as_secs_f32();
    let (time, exit) = if style.animation.continuous() {
        (elapsed, 1.0)
    } else {
        // Play the entrance, hold, leave, pause, repeat.
        let intro = intro_seconds(shown, &style);
        let period = intro + 2.2;
        let time = elapsed % (period + 0.5);
        let exit = ((period - time) / 0.35).clamp(0.0, 1.0);
        (time, exit)
    };
    let alpha = if text.is_empty() { 110 } else { 255 };
    draw_banner(
        painter,
        content,
        PREVIEW_SCALE,
        alpha,
        shown,
        &style,
        time,
        exit,
    );
    painter.restore_to(save);
}

fn draw_input_row(ctx: &TabContext<'_>, layout: &Layout, state: &State) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let field = layout.field;
    let radius = Radius::uniform(field.height() / 2.0);
    painter.fill_round_rect(
        field,
        radius,
        if state.editing || hovered(ctx, ID_FIELD) {
            colors.surface_overlay
        } else {
            colors.surface_raised
        },
    );
    if state.editing {
        painter.stroke_round_rect(field, radius, 1.5, colors.border_strong);
    }
    // Message glyph.
    let icon = Rect::from_xywh(field.left + 14.0, field.center_y() - 7.0, 16.0, 14.0);
    message_icon(painter, icon, colors.text_secondary);

    // Counter on the right, warning colour near the limit.
    let count = state.char_count();
    let counter = format!("{count}/{MAX_CHARS}");
    let counter_w = measure(&counter, COUNTER) + 2.0;
    let counter_rect = Rect::from_xywh(
        field.right - 16.0 - counter_w,
        field.center_y() - 9.0,
        counter_w,
        18.0,
    );
    draw_text(
        painter,
        &counter,
        COUNTER,
        counter_rect,
        Align::Start,
        if count >= MAX_CHARS {
            colors.state_warning
        } else {
            colors.text_secondary
        },
    );

    let text_rect = Rect::from_ltrb(
        icon.right + 10.0,
        field.center_y() - 10.0,
        counter_rect.left - 12.0,
        field.center_y() + 10.0,
    );
    let fonts = FontManager::global();
    let font = FontStyle::normal();
    if state.text.is_empty() {
        draw_text(
            painter,
            "Type a message for the notch\u{2026}",
            FIELD,
            text_rect,
            Align::Start,
            colors.text_disabled,
        );
    }
    // Keep the caret in view: scroll the text left when it runs past the field.
    let caret_x_full =
        fonts.measure_text_cached(&state.text[..state.byte_at(state.caret)], FIELD.size, font);
    let shift = if state.editing {
        (caret_x_full - text_rect.width() + 2.0).max(0.0)
    } else {
        0.0
    };
    if !state.text.is_empty() {
        let save = painter.save();
        painter.clip_rect(Rect::from_ltrb(
            text_rect.left - 1.0,
            field.top,
            text_rect.right + 2.0,
            field.bottom,
        ));
        if shift > 0.0 {
            fonts.draw_str(
                painter,
                &state.text,
                Point::new(
                    text_rect.left - shift,
                    text_rect.center_y() + FIELD.size * 0.35,
                ),
                FIELD.size,
                false,
                colors.text_primary,
            );
        } else {
            draw_text(
                painter,
                &state.text,
                FIELD,
                text_rect,
                Align::Start,
                colors.text_primary,
            );
        }
        painter.restore_to(save);
    }
    if state.editing {
        let since = ctx
            .now
            .saturating_duration_since(state.caret_moved)
            .as_secs_f32();
        if since < 0.5 || (since * 1.8).fract() < 0.55 {
            let x = text_rect.left + caret_x_full - shift + 1.0;
            painter.fill_rect(
                Rect::from_xywh(x, field.center_y() - 9.0, 1.5, 18.0),
                colors.text_primary,
            );
        }
    }

    // Flash: a white pill with a bolt.
    let flash = layout.flash;
    let enabled = !state.text.trim().is_empty();
    let flash_fill = if !enabled {
        Rgba::from_argb(90, 255, 255, 255)
    } else if hovered(ctx, ID_FLASH) {
        Rgba::from_rgb(222, 224, 228)
    } else {
        Rgba::WHITE
    };
    painter.fill_round_rect(flash, Radius::uniform(flash.height() / 2.0), flash_fill);
    let ink = Rgba::from_rgb(16, 17, 20);
    let label_w = measure("Flash", BUTTON);
    let group_w = 12.0 + 7.0 + label_w;
    let start = flash.center_x() - group_w / 2.0;
    bolt_icon(
        painter,
        Rect::from_xywh(start, flash.center_y() - 8.0, 12.0, 16.0),
        ink,
    );
    draw_text(
        painter,
        "Flash",
        BUTTON,
        Rect::from_xywh(start + 19.0, flash.top, label_w + 2.0, flash.height()),
        Align::Start,
        ink,
    );

    // Clear: a dark pill.
    let clear = layout.clear;
    painter.fill_round_rect(
        clear,
        Radius::uniform(clear.height() / 2.0),
        if hovered(ctx, ID_CLEAR) {
            colors.surface_interactive
        } else {
            colors.surface_raised
        },
    );
    draw_text(
        painter,
        "Clear",
        BUTTON,
        clear,
        Align::Center,
        colors.text_primary,
    );
}

/// A pill chip; selected chips get a bright outline.
fn chip(
    painter: Painter<'_>,
    rect: Rect,
    label: &str,
    selected: bool,
    hover: bool,
    colors: &SemanticColors,
) {
    let radius = Radius::uniform(rect.height() / 2.0);
    painter.fill_round_rect(
        rect,
        radius,
        if selected {
            colors.surface_interactive
        } else if hover {
            colors.surface_interactive.with_alpha(200)
        } else {
            colors.surface_overlay
        },
    );
    if selected {
        painter.stroke_round_rect(rect.inset(0.75), radius, 1.5, colors.text_primary);
    }
    draw_text(
        painter,
        label,
        if selected { CHIP_ON } else { CHIP },
        rect,
        Align::Center,
        if selected || hover {
            colors.text_primary
        } else {
            colors.text_secondary
        },
    );
}

fn draw_animation_card(ctx: &TabContext<'_>, rect: Rect, state: &State) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    card(painter, rect, colors);
    card_title(painter, rect, play_icon, "Animation", colors);
    for (index, chip_rect) in animation_chips(rect).into_iter().enumerate() {
        let animation = Animation::ALL[index];
        chip(
            painter,
            chip_rect,
            animation.label(),
            state.animation == animation,
            hovered(ctx, ID_ANIMATION + index as u16),
            colors,
        );
    }
    let (label, track, value) = speed_row(rect);
    draw_text(
        painter,
        "Speed",
        typography::LABEL,
        label,
        Align::Start,
        colors.text_secondary,
    );
    let hover = matches!(ctx.hover, Some(DashboardHit::Tab(id)) if (ID_SPEED..=ID_SPEED + SPEED_STEPS).contains(&id));
    let bar = Rect::from_xywh(track.left, track.center_y() - 2.0, track.width(), 4.0);
    painter.fill_round_rect(bar, Radius::uniform(2.0), colors.surface_overlay);
    let fill = bar.width() * state.speed.clamp(0.0, 1.0);
    painter.fill_round_rect(
        Rect::from_xywh(bar.left, bar.top, fill.max(bar.height()), bar.height()),
        Radius::uniform(2.0),
        colors.text_primary,
    );
    // Tick marks for slow / normal / fast.
    for tick in [0.0, 0.5, 1.0] {
        let x = bar.left + bar.width() * tick;
        if (x - (bar.left + fill)).abs() > 8.0 {
            painter.fill_circle(
                Point::new(x, bar.center_y()),
                1.5,
                colors.text_secondary.with_alpha(160),
            );
        }
    }
    let knob = Point::new(bar.left + fill, bar.center_y());
    if hover {
        painter.fill_circle(knob, 11.0, Rgba::from_argb(36, 255, 255, 255));
    }
    painter.fill_circle(knob, 7.0, Rgba::WHITE);
    let speed = speed_label(state.speed);
    let speed_w = measure(speed, typography::LABEL) + 2.0;
    draw_text(
        painter,
        speed,
        typography::LABEL,
        Rect::from_xywh(value.right - speed_w, value.top, speed_w, value.height()),
        Align::Start,
        colors.text_primary,
    );
}

fn draw_colour_card(ctx: &TabContext<'_>, rect: Rect, state: &State) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    card(painter, rect, colors);
    card_title(painter, rect, brush_icon, "Colour", colors);
    let solid = state.finish == Finish::Solid;
    for (index, (center, radius)) in colour_dots(rect).into_iter().enumerate() {
        let id = ID_COLOR + index as u16;
        let hover = hovered(ctx, id);
        let selected = if index < PRESETS.len() {
            state.color == ColorChoice::Preset(index)
        } else {
            state.color == ColorChoice::Custom
        };
        if selected {
            painter.stroke_circle(
                center,
                radius + 3.5,
                1.5,
                if solid {
                    colors.text_primary
                } else {
                    colors.text_secondary.with_alpha(140)
                },
            );
        } else if hover {
            painter.fill_circle(center, radius + 4.0, colors.surface_interactive);
        }
        if index < PRESETS.len() {
            painter.fill_circle(center, radius, PRESETS[index]);
        } else if state.color == ColorChoice::Custom {
            // The custom hue, with a small "+" to step to another.
            painter.fill_circle(center, radius, hue_color(state.custom_hue));
            plus(painter, center, 3.5, Rgba::from_argb(200, 20, 20, 24));
        } else {
            painter.stroke_circle(center, radius - 0.5, 1.0, colors.border_strong);
            plus(
                painter,
                center,
                4.0,
                if hover {
                    colors.text_primary
                } else {
                    colors.text_secondary
                },
            );
        }
    }
    let inner = inner(rect);
    let divider = colour_divider(rect);
    painter.fill_rect(
        Rect::from_xywh(inner.left, divider, inner.width(), 1.0),
        colors.border_subtle,
    );
    let swatches = finish_swatches(rect);
    draw_text(
        painter,
        "Finish",
        typography::LABEL,
        Rect::from_xywh(inner.left, swatches[0].0.y - 9.0, 44.0, 18.0),
        Align::Start,
        colors.text_secondary,
    );
    for (index, (center, radius)) in swatches.into_iter().enumerate() {
        let finish = Finish::ALL[index];
        let selected = state.finish == finish;
        let hover = hovered(ctx, ID_FINISH + index as u16);
        if selected {
            painter.stroke_circle(center, radius + 3.5, 1.5, colors.text_primary);
        } else if hover {
            painter.fill_circle(center, radius + 4.0, colors.surface_interactive);
        }
        let bounds = Rect::from_xywh(
            center.x - radius,
            center.y - radius,
            radius * 2.0,
            radius * 2.0,
        );
        match finish.stops() {
            None => {
                // Solid dark, with the chosen colour as a small dot.
                painter.fill_circle(center, radius, Rgba::from_rgb(28, 29, 33));
                painter.stroke_circle(center, radius - 0.5, 1.0, colors.border_strong);
                painter.fill_circle(center, 3.0, state.color());
            }
            Some(stops) => {
                let gradient: Vec<GradientStop> = stops
                    .iter()
                    .take(3)
                    .enumerate()
                    .map(|(at, color)| GradientStop {
                        offset: at as f32 / 2.0,
                        color: *color,
                    })
                    .collect();
                painter.fill_round_rect_with_gradient(
                    bounds,
                    Radius::uniform(radius),
                    Point::new(bounds.left, bounds.top),
                    Point::new(bounds.right, bounds.bottom),
                    &gradient,
                    TileMode::Clamp,
                );
            }
        }
        if selected {
            let label = finish.label();
            let w = measure(label, typography::CAPTION) + 2.0;
            draw_text(
                painter,
                label,
                typography::CAPTION,
                Rect::from_xywh(center.x - w / 2.0, center.y + radius + 5.0, w, 14.0),
                Align::Start,
                colors.text_primary,
            );
        }
    }
}

fn draw_duration_card(ctx: &TabContext<'_>, rect: Rect, state: &State) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    card(painter, rect, colors);
    card_title(painter, rect, clock_icon, "Duration", colors);
    let (control, segments) = duration_segments(rect);
    painter.fill_round_rect(
        control,
        Radius::uniform(control.height() / 2.0),
        colors.surface_overlay,
    );
    for (index, segment) in segments.into_iter().enumerate() {
        let selected = state.duration == index && !state.repeat;
        let chosen = state.duration == index;
        let hover = hovered(ctx, ID_DURATION + index as u16);
        let radius = Radius::uniform(segment.height() / 2.0);
        if chosen {
            painter.fill_round_rect(segment, radius, colors.surface_interactive);
            painter.stroke_round_rect(
                segment.inset(0.75),
                radius,
                1.5,
                if selected {
                    colors.text_primary
                } else {
                    colors.text_secondary.with_alpha(140)
                },
            );
        } else if hover {
            painter.fill_round_rect(segment, radius, colors.surface_interactive.with_alpha(170));
        }
        let label = if index < DURATIONS.len() {
            format!("{}s", DURATIONS[index])
        } else if chosen {
            format!(
                "{} \u{203a}",
                format_seconds(CUSTOM_SECONDS[state.custom_index % CUSTOM_SECONDS.len()])
            )
        } else {
            "Custom".to_string()
        };
        draw_text(
            painter,
            &label,
            if chosen { CHIP_ON } else { CHIP },
            segment,
            Align::Center,
            if chosen || hover {
                colors.text_primary
            } else {
                colors.text_secondary
            },
        );
    }
    let inner = inner(rect);
    painter.fill_rect(
        Rect::from_xywh(inner.left, card_divider(rect), inner.width(), 1.0),
        colors.border_subtle,
    );
    let (label, toggle) = repeat_row(rect);
    let hover = hovered(ctx, ID_REPEAT);
    draw_text(
        painter,
        "Keep until cleared",
        typography::BODY,
        label,
        Align::Start,
        if hover {
            colors.text_primary
        } else {
            colors.text_primary.with_alpha(225)
        },
    );
    let radius = Radius::uniform(toggle.height() / 2.0);
    painter.fill_round_rect(
        toggle,
        radius,
        if state.repeat {
            GREEN
        } else if hover {
            colors.surface_interactive
        } else {
            colors.surface_overlay
        },
    );
    let knob_r = toggle.height() / 2.0 - 2.5;
    let knob_x = if state.repeat {
        toggle.right - toggle.height() / 2.0
    } else {
        toggle.left + toggle.height() / 2.0
    };
    painter.fill_circle(
        Point::new(knob_x, toggle.center_y()),
        knob_r,
        if state.repeat {
            Rgba::WHITE
        } else {
            colors.text_secondary
        },
    );
}

fn card_divider(card: Rect) -> f32 {
    card.top + 70.0
}

// ---------------------------------------------------------------------------------------
// Icons (stroked paths, sized to their rect)
// ---------------------------------------------------------------------------------------

fn plus(painter: Painter<'_>, center: Point, arm: f32, color: Rgba) {
    painter.stroke_line(
        Point::new(center.x - arm, center.y),
        Point::new(center.x + arm, center.y),
        1.5,
        color,
        StrokeCap::Round,
    );
    painter.stroke_line(
        Point::new(center.x, center.y - arm),
        Point::new(center.x, center.y + arm),
        1.5,
        color,
        StrokeCap::Round,
    );
}

fn play_icon(painter: Painter<'_>, rect: Rect, color: Rgba) {
    let mut path = PathBuilder::new();
    path.move_to(Point::new(rect.left + 2.5, rect.top + 1.5))
        .line_to(Point::new(rect.right - 1.5, rect.center_y()))
        .line_to(Point::new(rect.left + 2.5, rect.bottom - 1.5))
        .close();
    painter.fill_path(&path.detach(), color);
}

fn brush_icon(painter: Painter<'_>, rect: Rect, color: Rgba) {
    // A paint tube: rounded head, neck and a bristle tip.
    painter.fill_round_rect(
        Rect::from_xywh(rect.left + 3.0, rect.top + 1.0, rect.width() - 6.0, 7.0),
        Radius::uniform(1.5),
        color,
    );
    painter.fill_rect(
        Rect::from_xywh(rect.center_x() - 1.0, rect.top + 8.0, 2.0, 2.5),
        color,
    );
    painter.fill_round_rect(
        Rect::from_xywh(rect.center_x() - 2.0, rect.top + 10.0, 4.0, 3.5),
        Radius::uniform(1.0),
        color,
    );
}

fn clock_icon(painter: Painter<'_>, rect: Rect, color: Rgba) {
    let center = Point::new(rect.center_x(), rect.center_y());
    painter.fill_circle(center, rect.width() / 2.0 - 0.5, color);
    let ink = Rgba::from_rgb(30, 33, 40);
    painter.stroke_line(
        center,
        Point::new(center.x, center.y - 3.5),
        1.5,
        ink,
        StrokeCap::Round,
    );
    painter.stroke_line(
        center,
        Point::new(center.x + 2.5, center.y + 1.0),
        1.5,
        ink,
        StrokeCap::Round,
    );
}

fn message_icon(painter: Painter<'_>, rect: Rect, color: Rgba) {
    painter.stroke_round_rect(rect.inset(0.75), Radius::uniform(3.0), 1.5, color);
    for (row, width) in [(0.38, 0.55), (0.64, 0.35)] {
        let y = rect.top + rect.height() * row;
        painter.stroke_line(
            Point::new(rect.left + 4.0, y),
            Point::new(rect.left + 4.0 + (rect.width() - 8.0) * width / 0.55, y),
            1.5,
            color,
            StrokeCap::Round,
        );
    }
}

fn bolt_icon(painter: Painter<'_>, rect: Rect, color: Rgba) {
    let at =
        |x: f32, y: f32| Point::new(rect.left + rect.width() * x, rect.top + rect.height() * y);
    let mut path = PathBuilder::new();
    path.move_to(at(0.62, 0.0))
        .line_to(at(0.05, 0.58))
        .line_to(at(0.48, 0.58))
        .line_to(at(0.38, 1.0))
        .line_to(at(0.95, 0.42))
        .line_to(at(0.52, 0.42))
        .close();
    painter.stroke_path(
        &path.detach(),
        1.5,
        color,
        StrokeCap::Round,
        StrokeJoin::Round,
    );
}
