//! # xesubst
//!
//! A high-performance, robust, Bash-compatible parameter expansion and template substitution library in Rust.
//!
//! ## Features
//! - GNU `envsubst` drop-in compatibility.
//! - Full Bash parameter expansion (`${VAR:-default}`, `${VAR#prefix}`, `${VAR//pat/rep}`, etc.).
//! - Arithmetic expansion (`$(( ... ))`).
//! - Sequential `.env` file evaluation without code execution.
//!
//! ## Example
//! ```rust
//! use xesubst::env::{EnvMap, VariableTable};
//! use xesubst::eval::expand_str;
//! use xesubst::ModeConfig;
//!
//! let mut env = EnvMap::new();
//! env.set("USER", "alice");
//!
//! let config = ModeConfig::default();
//! let output = expand_str("Hello, ${USER:-guest}!", &mut env, &config).unwrap();
//! assert_eq!(output, "Hello, alice!");
//! ```

#![deny(clippy::all)]

pub mod arith;
pub mod ast;
pub mod dotenv;
pub mod env;
pub mod error;
pub mod eval;
pub mod glob;
pub mod lexer;
pub mod parser;
pub mod span;

pub use env::{EnvMap, OverlayEnv, VariableTable};
pub use error::XesubstError;
pub use span::Span;

/// Configuration options for the expansion engine.
#[derive(Debug, Clone)]
pub struct ModeConfig {
    /// If true, replicate strict GNU envsubst behavior (disabling all Bash extensions).
    pub compat: bool,
    /// If non-empty, only substitute variables listed here. All others are left untouched.
    pub shell_format_vars: Option<Vec<String>>,
    /// If true, fail when referencing an unset variable (`set -u`).
    pub nounset: bool,
    /// If true, reject constructs like $(...), $1, $$, arrays, and CRLF line endings.
    pub strict: bool,
    /// If true, enable arithmetic expansions `$(( ... ))`.
    pub arith: bool,
    /// Current filename for diagnostics.
    pub filename: String,
}

impl Default for ModeConfig {
    fn default() -> Self {
        Self {
            compat: false,
            shell_format_vars: None,
            nounset: false,
            strict: false,
            arith: false,
            filename: "<stdin>".to_string(),
        }
    }
}
