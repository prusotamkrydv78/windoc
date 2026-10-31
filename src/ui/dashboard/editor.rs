//! In-island note editor. Escape saves and closes; an empty new note is discarded.

use std::cell::RefCell;

use winisland_core::i18n::tr;
use winisland_core::text_edit::{AreaKey, TextArea};
use winisland_platform::Key;
use winisland_render::text::FontManager;
use winisland_render::tokens::{SemanticColors, radius, space, typography};
use winisland_render::{FontStyle, Painter, Point, Radius, Rect, StrokeCap};

use crate::ui::components::text::{Align, draw_text};

/// Matches the notes store's body limit (256 KiB) in characters, conservatively.
const MAX_NOTE_CHARS: usize = 64 * 1024;

struct EditorState {
    note: Option<u64>,
    area: TextArea,
}

thread_local! {
    static EDITOR: RefCell<Option<EditorState>> = const { RefCell::new(None) };
}

pub fn open(note: Option<u64>, body: &str) {
    EDITOR.with(|cell| {
        *cell.borrow_mut() = Some(EditorState {
            note,
            area: TextArea::with_text(body, MAX_NOTE_CHARS),
        });
    });
}

pub fn is_open() -> bool {
    EDITOR.with(|cell| cell.borrow().is_some())
}

/// Saves and closes the editor.
pub fn close() {
    let state = EDITOR.with(|cell| cell.borrow_mut().take());
    if let Some(state) = state {
        super::save_note(state.note, state.area.text());
    }
}

/// Handles a key while the editor is open.
pub fn handle_key(key: &Key) {
    if *key == Key::Escape {
        close();
        return;
    }
    EDITOR.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(state) = slot.as_mut() else {
            return;
        };
        let area_key = match key {
            Key::Enter => Some(AreaKey::Enter),
            Key::Backspace => Some(AreaKey::Backspace),
            Key::Delete => Some(AreaKey::Delete),
            Key::ArrowLeft => Some(AreaKey::Left),
            Key::ArrowRight => Some(AreaKey::Right),
            Key::ArrowUp => Some(AreaKey::Up),
            Key::ArrowDown => Some(AreaKey::Down),
            Key::Home => Some(AreaKey::Home),
            Key::End => Some(AreaKey::End),
            Key::Tab => {
                state.area.insert("    ");
                None
            }
            Key::Character(text) => {
                state.area.insert(text);
                None
            }
            _ => None,
        };
        if let Some(area_key) = area_key {
            state.area.apply(area_key);
        }
    });
}

pub fn draw(painter: Painter<'_>, grid: Rect, colors: &SemanticColors) {
    EDITOR.with(|cell| {
        let slot = cell.borrow();
        let Some(state) = slot.as_ref() else {
            return;
        };
        painter.fill_round_rect(grid, Radius::uniform(radius::CARD), colors.surface_raised);
        painter.stroke_round_rect(
            grid,
            Radius::uniform(radius::CARD),
            1.0,
            colors.accent_primary,
        );
        let hint_rect = Rect::from_ltrb(
            grid.left + space::S3,
            grid.bottom - typography::CAPTION.size - space::S2,
            grid.right - space::S3,
            grid.bottom - space::S1,
        );
        draw_text(
            painter,
            &tr("notes_editor_hint"),
            typography::CAPTION,
            hint_rect,
            Align::Start,
            colors.text_secondary,
        );
        let line_height = typography::BODY.size * 1.4;
        let text_left = grid.left + space::S3;
        let text_top = grid.top + space::S2;
        let visible_lines = ((hint_rect.top - text_top) / line_height).floor().max(1.0) as usize;
        let (caret_line, caret_column) = state.area.caret_line_column();
        // Scroll so the caret line stays visible.
        let first_line = caret_line.saturating_sub(visible_lines.saturating_sub(1));
        let lines = state.area.lines();
        let save = painter.save();
        painter.clip_rect(Rect::from_ltrb(
            grid.left,
            grid.top,
            grid.right,
            hint_rect.top,
        ));
        for (offset, line) in lines
            .iter()
            .skip(first_line)
            .take(visible_lines)
            .enumerate()
        {
            let top = text_top + offset as f32 * line_height;
            draw_text(
                painter,
                line,
                typography::BODY,
                Rect::from_ltrb(text_left, top, grid.right - space::S3, top + line_height),
                Align::Start,
                colors.text_primary,
            );
        }
        let blink_on = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(true, |elapsed| {
                (elapsed.as_millis() / 500).is_multiple_of(2)
            });
        if blink_on && caret_line >= first_line {
            let line = lines.get(caret_line).copied().unwrap_or_default();
            let before: String = line.chars().take(caret_column).collect();
            let caret_x = text_left
                + FontManager::global().measure_text_cached(
                    &before,
                    typography::BODY.size,
                    FontStyle::normal(),
                );
            let top = text_top + (caret_line - first_line) as f32 * line_height;
            painter.stroke_line(
                Point::new(caret_x, top + 3.0),
                Point::new(caret_x, top + line_height - 3.0),
                1.0,
                colors.accent_primary,
                StrokeCap::Butt,
            );
        }
        painter.restore_to(save);
    });
}
