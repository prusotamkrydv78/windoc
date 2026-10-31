//! Multi-line text editing model for the Notes editor.

/// Keys a text area understands beyond inserted text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AreaKey {
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Enter,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextArea {
    text: String,
    /// Caret position in chars.
    caret: usize,
    max_chars: usize,
}

impl TextArea {
    pub fn with_text(text: &str, max_chars: usize) -> Self {
        let text: String = text.chars().take(max_chars).collect();
        let caret = text.chars().count();
        Self {
            text,
            caret,
            max_chars,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    fn chars(&self) -> Vec<char> {
        self.text.chars().collect()
    }

    fn byte_index(&self, char_index: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_index)
            .map_or(self.text.len(), |(index, _)| index)
    }

    /// Caret as `(line, column)`, both counted in chars from zero.
    pub fn caret_line_column(&self) -> (usize, usize) {
        let chars = self.chars();
        let before = &chars[..self.caret.min(chars.len())];
        let line = before
            .iter()
            .filter(|character| **character == '\n')
            .count();
        let column = before
            .iter()
            .rev()
            .take_while(|character| **character != '\n')
            .count();
        (line, column)
    }

    /// Char index where `line` starts, clamped to the last line.
    fn line_start(chars: &[char], line: usize) -> usize {
        let mut current = 0;
        for (index, character) in chars.iter().enumerate() {
            if current == line {
                return index;
            }
            if *character == '\n' {
                current += 1;
            }
        }
        chars.len()
    }

    fn line_length(chars: &[char], start: usize) -> usize {
        chars[start..]
            .iter()
            .take_while(|character| **character != '\n')
            .count()
    }

    pub fn insert(&mut self, input: &str) -> bool {
        let room = self.max_chars.saturating_sub(self.text.chars().count());
        let accepted: String = input
            .chars()
            .filter(|character| *character == '\n' || !character.is_control())
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

    pub fn apply(&mut self, key: AreaKey) -> bool {
        let chars = self.chars();
        let count = chars.len();
        let (line, column) = self.caret_line_column();
        match key {
            AreaKey::Enter => return self.insert("\n"),
            AreaKey::Backspace if self.caret > 0 => {
                let start = self.byte_index(self.caret - 1);
                let end = self.byte_index(self.caret);
                self.text.replace_range(start..end, "");
                self.caret -= 1;
            }
            AreaKey::Delete if self.caret < count => {
                let start = self.byte_index(self.caret);
                let end = self.byte_index(self.caret + 1);
                self.text.replace_range(start..end, "");
            }
            AreaKey::Left if self.caret > 0 => self.caret -= 1,
            AreaKey::Right if self.caret < count => self.caret += 1,
            AreaKey::Home => self.caret -= column,
            AreaKey::End => {
                self.caret = self.caret - column + Self::line_length(&chars, self.caret - column);
            }
            AreaKey::Up if line > 0 => {
                let start = Self::line_start(&chars, line - 1);
                self.caret = start + column.min(Self::line_length(&chars, start));
            }
            AreaKey::Down => {
                let last_line = chars.iter().filter(|character| **character == '\n').count();
                if line >= last_line {
                    return false;
                }
                let start = Self::line_start(&chars, line + 1);
                self.caret = start + column.min(Self::line_length(&chars, start));
            }
            _ => return false,
        }
        true
    }

    /// Lines of text for drawing.
    pub fn lines(&self) -> Vec<&str> {
        self.text.split('\n').collect()
    }
}
