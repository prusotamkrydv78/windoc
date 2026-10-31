//! Command palette overlay on the dashboard header.
//!
//! While open, the island takes keyboard focus; typed keys edit the query and Enter runs
//! the parsed command. Conversions show their result in place instead of closing.

use std::cell::RefCell;

use winisland_core::i18n::tr;
use winisland_core::palette::{self, Command};
use winisland_core::text_edit::{EditKey, TextBuffer};
use winisland_platform::Key;
use winisland_render::text::FontManager;
use winisland_render::tokens::{SemanticColors, radius, space, typography};
use winisland_render::{FontStyle, Painter, Point, Radius, Rect, StrokeCap};

use crate::ui::components::text::{Align, draw_text};

const MAX_QUERY_CHARS: usize = 200;

struct PaletteState {
    query: TextBuffer,
    result: Option<String>,
}

thread_local! {
    static PALETTE: RefCell<Option<PaletteState>> = const { RefCell::new(None) };
}

pub fn open() {
    PALETTE.with(|cell| {
        *cell.borrow_mut() = Some(PaletteState {
            query: TextBuffer::new(MAX_QUERY_CHARS),
            result: None,
        });
    });
}

pub fn close() {
    PALETTE.with(|cell| *cell.borrow_mut() = None);
}

pub fn is_open() -> bool {
    PALETTE.with(|cell| cell.borrow().is_some())
}

fn preview(query: &str) -> Option<String> {
    match palette::parse(query)? {
        Command::Convert {
            value,
            from,
            to,
            result,
        } => Some(format!(
            "{} {from} = {} {to}",
            winisland_core::units::format_value(value, 4),
            winisland_core::units::format_value(result, 4)
        )),
        Command::StartTimer(duration) => Some(winisland_core::i18n::tr_args(
            "palette_timer",
            &[&(duration.as_secs() / 60).to_string()],
        )),
        Command::StartPomodoro => Some(tr("palette_pomodoro")),
        Command::StartStopwatch => Some(tr("palette_stopwatch")),
        Command::QuickNote(_) => Some(tr("palette_note")),
        Command::OpenFolder(_) | Command::OpenModule(_) => Some(tr("palette_open")),
        Command::SearchClipboard(_) => Some(tr("palette_clipboard")),
        Command::WebSearch { provider, .. } => {
            Some(winisland_core::i18n::tr_args("palette_search", &[provider]))
        }
    }
}

/// Handles a key while the palette is open. Returns a command to run when Enter is
/// pressed on an actionable query; the palette closes unless it showed a conversion.
pub fn handle_key(key: &Key) -> Option<Command> {
    PALETTE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let state = slot.as_mut()?;
        match key {
            Key::Escape => {
                *slot = None;
                return None;
            }
            Key::Enter => {
                let command = palette::parse(state.query.text());
                if matches!(command, Some(Command::Convert { .. })) {
                    state.result = preview(state.query.text());
                    return None;
                }
                if command.is_some() {
                    *slot = None;
                }
                return command;
            }
            Key::Backspace => {
                state.query.apply(EditKey::Backspace);
            }
            Key::Delete => {
                state.query.apply(EditKey::Delete);
            }
            Key::ArrowLeft => {
                state.query.apply(EditKey::Left);
            }
            Key::ArrowRight => {
                state.query.apply(EditKey::Right);
            }
            Key::Home => {
                state.query.apply(EditKey::Home);
            }
            Key::End => {
                state.query.apply(EditKey::End);
            }
            Key::Character(text) => {
                state.query.insert(text);
            }
            _ => return None,
        }
        state.result = preview(state.query.text());
        None
    })
}

/// Draws the palette over `header` and the area below it.
pub fn draw(painter: Painter<'_>, header: Rect, colors: &SemanticColors, show_caret: bool) {
    PALETTE.with(|cell| {
        let slot = cell.borrow();
        let Some(state) = slot.as_ref() else {
            return;
        };
        let field = header.inset(-space::S1);
        painter.fill_round_rect(field, Radius::uniform(radius::ROW), colors.surface_overlay);
        painter.stroke_round_rect(
            field,
            Radius::uniform(radius::ROW),
            1.0,
            colors.accent_primary,
        );
        let text_rect = Rect::from_ltrb(
            field.left + space::S3,
            field.top,
            field.right - space::S3,
            field.bottom,
        );
        let (text, color) = if state.query.is_empty() {
            (tr("palette_placeholder"), colors.text_secondary)
        } else {
            (state.query.text().to_string(), colors.text_primary)
        };
        draw_text(
            painter,
            &text,
            typography::BODY,
            text_rect,
            Align::Start,
            color,
        );
        if show_caret {
            let caret_x = text_rect.left
                + FontManager::global().measure_text_cached(
                    state.query.before_caret(),
                    typography::BODY.size,
                    FontStyle::normal(),
                );
            painter.stroke_line(
                Point::new(caret_x, field.top + space::S2),
                Point::new(caret_x, field.bottom - space::S2),
                1.0,
                colors.accent_primary,
                StrokeCap::Butt,
            );
        }
        if let Some(result) = &state.result {
            let result_rect = Rect::from_ltrb(
                text_rect.left,
                field.bottom + space::S1,
                text_rect.right,
                field.bottom + space::S1 + typography::LABEL.size + space::S2,
            );
            draw_text(
                painter,
                result,
                typography::LABEL,
                result_rect,
                Align::Start,
                colors.text_secondary,
            );
        }
    });
}
