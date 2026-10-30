use winisland_render::text::FontManager;
use winisland_render::tokens::TextStyle;
use winisland_render::{FontStyle, Painter, Point, Rect, Rgba};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Start,
    Center,
}

/// Draws one line of text vertically centered in `rect`, truncated with an ellipsis.
///
/// The font manager only distinguishes regular and bold, so weights of 600 and above
/// render bold and everything else regular.
pub fn draw_text(
    painter: Painter<'_>,
    text: &str,
    style: TextStyle,
    rect: Rect,
    align: Align,
    color: Rgba,
) {
    if text.is_empty() || rect.width() <= 0.0 {
        return;
    }
    let fonts = FontManager::global();
    let bold = style.weight >= 600;
    let font_style = if bold {
        FontStyle::bold()
    } else {
        FontStyle::normal()
    };
    let fitted = crate::utils::settings_ui::ellipsize_text(
        fonts,
        text,
        style.size,
        font_style,
        rect.width(),
    );
    let width = fonts.measure_text_cached(&fitted, style.size, font_style);
    let x = match align {
        Align::Start => rect.left,
        Align::Center => rect.left + (rect.width() - width).max(0.0) / 2.0,
    };
    // Text is drawn from its baseline; this offset centers typical Latin and CJK glyphs.
    let baseline = rect.center_y() + style.size * 0.35;
    fonts.draw_str(
        painter,
        &fitted,
        Point::new(x, baseline),
        style.size,
        bold,
        color,
    );
}
