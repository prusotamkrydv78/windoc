//! The status-message widget in the collapsed notch.
//!
//! It shows the configured status message, unless the Message tab has *flashed* a banner:
//! a flash wins over the configured text until it expires or is taken down, and is drawn
//! with its own animation (slide, fade, type-on, scroll, blur, pulse), speed, colour and
//! finish (solid, silver, ocean, aurora). [`draw_banner`] is shared with the tab's live
//! preview so the preview matches the notch exactly.

use std::cell::RefCell;
use std::f32::consts::TAU;
use std::time::{Duration, Instant};

use winisland_render::text::{DrawTextCachedParams, FontManager};
use winisland_render::{BlurSpec, FontStyle, Painter, Radius, Rect, Rgba};

const SIZE: f32 = 11.0;
const PADDING: f32 = 6.0;
const MIN_WIDTH: f32 = 40.0;
const MAX_WIDTH: f32 = 220.0;
/// Marquee banners keep a fixed window and scroll through it.
const SCROLL_WIDTH: f32 = 150.0;
/// Seconds the banner takes to fade away when its time is up.
const EXIT: f32 = 0.35;
/// Pixels per second (at 1x scale and normal speed) for the marquee.
const SCROLL_RATE: f32 = 34.0;
/// Gap between marquee repeats, in pixels at 1x scale.
const SCROLL_GAP: f32 = 36.0;

const SILVER: [Rgba; 4] = [
    Rgba::from_rgb(250, 251, 253),
    Rgba::from_rgb(160, 168, 180),
    Rgba::from_rgb(226, 230, 236),
    Rgba::from_rgb(138, 146, 158),
];
const OCEAN: [Rgba; 4] = [
    Rgba::from_rgb(94, 231, 223),
    Rgba::from_rgb(56, 152, 255),
    Rgba::from_rgb(42, 98, 230),
    Rgba::from_rgb(72, 196, 250),
];
const AURORA: [Rgba; 4] = [
    Rgba::from_rgb(110, 240, 170),
    Rgba::from_rgb(80, 196, 248),
    Rgba::from_rgb(150, 128, 250),
    Rgba::from_rgb(236, 150, 250),
];

/// How the banner arrives (or, for scroll and pulse, keeps moving).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Animation {
    #[default]
    Slide,
    Fade,
    TypeOn,
    Scroll,
    Blur,
    Pulse,
}

impl Animation {
    pub(crate) const ALL: [Animation; 6] = [
        Animation::Slide,
        Animation::Fade,
        Animation::TypeOn,
        Animation::Scroll,
        Animation::Blur,
        Animation::Pulse,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Animation::Slide => "Slide",
            Animation::Fade => "Fade",
            Animation::TypeOn => "Type-on",
            Animation::Scroll => "Scroll",
            Animation::Blur => "Blur",
            Animation::Pulse => "Pulse",
        }
    }

    /// Scroll and pulse never settle, so their preview does not loop.
    pub(crate) const fn continuous(self) -> bool {
        matches!(self, Animation::Scroll | Animation::Pulse)
    }
}

/// How the text is filled.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Finish {
    /// The chosen colour.
    #[default]
    Solid,
    Silver,
    Ocean,
    /// A drifting green, blue and violet gradient.
    Aurora,
}

impl Finish {
    pub(crate) const ALL: [Finish; 4] =
        [Finish::Solid, Finish::Silver, Finish::Ocean, Finish::Aurora];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Finish::Solid => "Solid",
            Finish::Silver => "Silver",
            Finish::Ocean => "Ocean",
            Finish::Aurora => "Aurora",
        }
    }

    /// Gradient stops for the finish, `None` for a solid colour. Stops wrap around (the
    /// last colour blends back into the first) so the aurora can drift seamlessly.
    pub(crate) const fn stops(self) -> Option<&'static [Rgba]> {
        match self {
            Finish::Solid => None,
            Finish::Silver => Some(&SILVER),
            Finish::Ocean => Some(&OCEAN),
            Finish::Aurora => Some(&AURORA),
        }
    }

    /// Whether the gradient drifts over time.
    const fn drifts(self) -> bool {
        matches!(self, Finish::Aurora | Finish::Ocean)
    }
}

/// Everything that decides how a banner looks.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct BannerStyle {
    pub animation: Animation,
    pub finish: Finish,
    /// Text colour for the solid finish.
    pub color: Rgba,
    /// Speed multiplier: 1.0 is normal, 0.5 slow, 2.0 fast.
    pub speed: f32,
}

impl Default for BannerStyle {
    fn default() -> Self {
        Self {
            animation: Animation::Slide,
            finish: Finish::Solid,
            color: Rgba::WHITE,
            speed: 1.0,
        }
    }
}

impl BannerStyle {
    fn speed(&self) -> f32 {
        self.speed.clamp(0.25, 4.0)
    }
}

/// A banner flashed from the Message tab.
struct Flash {
    text: String,
    style: BannerStyle,
    started: Instant,
    /// How long the banner shows (one cycle when repeating).
    hold: Duration,
    /// Replay every `hold` until taken down instead of expiring.
    repeat: bool,
}

impl Flash {
    fn expired(&self, now: Instant) -> bool {
        !self.repeat && now.saturating_duration_since(self.started) >= self.hold
    }

    /// Seconds into the current cycle and the cycle length in seconds.
    fn cycle(&self, now: Instant) -> (f32, f32) {
        let elapsed = now.saturating_duration_since(self.started).as_secs_f32();
        let hold = self.hold.as_secs_f32().max(0.5);
        if self.repeat {
            (elapsed % hold, hold)
        } else {
            (elapsed.min(hold), hold)
        }
    }
}

thread_local! {
    static MESSAGE: RefCell<String> = const { RefCell::new(String::new()) };
    static FLASH: RefCell<Option<Flash>> = const { RefCell::new(None) };
}

/// Publishes the configured status message.
pub(crate) fn set(text: &str) {
    MESSAGE.with(|cell| {
        let mut current = cell.borrow_mut();
        if current.as_str() != text {
            *current = text.to_string();
        }
    });
}

/// Shows `text` in the notch with `style` for `hold`, over the configured message. With
/// `repeat` the banner replays every `hold` until [`clear_flash`].
pub(crate) fn flash(text: &str, style: BannerStyle, hold: Duration, repeat: bool) {
    FLASH.with(|cell| {
        *cell.borrow_mut() = Some(Flash {
            text: text.to_string(),
            style,
            started: Instant::now(),
            hold: hold.max(Duration::from_millis(500)),
            repeat,
        });
    });
}

/// Takes the flashed banner down. `true` when one was showing.
pub(crate) fn clear_flash() -> bool {
    FLASH.with(|cell| cell.borrow_mut().take().is_some())
}

/// The live flash: `None` when nothing is flashed, `Some(None)` when it repeats until
/// taken down, otherwise the time left.
pub(crate) fn flash_remaining(now: Instant) -> Option<Option<Duration>> {
    with_live_flash(now, |flash| {
        if flash.repeat {
            None
        } else {
            Some(
                flash
                    .hold
                    .saturating_sub(now.saturating_duration_since(flash.started)),
            )
        }
    })
}

/// How often the notch should redraw while a flash is up (to animate it and to take it
/// down on time). Meant for `compact::next_refresh_delay`.
pub(crate) fn frame_interval() -> Option<Duration> {
    with_live_flash(Instant::now(), |_| Duration::from_millis(16))
}

/// Runs `f` on the flash if one is live, dropping it once it has expired.
fn with_live_flash<T>(now: Instant, f: impl FnOnce(&Flash) -> T) -> Option<T> {
    FLASH.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.as_ref().is_some_and(|flash| flash.expired(now)) {
            *slot = None;
        }
        slot.as_ref().map(f)
    })
}

/// Width at 1x scale for a banner showing `text` with `animation`.
pub(crate) fn banner_width(text: &str, animation: Animation) -> f32 {
    let measured = FontManager::global().measure_text_cached(text, SIZE, FontStyle::bold());
    let max = if animation == Animation::Scroll {
        SCROLL_WIDTH
    } else {
        MAX_WIDTH
    };
    (measured + PADDING * 2.0).clamp(MIN_WIDTH, max)
}

/// Seconds the entrance takes (the whole text typed, for type-on).
pub(crate) fn intro_seconds(text: &str, style: &BannerStyle) -> f32 {
    let speed = style.speed();
    match style.animation {
        Animation::TypeOn => text.chars().count() as f32 * type_step(speed) + 0.2,
        Animation::Blur => 0.75 / speed,
        Animation::Fade => 0.7 / speed,
        _ => 0.55 / speed,
    }
}

/// Width at 1x scale, sized to the message within limits.
pub(super) fn width() -> f32 {
    if let Some(width) = with_live_flash(Instant::now(), |flash| {
        banner_width(&flash.text, flash.style.animation)
    }) {
        return width;
    }
    MESSAGE.with(|cell| {
        let text = cell.borrow();
        let measured = FontManager::global().measure_text_cached(&text, SIZE, FontStyle::bold());
        (measured + PADDING * 2.0).clamp(MIN_WIDTH, MAX_WIDTH)
    })
}

pub(super) fn draw(painter: Painter<'_>, rect: Rect, scale: f32, alpha: u8) {
    let now = Instant::now();
    let flashed = with_live_flash(now, |flash| {
        let (time, hold) = flash.cycle(now);
        // Fade out at the end of each cycle (the last one, without repeat).
        let exit = ((hold - time) / EXIT).clamp(0.0, 1.0);
        draw_banner(
            painter,
            rect,
            scale,
            alpha,
            &flash.text,
            &flash.style,
            time,
            exit,
        );
    });
    if flashed.is_some() {
        return;
    }
    MESSAGE.with(|cell| {
        let text = cell.borrow();
        let size = SIZE * scale;
        let fonts = FontManager::global();
        let shown = crate::utils::settings_ui::ellipsize_text(
            fonts,
            &text,
            size,
            FontStyle::bold(),
            rect.width() - PADDING * scale,
        );
        let width = fonts.measure_text_cached(&shown, size, FontStyle::bold());
        fonts.draw_text_cached(DrawTextCachedParams {
            painter,
            text: &shown,
            x: rect.center_x() - width / 2.0,
            y: rect.center_y() + size * 0.36,
            size,
            bold: true,
            color: Rgba::from_argb(alpha, 255, 255, 255),
            blur: None,
        });
    });
}

/// Draws a banner `time` seconds after it appeared. `exit` is 1 while showing and falls
/// to 0 as it leaves. Used by the notch and by the Message tab's live preview.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_banner(
    painter: Painter<'_>,
    rect: Rect,
    scale: f32,
    alpha: u8,
    text: &str,
    style: &BannerStyle,
    time: f32,
    exit: f32,
) {
    if text.is_empty() || rect.width() <= 0.0 {
        return;
    }
    let size = SIZE * scale;
    let fonts = FontManager::global();
    let font_style = FontStyle::bold();
    let speed = style.speed();
    let time = time.max(0.0);
    let baseline = rect.center_y() + size * 0.36;
    let mut opacity = f32::from(alpha) / 255.0 * exit.clamp(0.0, 1.0);
    let save = painter.save();
    painter.clip_rect(rect);

    if style.animation == Animation::Scroll {
        let width = fonts.measure_text_cached(text, size, font_style);
        let period = width + SCROLL_GAP * scale;
        let offset = (time * SCROLL_RATE * scale * speed) % period;
        // Start mid-window so the text is already arriving on the first frame.
        let start = rect.left + PADDING * scale + (rect.width() * 0.5) - offset;
        let mut first = start;
        while first > rect.left {
            first -= period;
        }
        opacity *= ease_out(time / (0.3 / speed));
        let mut x = first;
        while x < rect.right {
            if x + width > rect.left {
                draw_run(
                    painter,
                    text,
                    usize::MAX,
                    x,
                    baseline,
                    size,
                    style,
                    opacity,
                    None,
                    time,
                );
            }
            x += period;
        }
        painter.restore_to(save);
        return;
    }

    let shown = crate::utils::settings_ui::ellipsize_text(
        fonts,
        text,
        size,
        font_style,
        rect.width() - PADDING * scale,
    );
    let width = fonts.measure_text_cached(&shown, size, font_style);
    let x = rect.center_x() - width / 2.0;
    let intro = intro_seconds(&shown, style);
    let progress = (time / intro).clamp(0.0, 1.0);
    let mut y = baseline;
    let mut visible = usize::MAX;
    let mut blur = None;
    match style.animation {
        Animation::Slide => {
            let eased = ease_out_back(progress);
            y += (1.0 - eased) * rect.height() * 0.85;
            opacity *= ease_out(progress * 1.6);
        }
        Animation::Fade => opacity *= ease_in_out(progress),
        Animation::TypeOn => {
            let count = shown.chars().count();
            visible = ((time / type_step(speed)).floor() as usize + 1).min(count);
            // A caret while typing and for a moment after, blinking once done.
            let typing = visible < count;
            let blink = (time * 2.2).fract() < 0.55;
            if typing || (time < intro + 1.2 && blink) {
                let typed_end = shown
                    .char_indices()
                    .nth(visible)
                    .map_or(shown.len(), |(index, _)| index);
                let typed = fonts.measure_text_cached(&shown[..typed_end], size, font_style);
                let caret = Rect::from_xywh(
                    x + typed + 1.0 * scale,
                    rect.center_y() - size * 0.55,
                    1.5 * scale,
                    size * 1.1,
                );
                painter.fill_rect(caret, Rgba::WHITE.with_alpha_f(opacity * 0.85));
            }
        }
        Animation::Blur => {
            let eased = ease_out(progress);
            let sigma = (1.0 - eased) * 7.0 * scale;
            if sigma > 0.05 {
                blur = Some(BlurSpec::uniform(sigma));
            }
            opacity *= ease_out(progress * 1.4);
        }
        Animation::Pulse => {
            let wave = 0.5 + 0.5 * (time * TAU * speed / 1.6).cos();
            opacity *= ease_out(time / (0.3 / speed)) * (0.5 + 0.5 * wave);
            // A soft glow that breathes with the text.
            let glow = glow_color(style, time).with_alpha_f(0.28 * wave * opacity);
            painter.fill_round_rect(
                Rect::from_xywh(
                    x - 4.0 * scale,
                    rect.top + 2.0 * scale,
                    width + 8.0 * scale,
                    rect.height() - 4.0 * scale,
                ),
                Radius::uniform(rect.height() / 2.0),
                glow,
            );
        }
        Animation::Scroll => {}
    }
    draw_run(
        painter, &shown, visible, x, y, size, style, opacity, blur, time,
    );
    painter.restore_to(save);
}

/// Draws the first `visible` characters of `text` starting at (`x`, `baseline`), filled
/// with the style's colour or gradient.
#[allow(clippy::too_many_arguments)]
fn draw_run(
    painter: Painter<'_>,
    text: &str,
    visible: usize,
    x: f32,
    baseline: f32,
    size: f32,
    style: &BannerStyle,
    opacity: f32,
    blur: Option<BlurSpec>,
    time: f32,
) {
    if opacity <= 0.003 {
        return;
    }
    let fonts = FontManager::global();
    let font_style = FontStyle::bold();
    let end = text
        .char_indices()
        .nth(visible)
        .map_or(text.len(), |(index, _)| index);
    let Some(stops) = style.finish.stops() else {
        fonts.draw_text_cached(DrawTextCachedParams {
            painter,
            text: &text[..end],
            x,
            y: baseline,
            size,
            bold: true,
            color: style
                .color
                .with_alpha_f(f32::from(style.color.a()) / 255.0 * opacity),
            blur,
        });
        return;
    };
    // Gradient finishes colour each character by its position across the whole text.
    let total = fonts.measure_text_cached(text, size, font_style).max(1.0);
    let drift = if style.finish.drifts() {
        time * 0.18 * style.speed()
    } else {
        0.0
    };
    let mut indices = text.char_indices().peekable();
    while let Some((start, _)) = indices.next() {
        if start >= end {
            break;
        }
        let stop = indices.peek().map_or(text.len(), |(index, _)| *index);
        let (left, right) = fonts.measure_text_prefixes_cached(text, start, stop, size, font_style);
        let position = ((left + right) / 2.0) / total;
        let color = sample(stops, position * 0.75 + drift);
        fonts.draw_text_cached(DrawTextCachedParams {
            painter,
            text: &text[start..stop],
            x: x + left,
            y: baseline,
            size,
            bold: true,
            color: color.with_alpha_f(opacity),
            blur,
        });
    }
}

/// Samples wrapping gradient `stops` at `t` (any real; wraps to 0..1).
pub(crate) fn sample(stops: &[Rgba], t: f32) -> Rgba {
    if stops.is_empty() {
        return Rgba::WHITE;
    }
    let count = stops.len();
    let t = t.rem_euclid(1.0) * count as f32;
    let index = (t.floor() as usize).min(count - 1);
    let fraction = t - index as f32;
    mix(stops[index], stops[(index + 1) % count], fraction)
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let lerp = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Rgba::from_argb(
        lerp(a.a(), b.a()),
        lerp(a.r(), b.r()),
        lerp(a.g(), b.g()),
        lerp(a.b(), b.b()),
    )
}

fn glow_color(style: &BannerStyle, time: f32) -> Rgba {
    match style.finish.stops() {
        Some(stops) => sample(stops, time * 0.18),
        None => style.color,
    }
}

/// Seconds per character for type-on.
fn type_step(speed: f32) -> f32 {
    0.055 / speed
}

fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Ease out with a small overshoot, for the slide.
fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.2;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}
