//! Drawing helpers shared by the Shelf and File converter tabs: text measuring, small
//! stroked glyphs, dashed outlines, file-type tiles and byte formatting.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path as FsPath;

use winisland_render::text::FontManager;
use winisland_render::tokens::{SemanticColors, TextStyle};
use winisland_render::{
    Angle, FontStyle, ImageFit, ImageOptions, Painter, Path, PathBuilder, Point, Radius, Rect,
    Rgba, Sampling, StrokeCap, StrokeJoin, Vec2,
};

use crate::ui::components::text::{Align, draw_text};

pub const PLUS: &str = "M12 5 V19 M5 12 H19";
pub const CLOSE: &str = "M7 7 L17 17 M17 7 L7 17";
pub const TRASH: &str = "M4.5 7 H19.5 M10 4 H14 M6.5 7 L7.5 20 H16.5 L17.5 7 M10 11 V16 M14 11 V16";
pub const CHEVRON_DOWN: &str = "M7 10 L12 15 L17 10";
pub const CHEVRON_RIGHT: &str = "M10 7 L15 12 L10 17";
pub const UPLOAD: &str = "M12 15 V5 M8 9 L12 5 L16 9 M5 14 V19 H19 V14";
pub const CHECK: &str = "M5 12.5 L10 17 L19 7";
pub const FOLDER: &str = "M3 6.5 H9.5 L11.5 9 H21 V19 H3 Z";
pub const TRAY: &str = "M4 13 L6.5 5 H17.5 L20 13 V19 H4 Z M4 13 H9 L10 15 H14 L15 13 H20";
pub const CONVERT: &str = "M5 9 H18 L15 6 M19 15 H6 L9 18";

/// A text style with tabular figures off.
pub const fn style(size: f32, weight: u16) -> TextStyle {
    TextStyle {
        size,
        weight,
        tabular: false,
    }
}

/// One line of text in the font a `TextStyle` selects.
fn font_style(style: TextStyle) -> FontStyle {
    if style.weight >= 600 {
        FontStyle::bold()
    } else {
        FontStyle::normal()
    }
}

/// Width of `text` drawn in `style`.
pub fn text_width(text: &str, style: TextStyle) -> f32 {
    FontManager::global().measure_text_cached(text, style.size, font_style(style))
}

/// Draws `text` flush with the right edge of `rect` (ellipsized when it does not fit).
pub fn text_right(
    painter: Painter<'_>,
    text: &str,
    style: TextStyle,
    rect: Rect,
    color: Rgba,
) -> f32 {
    let width = (text_width(text, style) + 1.0).min(rect.width());
    draw_text(
        painter,
        text,
        style,
        Rect::from_ltrb(rect.right - width, rect.top, rect.right, rect.bottom),
        Align::Start,
        color,
    );
    width
}

/// Draws `text` at the left of `rect`.
pub fn text(painter: Painter<'_>, text: &str, style: TextStyle, rect: Rect, color: Rgba) {
    draw_text(painter, text, style, rect, Align::Start, color);
}

/// Draws `text` centred in `rect`.
pub fn text_center(painter: Painter<'_>, text: &str, style: TextStyle, rect: Rect, color: Rgba) {
    draw_text(painter, text, style, rect, Align::Center, color);
}

thread_local! {
    static GLYPHS: RefCell<HashMap<&'static str, Option<Path>>> = RefCell::new(HashMap::new());
}

/// Strokes a 24×24 SVG path centred in `rect`, scaled to fit.
pub fn glyph(painter: Painter<'_>, svg: &'static str, rect: Rect, color: Rgba) {
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
        painter.stroke_path(path, 1.8, color, StrokeCap::Round, StrokeJoin::Round);
        painter.restore_to(save);
    });
}

/// A square of `size` centred on `center`.
pub fn square(center: Point, size: f32) -> Rect {
    Rect::from_xywh(center.x - size / 2.0, center.y - size / 2.0, size, size)
}

/// A dashed rounded-rectangle outline: dashed straight edges, solid quarter-circle corners.
pub fn dashed_round_rect(painter: Painter<'_>, rect: Rect, radius: f32, color: Rgba) {
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let (dash, gap) = (5.0, 4.0);
    let edge = |from: Point, to: Point| {
        let length = ((to.x - from.x).powi(2) + (to.y - from.y).powi(2)).sqrt();
        if length <= 0.0 {
            return;
        }
        let (dx, dy) = ((to.x - from.x) / length, (to.y - from.y) / length);
        let mut at = 0.0;
        while at < length {
            let end = (at + dash).min(length);
            painter.stroke_line(
                Point::new(from.x + dx * at, from.y + dy * at),
                Point::new(from.x + dx * end, from.y + dy * end),
                1.0,
                color,
                StrokeCap::Butt,
            );
            at += dash + gap;
        }
    };
    let (l, t, r, b) = (rect.left, rect.top, rect.right, rect.bottom);
    edge(Point::new(l + radius, t), Point::new(r - radius, t));
    edge(Point::new(r, t + radius), Point::new(r, b - radius));
    edge(Point::new(r - radius, b), Point::new(l + radius, b));
    edge(Point::new(l, b - radius), Point::new(l, t + radius));
    let d = radius * 2.0;
    // `stroke_arc` measures angles clockwise from 12 o'clock.
    for (corner, start) in [
        (Rect::from_xywh(l, t, d, d), 270.0),
        (Rect::from_xywh(r - d, t, d, d), 0.0),
        (Rect::from_xywh(r - d, b - d, d, d), 90.0),
        (Rect::from_xywh(l, b - d, d, d), 180.0),
    ] {
        painter.stroke_arc(
            corner,
            Angle::from_degrees(start),
            Angle::from_degrees(90.0),
            1.0,
            color,
            StrokeCap::Butt,
        );
    }
}

/// A file type's short badge label and colour.
pub struct Kind {
    pub label: String,
    pub color: Rgba,
}

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "bmp", "gif", "webp", "tif", "tiff", "ico", "tga",
];

/// Lower-case extension of `path`, or an empty string.
pub fn extension(path: &FsPath) -> String {
    path.extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Whether the image decoder can read `path` (by extension).
pub fn is_image(path: &FsPath) -> bool {
    IMAGE_EXTENSIONS.contains(&extension(path).as_str())
}

pub fn kind(path: &FsPath) -> Kind {
    let extension = extension(path);
    let (label, color) = match extension.as_str() {
        "pdf" => ("PDF", Rgba::from_rgb(229, 57, 53)),
        "doc" | "docx" | "rtf" | "odt" | "pages" => ("DOC", Rgba::from_rgb(43, 108, 230)),
        "xls" | "xlsx" | "csv" | "ods" | "numbers" => ("XLS", Rgba::from_rgb(33, 150, 83)),
        "ppt" | "pptx" | "odp" | "key" => ("PPT", Rgba::from_rgb(234, 108, 40)),
        "zip" | "rar" | "7z" | "tar" | "gz" | "xz" | "bz2" => ("ZIP", Rgba::from_rgb(222, 160, 30)),
        "mp3" | "wav" | "flac" | "m4a" | "ogg" | "aac" => ("AUD", Rgba::from_rgb(226, 72, 140)),
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "wmv" => ("VID", Rgba::from_rgb(98, 92, 230)),
        "txt" | "md" | "log" => ("TXT", Rgba::from_rgb(120, 124, 134)),
        "exe" | "msi" | "appx" | "msix" => ("APP", Rgba::from_rgb(32, 156, 168)),
        "rs" | "js" | "ts" | "py" | "json" | "html" | "css" | "c" | "cpp" | "h" | "java" | "go"
        | "toml" | "yaml" | "yml" | "xml" | "sh" | "ps1" => ("CODE", Rgba::from_rgb(58, 138, 206)),
        _ if IMAGE_EXTENSIONS.contains(&extension.as_str()) || extension == "svg" => {
            ("IMG", Rgba::from_rgb(142, 92, 230))
        }
        _ => ("", Rgba::from_rgb(112, 116, 126)),
    };
    let label = if label.is_empty() {
        let short: String = extension.chars().take(4).collect();
        if short.is_empty() {
            "FILE".to_string()
        } else {
            short.to_ascii_uppercase()
        }
    } else {
        label.to_string()
    };
    Kind { label, color }
}

/// Draws a file's icon in the square `rect`: the image itself when a thumbnail is ready,
/// a folder for directories, otherwise a page with a coloured type badge.
pub fn file_icon(
    painter: Painter<'_>,
    rect: Rect,
    path: &FsPath,
    is_dir: bool,
    dim: bool,
    colors: &SemanticColors,
) {
    let alpha = |color: Rgba| {
        if dim {
            color.with_alpha((u16::from(color.a()) * 2 / 5) as u8)
        } else {
            color
        }
    };
    if is_dir {
        folder(
            painter,
            rect,
            alpha(Rgba::from_rgb(246, 196, 64)),
            alpha(Rgba::from_rgb(222, 166, 40)),
        );
        return;
    }
    if let Some(image) = super::thumbs::thumbnail(path) {
        let frame = rect.inset(rect.width() * 0.04);
        let radius = (frame.width() * 0.16).max(4.0);
        let save = painter.save();
        painter.clip_round_rect(frame, Radius::uniform(radius));
        painter.draw_image(
            &image,
            frame,
            &ImageOptions {
                fit: ImageFit::Cover,
                sampling: Sampling::LinearNone,
                alpha: if dim { 100 } else { 255 },
                ..ImageOptions::default()
            },
        );
        painter.restore_to(save);
        painter.stroke_round_rect(frame, Radius::uniform(radius), 1.0, colors.border_subtle);
        return;
    }
    page(painter, rect, &kind(path), dim);
}

fn folder(painter: Painter<'_>, rect: Rect, body: Rgba, back: Rgba) {
    let width = rect.width() * 0.9;
    let height = rect.height() * 0.7;
    let left = rect.center_x() - width / 2.0;
    let top = rect.center_y() - height / 2.0 + rect.height() * 0.03;
    let tab = Rect::from_xywh(left, top - height * 0.1, width * 0.42, height * 0.3);
    painter.fill_round_rect(tab, Radius::uniform(height * 0.08), back);
    painter.fill_round_rect(
        Rect::from_xywh(left, top, width, height),
        Radius::uniform(height * 0.1),
        back,
    );
    painter.fill_round_rect(
        Rect::from_xywh(left, top + height * 0.16, width, height * 0.84),
        Radius::uniform(height * 0.1),
        body,
    );
}

fn page(painter: Painter<'_>, rect: Rect, kind: &Kind, dim: bool) {
    let height = rect.height() * 0.92;
    let width = height * 0.78;
    let left = rect.center_x() - width / 2.0;
    let top = rect.center_y() - height / 2.0;
    let fold = width * 0.28;
    let (right, bottom) = (left + width, top + height);
    let paper = Rgba::from_rgb(236, 237, 241).with_alpha(if dim { 90 } else { 255 });
    let shade = Rgba::from_rgb(196, 199, 207).with_alpha(if dim { 90 } else { 255 });
    let round = width * 0.08;
    let mut builder = PathBuilder::new();
    builder
        .move_to(Point::new(left + round, top))
        .line_to(Point::new(right - fold, top))
        .line_to(Point::new(right, top + fold))
        .line_to(Point::new(right, bottom - round))
        .conic_to(
            Point::new(right, bottom),
            Point::new(right - round, bottom),
            0.707,
        )
        .line_to(Point::new(left + round, bottom))
        .conic_to(
            Point::new(left, bottom),
            Point::new(left, bottom - round),
            0.707,
        )
        .line_to(Point::new(left, top + round))
        .conic_to(Point::new(left, top), Point::new(left + round, top), 0.707)
        .close();
    painter.fill_path(&builder.detach(), paper);
    let mut corner = PathBuilder::new();
    corner
        .move_to(Point::new(right - fold, top))
        .line_to(Point::new(right - fold, top + fold))
        .line_to(Point::new(right, top + fold))
        .close();
    painter.fill_path(&corner.detach(), shade);
    // Faint text lines above the badge.
    for line in 0..2 {
        let y = top + height * (0.2 + line as f32 * 0.1);
        painter.fill_round_rect(
            Rect::from_xywh(
                left + width * 0.16,
                y,
                width * if line == 0 { 0.4 } else { 0.55 },
                height * 0.035,
            ),
            Radius::uniform(1.0),
            shade,
        );
    }
    let badge = Rect::from_xywh(
        left - width * 0.1,
        top + height * 0.5,
        width * 1.2,
        height * 0.26,
    );
    painter.fill_round_rect(
        badge,
        Radius::uniform(height * 0.05),
        kind.color.with_alpha(if dim { 110 } else { 255 }),
    );
    let size = (badge.height() * 0.62).clamp(8.0, 14.0);
    text_center(
        painter,
        &kind.label,
        style(size, 700),
        badge,
        Rgba::from_rgb(255, 255, 255).with_alpha(if dim { 150 } else { 255 }),
    );
}

/// "39.7 MB", "160 KB", "512 B", "1.2 GB".
pub fn format_bytes(bytes: u64) -> String {
    let value = bytes as f64;
    if value >= 1_073_741_824.0 {
        format!("{:.1} GB", value / 1_073_741_824.0)
    } else if value >= 1_048_576.0 {
        format!("{:.1} MB", value / 1_048_576.0)
    } else if value >= 1024.0 {
        format!("{:.0} KB", value / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// A small round icon button: a hover disc behind the glyph.
pub fn icon_button(
    painter: Painter<'_>,
    rect: Rect,
    svg: &'static str,
    hovered: bool,
    enabled: bool,
    colors: &SemanticColors,
) {
    if hovered && enabled {
        painter.fill_round_rect(
            rect,
            Radius::uniform(rect.height() / 2.0),
            colors.surface_overlay,
        );
    }
    let color = if !enabled {
        colors.text_disabled
    } else if hovered {
        colors.text_primary
    } else {
        colors.text_secondary
    };
    glyph(
        painter,
        svg,
        square(Point::new(rect.center_x(), rect.center_y()), 16.0),
        color,
    );
}
