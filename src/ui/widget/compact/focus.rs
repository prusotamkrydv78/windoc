use winisland_render::text::{DrawTextCachedParams, FontManager};
use winisland_render::{Painter, Rect, Rgba};

/// Focus countdown in the collapsed island, styled like the time widget.
pub(super) fn draw(painter: Painter<'_>, rect: Rect, scale: f32, alpha: u8) {
    let size = (rect.height() * 0.43).clamp(10.0 * scale, 12.0 * scale);
    let (text, running) = crate::ui::dashboard::focus_compact_text();
    let color = if running {
        Rgba::from_argb(alpha, 255, 255, 255)
    } else {
        Rgba::from_argb((u16::from(alpha) * 3 / 5) as u8, 255, 255, 255)
    };
    let fonts = FontManager::global();
    let text_width = fonts.measure_text_cached(&text, size, winisland_render::FontStyle::bold());
    fonts.draw_text_cached(DrawTextCachedParams {
        painter,
        text: &text,
        x: rect.center_x() - text_width / 2.0,
        y: rect.center_y() + size * 0.36,
        size,
        bold: true,
        color,
        blur: None,
    });
}
