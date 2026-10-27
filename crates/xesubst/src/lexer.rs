//! Lexer and token scanner for template streams.

use crate::span::Span;

/// A cursor for scanning a UTF-8 string with line, column, and byte offset tracking.
#[derive(Debug, Clone)]
pub struct SourceCursor<'a> {
    /// Full input string reference.
    pub input: &'a str,
    /// Pre-indexed characters with byte offsets.
    pub chars: Vec<(usize, char)>,
    /// Current character index.
    pub char_idx: usize,
    /// 1-based current line number.
    pub line: usize,
    /// 1-based current column number.
    pub col: usize,
}

impl<'a> SourceCursor<'a> {
    /// Creates a new cursor over `input`.
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            chars: input.char_indices().collect(),
            char_idx: 0,
            line: 1,
            col: 1,
        }
    }

    /// Returns true if at end of input.
    pub fn is_eof(&self) -> bool {
        self.char_idx >= self.chars.len()
    }

    /// Current byte offset in the input string.
    pub fn current_offset(&self) -> usize {
        if self.char_idx < self.chars.len() {
            self.chars[self.char_idx].0
        } else {
            self.input.len()
        }
    }

    /// Creates a span starting at current position with given length.
    pub fn current_span(&self, len: usize) -> Span {
        Span::new(self.line, self.col, self.current_offset(), len)
    }

    /// Peeks at current character.
    pub fn peek(&self) -> Option<char> {
        self.chars.get(self.char_idx).map(|(_, c)| *c)
    }

    /// Peeks `n` characters ahead.
    pub fn peek_ahead(&self, n: usize) -> Option<char> {
        self.chars.get(self.char_idx + n).map(|(_, c)| *c)
    }

    /// Checks if remaining input starts with prefix `s`.
    pub fn starts_with(&self, s: &str) -> bool {
        let offset = self.current_offset();
        self.input[offset..].starts_with(s)
    }

    /// Advances the cursor past prefix string `s`.
    pub fn advance_str(&mut self, s: &str) {
        for _ in 0..s.chars().count() {
            self.next();
        }
    }
}

impl<'a> Iterator for SourceCursor<'a> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        if self.char_idx < self.chars.len() {
            let (_, c) = self.chars[self.char_idx];
            self.char_idx += 1;
            if c == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
            Some(c)
        } else {
            None
        }
    }
}

/// Checks if a character is a valid identifier start: `[A-Za-z_]`.
pub fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

/// Checks if a character is a valid identifier continuation: `[A-Za-z0-9_]`.
pub fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}
