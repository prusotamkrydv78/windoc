use std::sync::OnceLock;
use std::time::{Duration, Instant};

use winisland_render::text::{DrawTextCachedParams, FontManager};
use winisland_render::{Painter, Point, Radius, Rect, Rgba, StrokeCap};

/// A sleeping robot for the idle island: it breathes slowly while a "z Z" drifts up.
pub(super) const WIDTH: f32 = 46.0;
/// About 15 frames a second keeps the drift smooth without busy redraws.
pub(super) const FRAME: Duration = Duration::from_millis(66);
const BREATH_SECONDS: f32 = 3.2;
const DRIFT_SECONDS: f32 = 2.4;

fn seconds() -> f32 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

fn with_alpha(alpha: u8, opacity: f32, r: u8, g: u8, b: u8) -> Rgba {
    Rgba::from_argb((f32::from(alpha) * opacity.clamp(0.0, 1.0)) as u8, r, g, b)
}

pub(super) fn draw(painter: Painter<'_>, rect: Rect, scale: f32, alpha: u8) {
    let time = seconds();
    let unit = (rect.height() / 32.0).clamp(0.6 * scale, 1.4 * scale);
    let breath = (time / BREATH_SECONDS * std::f32::consts::TAU).sin();
    let center_x = rect.left + 15.0 * unit;
    let base_y = rect.center_y() + 7.0 * unit;

    // Desk under the robot.
    let desk = Rect::from_xywh(center_x - 11.0 * unit, base_y, 22.0 * unit, 2.6 * unit);
    painter.fill_round_rect(
        desk,
        Radius::uniform(1.3 * unit),
        with_alpha(alpha, 0.9, 236, 230, 214),
    );

    // Head, rising and settling with each breath.
    let head_top = base_y - 15.0 * unit - breath * 0.7 * unit;
    let head = Rect::from_xywh(center_x - 7.5 * unit, head_top, 15.0 * unit, 12.0 * unit);
    painter.fill_round_rect(
        head,
        Radius::uniform(3.6 * unit),
        with_alpha(alpha, 1.0, 245, 240, 226),
    );
    // Antenna.
    let antenna_base = Point::new(center_x, head_top);
    let antenna_tip = Point::new(center_x, head_top - 3.0 * unit);
    painter.stroke_line(
        antenna_base,
        antenna_tip,
        1.2 * unit,
        with_alpha(alpha, 1.0, 245, 240, 226),
        StrokeCap::Round,
    );
    painter.fill_circle(
        antenna_tip,
        1.3 * unit,
        with_alpha(alpha, 1.0, 245, 240, 226),
    );
    // Closed eyes.
    let eye_y = head_top + 6.2 * unit;
    for side in [-1.0, 1.0] {
        let eye_x = center_x + side * 3.2 * unit;
        painter.stroke_line(
            Point::new(eye_x - 1.4 * unit, eye_y),
            Point::new(eye_x + 1.4 * unit, eye_y),
            1.3 * unit,
            with_alpha(alpha, 1.0, 34, 34, 38),
            StrokeCap::Round,
        );
    }
    // Neck between head and desk.
    let neck = Rect::from_xywh(
        center_x - 2.5 * unit,
        head.bottom,
        5.0 * unit,
        (base_y - head.bottom).max(0.0),
    );
    painter.fill_rect(neck, with_alpha(alpha, 0.9, 236, 230, 214));

    // Two letters drift up and fade, half a cycle apart.
    let fonts = FontManager::global();
    // Each letter keeps its own lane so they never overlap.
    for (offset, text, size, lane_x, lane_y) in
        [(0.0, "z", 7.0, 11.0, 3.0), (0.5, "Z", 9.5, 19.0, -2.0)]
    {
        let phase = (time / DRIFT_SECONDS + offset).fract();
        let fade = (phase * std::f32::consts::PI).sin();
        let x = center_x + (lane_x + phase * 2.0) * unit;
        let y = rect.center_y() + (lane_y - phase * 4.0) * unit;
        fonts.draw_text_cached(DrawTextCachedParams {
            painter,
            text,
            x,
            y,
            size: size * unit,
            bold: true,
            color: with_alpha(alpha, fade * 0.9, 255, 255, 255),
            blur: None,
        });
    }
}
