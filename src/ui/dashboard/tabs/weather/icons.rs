//! Small vector weather glyphs drawn with painter primitives, so the Weather tab needs no
//! image assets. Every glyph fits a square of side `size` centred on `center`.

use winisland_render::tokens::SemanticColors;
use winisland_render::{Painter, PathBuilder, Point, Radius, Rect, Rgba, StrokeCap, StrokeJoin};

const SUN: Rgba = Rgba::from_rgb(255, 196, 46);
const MOON: Rgba = Rgba::from_rgb(222, 226, 240);
const RAIN: Rgba = Rgba::from_rgb(90, 170, 255);
const BOLT: Rgba = Rgba::from_rgb(255, 214, 10);

/// What a glyph shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Sun,
    Moon,
    SunCloud,
    MoonCloud,
    Cloud,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Thunder,
}

/// Draws `glyph`. `background` is the colour behind it, used to cut a gap between
/// overlapping shapes (the sun behind a cloud, the moon's crescent).
pub fn draw(
    painter: Painter<'_>,
    glyph: Glyph,
    center: Point,
    size: f32,
    colors: &SemanticColors,
    background: Rgba,
) {
    let cloud = colors.text_primary;
    match glyph {
        Glyph::Sun => sun(painter, center, size, true),
        Glyph::Moon => moon(painter, center, size * 0.8, background),
        Glyph::SunCloud | Glyph::MoonCloud => {
            let behind = Point::new(center.x + size * 0.16, center.y - size * 0.16);
            if glyph == Glyph::SunCloud {
                sun(painter, behind, size * 0.7, true);
            } else {
                moon(painter, behind, size * 0.55, background);
            }
            let front = Point::new(center.x - size * 0.06, center.y + size * 0.1);
            cloud_shape(painter, front, size * 0.82, background, size * 0.06);
            cloud_shape(painter, front, size * 0.82, cloud, 0.0);
        }
        Glyph::Cloud => cloud_shape(painter, center, size, cloud, 0.0),
        Glyph::Fog => {
            let top = Point::new(center.x, center.y - size * 0.1);
            cloud_shape(painter, top, size * 0.85, cloud.with_alpha(200), 0.0);
            for (row, inset) in [(0.3, 0.12), (0.42, 0.24)] {
                let y = center.y + size * row;
                painter.stroke_line(
                    Point::new(center.x - size * (0.42 - inset), y),
                    Point::new(center.x + size * (0.42 - inset * 0.4), y),
                    (size * 0.07).max(1.2),
                    cloud.with_alpha(170),
                    StrokeCap::Round,
                );
            }
        }
        Glyph::Drizzle | Glyph::Rain | Glyph::Snow | Glyph::Thunder => {
            let top = Point::new(center.x, center.y - size * 0.12);
            cloud_shape(painter, top, size * 0.9, cloud, 0.0);
            let base = center.y + size * 0.24;
            match glyph {
                Glyph::Snow => {
                    for (dx, dy) in [(-0.22, 0.0), (0.0, 0.1), (0.22, 0.0)] {
                        painter.fill_circle(
                            Point::new(center.x + size * dx, base + size * dy + size * 0.04),
                            (size * 0.055).max(1.0),
                            MOON,
                        );
                    }
                }
                Glyph::Thunder => {
                    let unit = size;
                    let mut bolt = PathBuilder::new();
                    bolt.move_to(Point::new(center.x + unit * 0.04, base - unit * 0.06))
                        .line_to(Point::new(center.x - unit * 0.12, base + unit * 0.14))
                        .line_to(Point::new(center.x - unit * 0.01, base + unit * 0.14))
                        .line_to(Point::new(center.x - unit * 0.08, base + unit * 0.3))
                        .line_to(Point::new(center.x + unit * 0.12, base + unit * 0.08))
                        .line_to(Point::new(center.x + unit * 0.01, base + unit * 0.08))
                        .close();
                    painter.fill_path(&bolt.detach(), BOLT);
                }
                _ => {
                    let drops: &[f32] = if glyph == Glyph::Rain {
                        &[-0.22, 0.0, 0.22]
                    } else {
                        &[-0.12, 0.12]
                    };
                    let length = if glyph == Glyph::Rain { 0.2 } else { 0.11 };
                    for dx in drops {
                        let x = center.x + size * dx;
                        painter.stroke_line(
                            Point::new(x + size * 0.04, base),
                            Point::new(x - size * 0.03, base + size * length),
                            (size * 0.075).max(1.2),
                            RAIN,
                            StrokeCap::Round,
                        );
                    }
                }
            }
        }
    }
}

fn sun(painter: Painter<'_>, center: Point, size: f32, rays: bool) {
    let core = size * 0.22;
    painter.fill_circle(center, core, SUN);
    if !rays {
        return;
    }
    let inner = size * 0.32;
    let outer = size * 0.46;
    let width = (size * 0.075).max(1.2);
    for index in 0..8 {
        let angle = index as f32 * std::f32::consts::FRAC_PI_4;
        let (sin, cos) = angle.sin_cos();
        painter.stroke_line(
            Point::new(center.x + cos * inner, center.y + sin * inner),
            Point::new(center.x + cos * outer, center.y + sin * outer),
            width,
            SUN,
            StrokeCap::Round,
        );
    }
}

/// A crescent: a full disc with a background disc cut out of its upper right.
fn moon(painter: Painter<'_>, center: Point, size: f32, background: Rgba) {
    let radius = size * 0.42;
    painter.fill_circle(center, radius, MOON);
    painter.fill_circle(
        Point::new(center.x + radius * 0.55, center.y - radius * 0.4),
        radius * 0.85,
        background,
    );
}

/// A cloud built from a rounded base and two puffs; `grow` inflates it (for knockouts).
fn cloud_shape(painter: Painter<'_>, center: Point, size: f32, color: Rgba, grow: f32) {
    let base = Rect::from_ltrb(
        center.x - size * 0.45 - grow,
        center.y - size * 0.02 - grow,
        center.x + size * 0.45 + grow,
        center.y + size * 0.26 + grow,
    );
    painter.fill_round_rect(base, Radius::uniform(base.height() / 2.0), color);
    painter.fill_circle(
        Point::new(center.x - size * 0.18, center.y + size * 0.02),
        size * 0.18 + grow,
        color,
    );
    painter.fill_circle(
        Point::new(center.x + size * 0.08, center.y - size * 0.04),
        size * 0.25 + grow,
        color,
    );
}

/// A water drop (humidity).
pub fn drop(painter: Painter<'_>, center: Point, size: f32, color: Rgba) {
    let radius = size * 0.3;
    let bottom = Point::new(center.x, center.y + size * 0.15);
    let mut path = PathBuilder::new();
    path.move_to(Point::new(center.x, center.y - size * 0.45))
        .cubic_to(
            Point::new(center.x + radius * 0.4, center.y - size * 0.2),
            Point::new(bottom.x + radius * 1.35, bottom.y - radius * 0.2),
            Point::new(bottom.x, bottom.y + radius),
        )
        .cubic_to(
            Point::new(bottom.x - radius * 1.35, bottom.y - radius * 0.2),
            Point::new(center.x - radius * 0.4, center.y - size * 0.2),
            Point::new(center.x, center.y - size * 0.45),
        )
        .close();
    painter.stroke_path(
        &path.detach(),
        (size * 0.09).max(1.2),
        color,
        StrokeCap::Round,
        StrokeJoin::Round,
    );
}

/// Three wind streaks.
pub fn wind(painter: Painter<'_>, center: Point, size: f32, color: Rgba) {
    let width = (size * 0.09).max(1.2);
    for (dy, from, to) in [(-0.25, -0.42, 0.2), (0.0, -0.42, 0.42), (0.25, -0.42, 0.08)] {
        painter.stroke_line(
            Point::new(center.x + size * from, center.y + size * dy),
            Point::new(center.x + size * to, center.y + size * dy),
            width,
            color,
            StrokeCap::Round,
        );
    }
}

/// A small outlined sun (UV index).
pub fn uv(painter: Painter<'_>, center: Point, size: f32, color: Rgba) {
    let width = (size * 0.09).max(1.2);
    painter.stroke_circle(center, size * 0.18, width, color);
    for index in 0..8 {
        let angle = index as f32 * std::f32::consts::FRAC_PI_4;
        let (sin, cos) = angle.sin_cos();
        painter.stroke_line(
            Point::new(center.x + cos * size * 0.32, center.y + sin * size * 0.32),
            Point::new(center.x + cos * size * 0.45, center.y + sin * size * 0.45),
            width,
            color,
            StrokeCap::Round,
        );
    }
}
