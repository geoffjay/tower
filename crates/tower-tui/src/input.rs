//! Single-line text input (palette, prompt, replies, filters).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Default, Clone)]
pub struct LineInput {
    text: String,
    /// Cursor position in chars.
    cursor: usize,
}

/// What a key did to the input.
#[derive(Debug, PartialEq, Eq)]
pub enum Edit {
    Changed,
    Submit(String),
    Cancel,
    Ignored,
}

impl LineInput {
    pub fn with(text: &str) -> Self {
        Self {
            text: text.to_string(),
            cursor: text.chars().count(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map_or(self.text.len(), |(b, _)| b)
    }

    pub fn key(&mut self, k: KeyEvent) -> Edit {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Enter => Edit::Submit(std::mem::take(&mut self.text)),
            KeyCode::Esc => Edit::Cancel,
            KeyCode::Char('c') if ctrl => Edit::Cancel,
            KeyCode::Char('u') if ctrl => {
                self.text.drain(..self.byte_at(self.cursor));
                self.cursor = 0;
                Edit::Changed
            }
            KeyCode::Char('a') if ctrl => {
                self.cursor = 0;
                Edit::Changed
            }
            KeyCode::Char('e') if ctrl => {
                self.cursor = self.text.chars().count();
                Edit::Changed
            }
            KeyCode::Char('w') if ctrl => {
                let end = self.byte_at(self.cursor);
                let head = &self.text[..end];
                let start = head.trim_end().rfind(' ').map_or(0, |i| i + 1);
                let removed = self.text[start..end].chars().count();
                self.text.drain(start..end);
                self.cursor -= removed;
                Edit::Changed
            }
            KeyCode::Char(c) if !ctrl => {
                let at = self.byte_at(self.cursor);
                self.text.insert(at, c);
                self.cursor += 1;
                Edit::Changed
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let at = self.byte_at(self.cursor - 1);
                self.text.remove(at);
                self.cursor -= 1;
                Edit::Changed
            }
            KeyCode::Delete if self.cursor < self.text.chars().count() => {
                let at = self.byte_at(self.cursor);
                self.text.remove(at);
                Edit::Changed
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                Edit::Changed
            }
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.text.chars().count());
                Edit::Changed
            }
            KeyCode::Home => {
                self.cursor = 0;
                Edit::Changed
            }
            KeyCode::End => {
                self.cursor = self.text.chars().count();
                Edit::Changed
            }
            _ => Edit::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn edits_in_the_middle_of_multibyte_text() {
        let mut i = LineInput::with("héllo");
        i.key(k(KeyCode::Left));
        i.key(k(KeyCode::Left));
        i.key(k(KeyCode::Backspace)); // removes 'l' before the cursor
        i.key(k(KeyCode::Char('✓')));
        assert_eq!(i.text(), "hé✓lo");
        assert_eq!(i.key(k(KeyCode::Enter)), Edit::Submit("hé✓lo".into()));
    }

    #[test]
    fn ctrl_w_deletes_the_previous_word() {
        let mut i = LineInput::with("prompt backend hi there");
        i.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(i.text(), "prompt backend hi ");
        assert_eq!(i.cursor(), 18);
    }
}
