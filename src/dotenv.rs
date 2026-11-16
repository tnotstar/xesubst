//! Sequential `.env` file evaluation and serialization.

use std::collections::{BTreeMap, HashSet};

use crate::ModeConfig;
use crate::env::VariableTable;
use crate::error::XesubstError;
use crate::eval::eval_word;
use crate::lexer::{SourceCursor, is_ident_continue, is_ident_start};
use crate::parser::parse_dotenv_value;
use crate::span::Span;

/// Supported output formats for `.env` evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DotenvFormat {
    /// Standard dotenv format: KEY="value".
    #[default]
    Dotenv,
    /// Sourceable shell format: export KEY='value'.
    Export,
    /// JSON object format: { "KEY": "value" }.
    Json,
    /// NUL-separated format: KEY=VALUE\0.
    Null,
}

impl std::str::FromStr for DotenvFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "dotenv" => Ok(Self::Dotenv),
            "export" => Ok(Self::Export),
            "json" => Ok(Self::Json),
            "null" => Ok(Self::Null),
            other => Err(format!(
                "unknown format '{other}' (expected dotenv, export, json, null)"
            )),
        }
    }
}

/// Options controlling dotenv parsing and serialization.
#[derive(Debug, Clone, Default)]
pub struct DotenvOptions {
    /// If true, do not overwrite variables already present in the environment.
    pub no_override: bool,
    /// Output serialization format.
    pub format: DotenvFormat,
    /// Only output these specific keys if non-empty.
    pub only: Option<Vec<String>>,
    /// If true, list keys only without values.
    pub keys_only: bool,
}

/// Evaluates `.env` content sequentially, line by line, updating `env` and recording assignments.
pub fn eval_dotenv(
    input: &str,
    env: &mut dyn VariableTable,
    config: &ModeConfig,
    options: &DotenvOptions,
) -> Result<Vec<(String, String)>, XesubstError> {
    let mut initial_keys = HashSet::new();
    if options.no_override {
        for (k, _) in env.all_vars() {
            initial_keys.insert(k);
        }
    }

    let mut assigned_order = Vec::new();
    let mut assigned_map: BTreeMap<String, String> = BTreeMap::new();

    let lines: Vec<&str> = input.split('\n').collect();

    for (line_idx, raw_line) in lines.iter().enumerate() {
        let line_num = line_idx + 1;
        let mut line = *raw_line;

        // Check for CRLF
        if line.ends_with('\r') {
            if config.strict {
                let span = Span::new(line_num, line.len(), 0, 1);
                return Err(XesubstError::CrlfStrict { span });
            }
            line = &line[..line.len() - 1];
            eprintln!(
                "{}:{}: warning: carriage return (CRLF) detected and stripped",
                config.filename, line_num
            );
        }

        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Parse key-value assignment
        let (key, word) = parse_assignment_line(trimmed, line_num, config)?;

        // Expand value expression in the current environment
        let expanded_val = eval_word(&word, env, config)?;

        // Update environment if not prevented by --no-override
        let can_override = !options.no_override || !initial_keys.contains(&key);
        if can_override {
            env.set(&key, &expanded_val);
        }

        if !assigned_map.contains_key(&key) {
            assigned_order.push(key.clone());
        }
        assigned_map.insert(key, expanded_val);
    }

    let results: Vec<(String, String)> = assigned_order
        .into_iter()
        .map(|k| {
            let v = assigned_map.get(&k).unwrap().clone();
            (k, v)
        })
        .collect();

    Ok(results)
}

/// Parses an assignment line: `[export ]KEY=value`
fn parse_assignment_line(
    mut line: &str,
    line_num: usize,
    config: &ModeConfig,
) -> Result<(String, crate::ast::Word), XesubstError> {
    if line.starts_with("export ") {
        line = line["export ".len()..].trim_start();
    }

    let eq_pos = match line.find('=') {
        Some(pos) => pos,
        None => {
            let span = Span::new(line_num, 1, 0, line.len());
            return Err(XesubstError::Syntax {
                span,
                message: format!("invalid assignment on line {line_num}: missing '='"),
            });
        }
    };

    let key = line[..eq_pos].trim();
    if !is_valid_ident(key) {
        let span = Span::new(line_num, 1, 0, key.len());
        if config.strict {
            return Err(XesubstError::StrictViolation {
                span,
                message: format!("invalid variable name '{key}' on line {line_num}"),
            });
        }
        return Err(XesubstError::Syntax {
            span,
            message: format!("invalid variable name '{key}' on line {line_num}"),
        });
    }

    let val_part = line[eq_pos + 1..].trim_start();
    let mut cursor = SourceCursor::new(val_part);
    cursor.line = line_num;
    let word = parse_dotenv_value(&mut cursor, config)?;

    Ok((key.to_string(), word))
}

fn is_valid_ident(name: &str) -> bool {
    let mut chars = name.chars();
    if let Some(first) = chars.next() {
        if !is_ident_start(first) {
            return false;
        }
        for c in chars {
            if !is_ident_continue(c) {
                return false;
            }
        }
        true
    } else {
        false
    }
}

/// Formats evaluated assignments according to the requested DotenvFormat and options.
pub fn format_dotenv_output(
    vars: &[(String, String)],
    options: &DotenvOptions,
) -> Result<String, XesubstError> {
    let filtered_vars: Vec<(&str, &str)> = vars
        .iter()
        .filter(|(k, _)| {
            if let Some(ref only_list) = options.only {
                only_list.contains(k)
            } else {
                true
            }
        })
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    if options.keys_only {
        let mut out = String::new();
        for (k, _) in filtered_vars {
            out.push_str(k);
            out.push('\n');
        }
        return Ok(out);
    }

    match options.format {
        DotenvFormat::Dotenv => {
            let mut out = String::new();
            for (k, v) in filtered_vars {
                out.push_str(k);
                out.push('=');
                out.push_str(&safe_quote_dotenv(v));
                out.push('\n');
            }
            Ok(out)
        }
        DotenvFormat::Export => {
            let mut out = String::new();
            for (k, v) in filtered_vars {
                out.push_str("export ");
                out.push_str(k);
                out.push('=');
                out.push_str(&safe_quote_shell(v));
                out.push('\n');
            }
            Ok(out)
        }
        DotenvFormat::Json => {
            let mut map = serde_json::Map::new();
            for (k, v) in filtered_vars {
                map.insert(k.to_string(), serde_json::Value::String(v.to_string()));
            }
            serde_json::to_string_pretty(&serde_json::Value::Object(map))
                .map_err(|e| XesubstError::Usage(format!("failed to serialize JSON: {e}")))
        }
        DotenvFormat::Null => {
            let mut out = String::new();
            for (k, v) in filtered_vars {
                out.push_str(k);
                out.push('=');
                out.push_str(v);
                out.push('\0');
            }
            Ok(out)
        }
    }
}

/// Safely quotes a value for standard .env files.
pub fn safe_quote_dotenv(val: &str) -> String {
    if val.is_empty() {
        return "\"\"".to_string();
    }

    let needs_quotes = val.contains(' ')
        || val.contains('\t')
        || val.contains('\n')
        || val.contains('\r')
        || val.contains('#')
        || val.contains('"')
        || val.contains('\'')
        || val.contains('$')
        || val.contains('\\')
        || val.contains('&')
        || val.contains(';')
        || val.contains('|');

    if !needs_quotes {
        return val.to_string();
    }

    let mut out = String::from("\"");
    for c in val.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Safely quotes a value for POSIX shell scripts (`export KEY='...'`).
pub fn safe_quote_shell(val: &str) -> String {
    format!("'{}'", val.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::EnvMap;

    #[test]
    fn test_acceptance_fixture_lf_and_crlf() {
        let fixture_lf = r#"RUNTIME_ENVIRON="dev"
BC_TENANT_ID=${BC_TENANT_ID:-"$AZURE_TENANT_ID"}
MSSQL_USERNAME="user"
MSSQL_PASSWORD="secret"
XDATA_ENVIRON=${XDATA_ENVIRON:-"$RUNTIME_ENVIRON"}
XDATA_USERNAME=${MSSQL_USERNAME}
XDATA_HOSTNAME="lnx-msql-${XDATA_ENVIRON}-001.example.global:1433"
XDATA_DATABASE="hic-sdb-xdata-${XDATA_ENVIRON}-001"
OUTPOST_DEBUG=False
UTA_CACHE_DIR="${HOME}/Data/Containers/UTA"
DAGSTER_PG_PASSWORD="abc&def"
LOG_DIR=
LOG_RETENTION_DAYS=30
DEBUGINFOD_URLS=${DEBUGINFOD_URLS:-""}
"#;
        let fixture_crlf = fixture_lf.replace('\n', "\r\n");

        for (desc, content) in [("LF", fixture_lf), ("CRLF", fixture_crlf.as_str())] {
            let mut env = EnvMap::new();
            env.set("HOME", "/home/user");
            let config = ModeConfig::default();
            let options = DotenvOptions::default();

            let results = eval_dotenv(content, &mut env, &config, &options)
                .unwrap_or_else(|e| panic!("failed on {desc}: {e}"));
            let map: BTreeMap<String, String> = results.into_iter().collect();

            assert_eq!(map.get("RUNTIME_ENVIRON").unwrap(), "dev", "{desc}");
            assert_eq!(map.get("BC_TENANT_ID").unwrap(), "", "{desc}");
            assert_eq!(map.get("MSSQL_USERNAME").unwrap(), "user", "{desc}");
            assert_eq!(map.get("MSSQL_PASSWORD").unwrap(), "secret", "{desc}");
            assert_eq!(map.get("XDATA_ENVIRON").unwrap(), "dev", "{desc}");
            assert_eq!(map.get("XDATA_USERNAME").unwrap(), "user", "{desc}");
            assert_eq!(
                map.get("XDATA_HOSTNAME").unwrap(),
                "lnx-msql-dev-001.example.global:1433",
                "{desc}"
            );
            assert_eq!(
                map.get("XDATA_DATABASE").unwrap(),
                "hic-sdb-xdata-dev-001",
                "{desc}"
            );
            assert_eq!(map.get("OUTPOST_DEBUG").unwrap(), "False", "{desc}");
            assert_eq!(
                map.get("UTA_CACHE_DIR").unwrap(),
                "/home/user/Data/Containers/UTA",
                "{desc}"
            );
            assert_eq!(map.get("DAGSTER_PG_PASSWORD").unwrap(), "abc&def", "{desc}");
            assert_eq!(map.get("LOG_DIR").unwrap(), "", "{desc}");
            assert_eq!(map.get("LOG_RETENTION_DAYS").unwrap(), "30", "{desc}");
            assert_eq!(map.get("DEBUGINFOD_URLS").unwrap(), "", "{desc}");
        }
    }
}
