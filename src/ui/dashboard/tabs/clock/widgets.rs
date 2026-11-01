//! Small controls shared by the Clock and Timers tabs: right-aligned text, toggles,
//! segmented pickers, steppers, round buttons and stroked glyphs.

use std::cell::RefCell;
use std::collections::HashMap;

use winisland_render::text::FontManager;
use winisland_render::tokens::{SemanticColors, TextStyle, typography};
use winisland_render::{
    FontStyle, Painter, Path, Point, Radius, Rect, StrokeCap, StrokeJoin, Vec2,
};

use crate::ui::components::text::{Align, draw_text};

/// Inner padding of a card.
pub const CARD_PAD: f32 = 16.0;
/// Width and height of a toggle switch.
pub const TOGGLE_W: f32 = 36.0;
pub const TOGGLE_H: f32 = 20.0;
/// Height of a segmented picker.
pub const SEGMENT_H: f32 = 24.0;

/// Width of `text` drawn in `style`.
pub fn text_width(text: &str, style: TextStyle) -> f32 {
    let font = if style.weight >= 600 {
        FontStyle::bold()
    } else {
        FontStyle::normal()
    };
    FontManager::global().measure_text_cached(text, style.size, font)
}

/// One line of text flush with the right edge of `rect`.
pub fn draw_right(
    painter: Painter<'_>,
    text: &str,
    style: TextStyle,
    rect: Rect,
    color: winisland_render::Rgba,
) {
    let width = text_width(text, style) + 1.0;
    let left = (rect.right - width).max(rect.left);
    draw_text(
        painter,
        text,
        style,
        Rect::from_ltrb(left, rect.top, rect.right, rect.bottom),
        Align::Start,
        color,
    );
}

/// A switch: white track with a dark knob when on, grey when off.
pub fn draw_toggle(
    painter: Painter<'_>,
    rect: Rect,
    on: bool,
    hovered: bool,
    colors: &SemanticColors,
) {
    let track = if on {
        colors.text_primary
    } else if hovered {
        colors.surface_interactive.with_alpha(255)
    } else {
        colors.surface_overlay
    };
    painter.fill_round_rect(rect, Radius::uniform(rect.height() / 2.0), track);
    if !on {
        painter.stroke_round_rect(
            rect,
            Radius::uniform(rect.height() / 2.0),
            1.0,
            colors.border_subtle,
        );
    }
    let knob_r = rect.height() / 2.0 - 3.0;
    let cx = if on {
        rect.right - rect.height() / 2.0
    } else {
        rect.left + rect.height() / 2.0
    };
    let knob = if on {
        colors.surface_base
    } else {
        colors.text_secondary
    };
    painter.fill_circle(Point::new(cx, rect.center_y()), knob_r, knob);
}

/// A toggle of the standard size, flush right and vertically centred in `row`.
pub fn toggle_rect(row: Rect) -> Rect {
    Rect::from_xywh(
        row.right - TOGGLE_W,
        row.center_y() - TOGGLE_H / 2.0,
        TOGGLE_W,
        TOGGLE_H,
    )
}

const SEGMENT_STYLE: TextStyle = TextStyle {
    size: 12.0,
    weight: 600,
    tabular: false,
};

/// Segment rects of a picker whose right edge is `right`, centred on `center_y`.
/// Returns the background and one rect per label.
pub fn segment_rects(right: f32, center_y: f32, labels: &[&str]) -> (Rect, Vec<Rect>) {
    let widths: Vec<f32> = labels
        .iter()
        .map(|label| text_width(label, SEGMENT_STYLE).ceil() + 20.0)
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 4.0;
    let background = Rect::from_xywh(right - total, center_y - SEGMENT_H / 2.0, total, SEGMENT_H);
    let mut left = background.left + 2.0;
    let rects = widths
        .iter()
        .map(|width| {
            let rect = Rect::from_xywh(left, background.top + 2.0, *width, SEGMENT_H - 4.0);
            left += width;
            rect
        })
        .collect();
    (background, rects)
}

/// A segmented picker: the selected segment is outlined and bold white.
pub fn draw_segmented(
    painter: Painter<'_>,
    background: Rect,
    rects: &[Rect],
    labels: &[&str],
    selected: usize,
    hovered: Option<usize>,
    colors: &SemanticColors,
) {
    painter.fill_round_rect(
        background,
        Radius::uniform(background.height() / 2.0),
        colors.surface_overlay,
    );
    for (index, (rect, label)) in rects.iter().zip(labels).enumerate() {
        let radius = Radius::uniform(rect.height() / 2.0);
        let active = index == selected;
        if active {
            painter.fill_round_rect(*rect, radius, colors.surface_base);
            painter.stroke_round_rect(rect.inset(0.5), radius, 1.2, colors.text_primary);
        } else if hovered == Some(index) {
            painter.fill_round_rect(*rect, radius, colors.surface_interactive);
        }
        let color = if active || hovered == Some(index) {
            colors.text_primary
        } else {
            colors.text_secondary
        };
        draw_text(painter, label, SEGMENT_STYLE, *rect, Align::Center, color);
    }
}

/// The two halves of a stepper that react to clicks: minus (left) and plus (right).
pub fn stepper_halves(rect: Rect) -> (Rect, Rect) {
    let width = rect.height().max(28.0);
    (
        Rect::from_xywh(rect.left, rect.top, width, rect.height()),
        Rect::from_xywh(rect.right - width, rect.top, width, rect.height()),
    )
}

/// A pill with "−" and "+" at its ends and `label` in the middle.
pub fn draw_stepper(
    painter: Painter<'_>,
    rect: Rect,
    label: &str,
    hovered: Option<bool>,
    colors: &SemanticColors,
) {
    let radius = Radius::uniform(rect.height() / 2.0);
    painter.fill_round_rect(rect, radius, colors.surface_raised);
    let (minus, plus) = stepper_halves(rect);
    for (half, is_plus) in [(minus, false), (plus, true)] {
        let hot = hovered == Some(is_plus);
        if hot {
            painter.fill_round_rect(half.inset(2.0), radius, colors.surface_interactive);
        }
        let color = if hot {
            colors.text_primary
        } else {
            colors.text_secondary
        };
        let c = Point::new(half.center_x(), half.center_y());
        painter.stroke_line(
            Point::new(c.x - 5.0, c.y),
            Point::new(c.x + 5.0, c.y),
            1.5,
            color,
            StrokeCap::Round,
        );
        if is_plus {
            painter.stroke_line(
                Point::new(c.x, c.y - 5.0),
                Point::new(c.x, c.y + 5.0),
                1.5,
                color,
                StrokeCap::Round,
            );
        }
    }
    draw_text(
        painter,
        label,
        typography::LABEL,
        Rect::from_ltrb(minus.right, rect.top, plus.left, rect.bottom),
        Align::Center,
        colors.text_primary,
    );
}

/// A pill-shaped chip; `active` fills it white.
pub fn draw_chip(
    painter: Painter<'_>,
    rect: Rect,
    label: &str,
    active: bool,
    hovered: bool,
    colors: &SemanticColors,
) {
    let radius = Radius::uniform(rect.height() / 2.0);
    let (fill, text) = if active {
        (colors.text_primary, colors.surface_base)
    } else if hovered {
        (colors.surface_interactive, colors.text_primary)
    } else {
        (colors.surface_raised, colors.text_secondary)
    };
    painter.fill_round_rect(rect, radius, fill);
    draw_text(painter, label, typography::LABEL, rect, Align::Center, text);
}

/// A round button. The primary one is white with a dark glyph; the others are grey.
/// Returns the glyph colour to draw with.
pub fn draw_round_button(
    painter: Painter<'_>,
    rect: Rect,
    primary: bool,
    hovered: bool,
    enabled: bool,
    colors: &SemanticColors,
) -> winisland_render::Rgba {
    let center = Point::new(rect.center_x(), rect.center_y());
    let radius = rect.width().min(rect.height()) / 2.0;
    if primary {
        let fill = if hovered {
            colors.text_primary.with_alpha(225)
        } else {
            colors.text_primary
        };
        painter.fill_circle(center, radius, fill);
        colors.surface_base
    } else {
        let fill = if hovered && enabled {
            colors.surface_interactive
        } else {
            colors.surface_overlay
        };
        painter.fill_circle(center, radius, fill);
        if enabled {
            colors.text_primary
        } else {
            colors.text_disabled
        }
    }
}

thread_local! {
    static GLYPHS: RefCell<HashMap<&'static str, Option<Path>>> = RefCell::new(HashMap::new());
}

/// Strokes an SVG path drawn on a 24 × 24 grid, scaled to fit `rect`.
pub fn draw_glyph(
    painter: Painter<'_>,
    svg: &'static str,
    rect: Rect,
    color: winisland_render::Rgba,
    width: f32,
) {
    let size = rect.width().min(rect.height());
    if size <= 0.0 {
        return;
    }
    GLYPHS.with(|cache| {
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
        painter.stroke_path(path, width, color, StrokeCap::Round, StrokeJoin::Round);
        painter.restore_to(save);
    });
}

/// A small globe for the World clocks header.
pub const GLOBE: &str = "M12 3 A9 9 0 1 1 11.99 3 M3 12 H21 M12 3 A14 14 0 0 1 12 21 \
                         M12 3 A14 14 0 0 0 12 21";
