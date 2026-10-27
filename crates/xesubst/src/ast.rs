//! Abstract Syntax Tree (AST) definitions for Bash expansions.

use crate::span::{Span, Spanned};

/// A sequence of word parts forming a word, pattern, or replacement string.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Word {
    pub parts: Vec<Spanned<WordPart>>,
}

impl Word {
    pub fn new(parts: Vec<Spanned<WordPart>>) -> Self {
        Self { parts }
    }

    pub fn from_literal(text: &str, span: Span) -> Self {
        Self {
            parts: vec![Spanned::new(WordPart::Literal(text.to_string()), span)],
        }
    }
}

/// A constituent part of a `Word`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WordPart {
    /// Unquoted literal text.
    Literal(String),
    /// Escaped character, e.g. `\*` or `\"`.
    Escaped(char),
    /// Single-quoted string literal (no expansion, wildcards are literal).
    SingleQuoted(String),
    /// Double-quoted sequence (expands vars, but wildcards inside are literal).
    DoubleQuoted(Vec<Spanned<WordPart>>),
    /// Simple variable expansion `$VAR`.
    SimpleVar(String),
    /// Braced parameter expansion `${...}`.
    Braced(Box<ParamExp>),
    /// Arithmetic expansion `$((...))`.
    Arith(String),
}

/// Complete parameter expansion expression inside `${...}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamExp {
    /// Variable name (or prefix for `${!prefix*}`).
    pub name: String,
    /// Whether indirect expansion was requested (`${!name}`).
    pub indirect: bool,
    /// The specific operation to perform.
    pub op: ParamOp,
}

/// Specific parameter expansion operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamOp {
    /// Plain variable value: `${v}`.
    Value,
    /// Length of value in UTF-8 characters: `${#v}`.
    Length,
    /// Names of all variables starting with prefix: `${!prefix*}` or `${!prefix@}`.
    PrefixNames { star: bool },
    /// Default value testing: `${v:-w}`, `${v-w}`, `${v:=w}`, `${v=w}`, `${v:?w}`, `${v?w}`, `${v:+w}`, `${v+w}`.
    Default {
        colon: bool,
        kind: DefaultKind,
        word: Word,
    },
    /// Substring extraction: `${v:offset}` or `${v:offset:length}`.
    Substring {
        offset: String,
        length: Option<String>,
    },
    /// Pattern prefix/suffix stripping: `${v#p}`, `${v##p}`, `${v%p}`, `${v%%p}`.
    Strip {
        side: StripSide,
        greedy: bool,
        pattern: Word,
    },
    /// Pattern replacement: `${v/p/r}`, `${v//p/r}`, `${v/#p/r}`, `${v/%p/r}`.
    Replace {
        mode: ReplaceMode,
        pattern: Word,
        replacement: Option<Word>,
    },
    /// Case modification: `${v^p}`, `${v^^p}`, `${v,p}`, `${v,,p}`.
    CaseMod {
        upper: bool,
        all: bool,
        pattern: Option<Word>,
    },
    /// Transformation operator: `${v@Q}`, `${v@E}`, `${v@A}`, `${v@U}`, `${v@u}`, `${v@L}`.
    Transform { op: TransformOp },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultKind {
    /// Use default value if unset/null: `:-` / `-`
    UseDefault,
    /// Assign default value to variable if unset/null: `:=` / `=`
    AssignDefault,
    /// Exit with error if unset/null: `:?` / `?`
    ErrorIfUnset,
    /// Use alternative value if set and not null / set: `:+` / `+`
    UseAlternate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripSide {
    /// `#` or `##`
    Prefix,
    /// `%` or `%%`
    Suffix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceMode {
    /// `/pat/rep`: First match
    First,
    /// `//pat/rep`: All matches
    All,
    /// `/#pat/rep`: Anchored at prefix
    Prefix,
    /// `/%pat/rep`: Anchored at suffix
    Suffix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformOp {
    /// `@Q`: Quote value in a format that can be reused as shell input
    Quote,
    /// `@E`: Expand backslash escape sequences
    Escape,
    /// `@A`: Assignment statement or attribute evaluation
    Assignment,
    /// `@U`: Convert all lowercase letters to uppercase
    UpperAll,
    /// `@u`: Convert first character to uppercase
    UpperFirst,
    /// `@L`: Convert all uppercase letters to lowercase
    LowerAll,
}
