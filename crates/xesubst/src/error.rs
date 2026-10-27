//! Error types for expansion and parsing in `xesubst`.

use crate::span::Span;
use thiserror::Error;

/// Core error type representing any failure during parsing or expansion.
#[derive(Debug, Error)]
pub enum XesubstError {
    /// Expansion error (Bash `:?`, `-u` nounset, or `--strict` violation). Exit code 1.
    #[error("{message}")]
    Expansion { span: Span, message: String },

    /// Unterminated syntax structure (e.g. unterminated `${` or quote). Exit code 1.
    #[error("{message}")]
    Syntax { span: Span, message: String },

    /// Arithmetic evaluation error (e.g. divide by zero). Exit code 1.
    #[error("{message}")]
    Arithmetic { span: Span, message: String },

    /// Strict mode error: construct not permitted. Exit code 1.
    #[error("{message}")]
    StrictViolation { span: Span, message: String },

    /// CRLF line endings detected with strict mode active. Exit code 1.
    #[error("CRLF line endings are prohibited in strict mode")]
    CrlfStrict { span: Span },

    /// I/O error when reading or writing files. Exit code 3.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Usage / configuration error. Exit code 2.
    #[error("Usage error: {0}")]
    Usage(String),
}

impl XesubstError {
    /// Returns the recommended exit code for this error.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Expansion { .. }
            | Self::Syntax { .. }
            | Self::Arithmetic { .. }
            | Self::StrictViolation { .. }
            | Self::CrlfStrict { .. } => 1,
            Self::Usage(_) => 2,
            Self::Io(_) => 3,
        }
    }

    /// Returns the source span of the error, if applicable.
    pub fn span(&self) -> Option<Span> {
        match self {
            Self::Expansion { span, .. }
            | Self::Syntax { span, .. }
            | Self::Arithmetic { span, .. }
            | Self::StrictViolation { span, .. }
            | Self::CrlfStrict { span } => Some(*span),
            Self::Io(_) | Self::Usage(_) => None,
        }
    }
}
