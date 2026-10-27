//! Source location tracking and diagnostic formatting.

use std::fmt;

/// Represents a source location span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number.
    pub col: usize,
    /// Byte offset within the source.
    pub offset: usize,
    /// Length in bytes.
    pub len: usize,
}

impl Span {
    /// Creates a new `Span`.
    pub const fn new(line: usize, col: usize, offset: usize, len: usize) -> Self {
        Self {
            line,
            col,
            offset,
            len,
        }
    }
}

/// A value paired with its source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub const fn new(node: T, span: Span) -> Self {
        Self { node, span }
    }
}

/// Formats a diagnostic message with file:line:col, snippet, and caret.
pub fn format_diagnostic(file: &str, source: &str, span: Span, message: &str) -> String {
    let mut out = String::new();
    let line_num = span.line.max(1);
    let col_num = span.col.max(1);

    out.push_str(&format!("{file}:{line_num}:{col_num}: {message}\n"));

    // Extract the line from source
    if let Some(line_content) = source.lines().nth(line_num - 1) {
        // Strip trailing \r if present
        let clean_line = line_content.strip_suffix('\r').unwrap_or(line_content);
        out.push_str(clean_line);
        out.push('\n');

        // Render indent and caret
        let indent = " ".repeat(col_num.saturating_sub(1));
        let carets = if span.len > 1 {
            "^".repeat(
                span.len
                    .min(clean_line.len().saturating_sub(col_num - 1).max(1)),
            )
        } else {
            "^".to_string()
        };
        out.push_str(&indent);
        out.push_str(&carets);
    }

    out
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}
