use std::cell::RefCell;

use winisland_render::text::{DrawTextCachedParams, FontManager};
use winisland_render::{FontStyle, Painter, Rect, Rgba};

const SIZE: f32 = 11.0;
const PADDING: f32 = 6.0;
const MIN_WIDTH: f32 = 40.0;
const MAX_WIDTH: f32 = 220.0;

thread_local! {
    static MESSAGE: RefCell<String> = const { RefCell::new(String::new()) };
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

/// Width at 1× scale, sized to the message within limits.
pub(super) fn width() -> f32 {
    MESSAGE.with(|cell| {
        let text = cell.borrow();
        let measured = FontManager::global().measure_text_cached(&text, SIZE, FontStyle::bold());
        (measured + PADDING * 2.0).clamp(MIN_WIDTH, MAX_WIDTH)
    })
}

pub(super) fn draw(painter: Painter<'_>, rect: Rect, scale: f32, alpha: u8) {
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
