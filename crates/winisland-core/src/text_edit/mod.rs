//! Single-line text editing model shared by text fields (search, city names, notes titles).
//!
//! Positions count Unicode scalar values (`char`s), so edits never split a UTF-8 sequence.
//! Grapheme clusters such as combined emoji may take more than one arrow press; that is a
//! known limitation until a segmentation library is adopted.

mod area;
pub use area::{AreaKey, TextArea};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditKey {
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextBuffer {
    text: String,
    /// Caret position in chars, `0..=char_count`.
    caret: usize,
    max_chars: usize,
}

impl TextBuffer {
    /// An empty buffer accepting at most `max_chars` characters.
    pub fn new(max_chars: usize) -> Self {
        Self {
            text: String::new(),
            caret: 0,
            max_chars,
        }
    }

    /// A buffer holding `text` (truncated to the limit) with the caret at the end.
    pub fn with_text(text: &str, max_chars: usize) -> Self {
        let mut buffer = Self::new(max_chars);
        buffer.insert(text);
        buffer
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    pub fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Text before the caret, for measuring where to draw it.
    pub fn before_caret(&self) -> &str {
        &self.text[..self.byte_index(self.caret)]
    }

    fn byte_index(&self, char_index: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_index)
            .map_or(self.text.len(), |(index, _)| index)
    }

    /// Inserts typed or pasted text at the caret. Control characters (including line
    /// breaks) are dropped and text beyond the limit is cut. Returns whether anything
    /// changed.
    pub fn insert(&mut self, input: &str) -> bool {
        let room = self.max_chars.saturating_sub(self.char_count());
        let accepted: String = input
            .chars()
            .filter(|character| !character.is_control())
            .take(room)
            .collect();
        if accepted.is_empty() {
            return false;
        }
        let at = self.byte_index(self.caret);
        self.text.insert_str(at, &accepted);
        self.caret += accepted.chars().count();
        true
    }

    /// Applies an editing key. Returns whether the text or caret changed.
    pub fn apply(&mut self, key: EditKey) -> bool {
        let count = self.char_count();
        match key {
            EditKey::Backspace if self.caret > 0 => {
                let start = self.byte_index(self.caret - 1);
                let end = self.byte_index(self.caret);
                self.text.replace_range(start..end, "");
                self.caret -= 1;
                true
            }
            EditKey::Delete if self.caret < count => {
                let start = self.byte_index(self.caret);
                let end = self.byte_index(self.caret + 1);
                self.text.replace_range(start..end, "");
                true
            }
            EditKey::Left if self.caret > 0 => {
                self.caret -= 1;
                true
            }
            EditKey::Right if self.caret < count => {
                self.caret += 1;
                true
            }
            EditKey::Home if self.caret > 0 => {
                self.caret = 0;
                true
            }
            EditKey::End if self.caret < count => {
                self.caret = count;
                true
            }
            _ => false,
        }
    }

    /// Moves the caret to `char_index`, clamped to the text.
    pub fn set_caret(&mut self, char_index: usize) {
        self.caret = char_index.min(self.char_count());
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.caret = 0;
    }
}

#[cfg(test)]
mod tests;
