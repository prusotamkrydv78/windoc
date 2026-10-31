use std::sync::OnceLock;
use std::time::{Duration, Instant};

use winisland_render::text::{DrawTextCachedParams, FontManager};
use winisland_render::{Painter, Point, Radius, Rect, Rgba, StrokeCap};

/// A sleeping robot for the idle island: a cream body with a black screen for a face, its
/// eyes shut, bobbing gently with each breath while a "z Z" drifts up.
pub(super) const WIDTH: f32 = 46.0;
/// About 15 frames a second keeps the drift smooth without busy redraws.
pub(super) const FRAME: Duration = Duration::from_millis(66);
const BREATH_SECONDS: f32 = 3.2;
const DRIFT_SECONDS: f32 = 2.4;

const SHELL: (u8, u8, u8) = (242, 238, 228);
const SCREEN: (u8, u8, u8) = (14, 14, 16);
const FEET: (u8, u8, u8) = (178, 178, 182);

fn seconds() -> f32 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

fn color(alpha: u8, opacity: f32, (r, g, b): (u8, u8, u8)) -> Rgba {
    Rgba::from_argb((f32::from(alpha) * opacity.clamp(0.0, 1.0)) as u8, r, g, b)
}

pub(super) fn draw(painter: Painter<'_>, rect: Rect, scale: f32, alpha: u8) {
    let time = seconds();
    let unit = (rect.height() / 32.0).clamp(0.6 * scale, 1.4 * scale);
    let breath = (time / BREATH_SECONDS * std::f32::consts::TAU).sin();
    let center_x = rect.left + 14.0 * unit;
    // Feet rest near the bottom of the notch; the rest of the body rises with each breath.
    let floor = rect.center_y() + 11.0 * unit;
    let lift = (breath * 0.5 + 0.5) * 0.8 * unit;

    for side in [-1.0, 1.0] {
        painter.fill_oval(
            Rect::from_xywh(
                center_x + side * 3.0 * unit - 2.2 * unit,
                floor - 2.4 * unit,
                4.4 * unit,
                2.4 * unit,
            ),
            color(alpha, 1.0, FEET),
        );
    }

    let body = Rect::from_xywh(
        center_x - 4.6 * unit,
        floor - 8.2 * unit - lift * 0.5,
        9.2 * unit,
        6.4 * unit,
    );
    painter.fill_round_rect(body, Radius::uniform(2.4 * unit), color(alpha, 1.0, SHELL));

    // Head: a cream shell around a black screen, with a thicker chin like a small monitor.
    let head = Rect::from_xywh(
        center_x - 8.0 * unit,
        body.top - 13.0 * unit - lift * 0.5,
        16.0 * unit,
        13.6 * unit,
    );
    painter.fill_round_rect(head, Radius::uniform(3.8 * unit), color(alpha, 1.0, SHELL));
    let screen = Rect::from_xywh(
        head.left + 2.0 * unit,
        head.top + 2.0 * unit,
        head.width() - 4.0 * unit,
        head.height() - 5.2 * unit,
    );
    painter.fill_round_rect(
        screen,
        Radius::uniform(2.4 * unit),
        color(alpha, 1.0, SCREEN),
    );

    // Closed eyes: short white strokes on the screen.
    let eye_y = screen.center_y() + 0.6 * unit;
    for side in [-1.0, 1.0] {
        let eye_x = screen.center_x() + side * 2.8 * unit;
        painter.stroke_line(
            Point::new(eye_x - 1.3 * unit, eye_y),
            Point::new(eye_x + 1.3 * unit, eye_y),
            1.4 * unit,
            color(alpha, 1.0, SHELL),
            StrokeCap::Round,
        );
    }

    // "z" and "Z" each keep their own lane, rising and fading half a cycle apart.
    let fonts = FontManager::global();
    for (offset, text, size, lane_x, lane_y) in
        [(0.0, "z", 7.0, 11.0, -1.0), (0.5, "Z", 9.5, 18.0, -6.0)]
    {
        let phase = (time / DRIFT_SECONDS + offset).fract();
        let fade = (phase * std::f32::consts::PI).sin();
        fonts.draw_text_cached(DrawTextCachedParams {
            painter,
            text,
            x: center_x + (lane_x + phase * 2.0) * unit,
            y: rect.center_y() + (lane_y - phase * 4.0) * unit,
            size: size * unit,
            bold: true,
            color: color(alpha, fade * 0.9, (255, 255, 255)),
            blur: None,
        });
    }
}
