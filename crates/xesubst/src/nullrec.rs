//! NUL-delimited `KEY=VALUE` record parsing and the receiver-side name policy.
//!
//! The records are the exact output of `xes --format null`. Values are raw
//! data: they are never expanded, unquoted, or otherwise interpreted.

use crate::error::XesubstError;
use crate::glob::{Pattern, parse_pattern_from_str};

/// Default maximum accepted input size for NUL-delimited records (1 MiB).
pub const DEFAULT_MAX_INPUT: usize = 1024 * 1024;

/// Variable names refused by the default receiver policy: they can subvert
/// the dynamic loader, the interpreter, or the shell that runs the target
/// process. Exact names and prefixes are listed separately.
const UNSAFE_EXACT: &[&str] = &[
    // Shell behavior
    "PATH",
    "IFS",
    "ENV",
    "BASH_ENV",
    "SHELLOPTS",
    "BASHOPTS",
    "PS4",
    "PROMPT_COMMAND",
    "SHELL",
    "CDPATH",
    "HOME",
    // Dynamic loaders (glibc, macOS, AIX, HP-UX)
    "GLIBC_TUNABLES",
    "LIBPATH",
    "SHLIB_PATH",
    "KRB5RCNAME",
    // Interpreters
    "PERL5OPT",
    "PERLLIB",
    "PYTHONPATH",
    "PYTHONHOME",
    "PYTHONSTARTUP",
    "RUBYOPT",
    "RUBYLIB",
    "GEM_HOME",
    "GEM_PATH",
    "NODE_OPTIONS",
    "JAVA_TOOL_OPTIONS",
];

const UNSAFE_PREFIX: &[&str] = &[
    "LD_",
    "DYLD_",
    "LDR_",
    "MALLOC_",
    // Exported bash functions are executable code, not data.
    "BASH_FUNC_",
];

/// Returns true if `name` is refused by the default receiver policy.
pub fn is_unsafe_name(name: &str) -> bool {
    UNSAFE_EXACT.contains(&name) || UNSAFE_PREFIX.iter().any(|p| name.starts_with(p))
}

/// Returns true if `name` is a valid environment variable name
/// (`[A-Za-z_][A-Za-z0-9_]*`).
pub fn is_valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    match bytes.first() {
        Some(&b) if b == b'_' || b.is_ascii_alphabetic() => {}
        _ => return false,
    }
    bytes[1..]
        .iter()
        .all(|&b| b == b'_' || b.is_ascii_alphanumeric())
}

/// Name policy applied to untrusted `KEY=VALUE` records.
///
/// Semantics (fail closed):
/// - When `denylist` is active, unsafe names are always refused; only the
///   caller (via `--allow-unsafe-names`, which constructs the policy with
///   `denylist: false`) can lift it. `accept` never overrides the denylist.
/// - When `accept` is non-empty it is a restrictive allowlist: a name that
///   matches no glob is refused, not silently dropped.
#[derive(Debug, Clone, Default)]
pub struct NamePolicy {
    /// Restrictive allowlist globs (repeatable `--accept`); empty means all
    /// (otherwise permitted) names pass.
    pub accept: Vec<Pattern>,
    /// Whether the unsafe-name denylist is active.
    pub denylist: bool,
}

impl NamePolicy {
    /// Builds a policy from raw `--accept` glob strings and whether the
    /// unsafe-name denylist is active (`--allow-unsafe-names` disables it).
    pub fn new(accept_globs: &[String], denylist: bool) -> Self {
        Self {
            accept: accept_globs
                .iter()
                .map(|g| parse_pattern_from_str(g))
                .collect(),
            denylist,
        }
    }

    /// Returns true if `name` is permitted under this policy.
    pub fn permits(&self, name: &str) -> bool {
        if self.denylist && is_unsafe_name(name) {
            return false;
        }
        if self.accept.is_empty() {
            return true;
        }
        let chars: Vec<char> = name.chars().collect();
        self.accept.iter().any(|p| p.matches_all(&chars))
    }
}

/// Parses NUL-delimited `KEY=VALUE` records (the exact output of
/// `--format null`). Every record, including the last, must be terminated
/// by NUL; a final unterminated record is rejected as truncated. Duplicate
/// keys are rejected. Values are raw bytes carried through verbatim.
/// Diagnostics mention only the record index and the variable name, never
/// a value.
pub fn parse_null_records(
    input: &[u8],
    max_input: usize,
) -> Result<Vec<(String, String)>, XesubstError> {
    if input.len() > max_input {
        return Err(XesubstError::NullRecord {
            index: 0,
            name: None,
            message: format!(
                "input is {} bytes, exceeding the {} byte limit (see --max-input)",
                input.len(),
                max_input
            ),
        });
    }

    let mut pairs = Vec::new();
    if input.is_empty() {
        return Ok(pairs);
    }

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut start = 0usize;
    let mut index = 0usize;
    for pos in 0..input.len() {
        if input[pos] != 0 {
            continue;
        }
        let record = parse_record(&input[start..pos], index)?;
        if !seen.insert(record.0.clone()) {
            let name = record.0;
            return Err(XesubstError::NullRecord {
                index,
                name: Some(name.clone()),
                message: format!("duplicate variable name '{name}'"),
            });
        }
        pairs.push(record);
        index += 1;
        start = pos + 1;
    }
    if start < input.len() {
        return Err(XesubstError::NullRecord {
            index,
            name: None,
            message: "final record is truncated (missing NUL terminator)".to_string(),
        });
    }

    Ok(pairs)
}

/// Parses one raw record; `index` is its zero-based position for diagnostics.
fn parse_record(record: &[u8], index: usize) -> Result<(String, String), XesubstError> {
    let eq = match record.iter().position(|&b| b == b'=') {
        Some(pos) => pos,
        None => {
            return Err(XesubstError::NullRecord {
                index,
                name: None,
                message: "missing '=' separator".to_string(),
            });
        }
    };

    let key = match std::str::from_utf8(&record[..eq]) {
        Ok(k) => k,
        Err(_) => {
            return Err(XesubstError::NullRecord {
                index,
                name: None,
                message: "key is not valid UTF-8".to_string(),
            });
        }
    };

    if key.is_empty() {
        return Err(XesubstError::NullRecord {
            index,
            name: None,
            message: "empty variable name".to_string(),
        });
    }
    if !is_valid_name(key) {
        return Err(XesubstError::NullRecord {
            index,
            name: Some(key.to_string()),
            message: format!("invalid variable name '{key}'"),
        });
    }

    let value = match std::str::from_utf8(&record[eq + 1..]) {
        Ok(v) => v,
        Err(_) => {
            return Err(XesubstError::NullRecord {
                index,
                name: Some(key.to_string()),
                message: format!("value of '{key}' is not valid UTF-8"),
            });
        }
    };

    Ok((key.to_string(), value.to_string()))
}

/// Applies `policy` to parsed records, in order. The first refusal is
/// reported with its record index and name only.
pub fn check_name_policy(
    pairs: &[(String, String)],
    policy: &NamePolicy,
) -> Result<(), XesubstError> {
    for (index, (name, _)) in pairs.iter().enumerate() {
        if !policy.permits(name) {
            return Err(XesubstError::UnsafeName {
                index,
                name: name.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &[u8]) -> Vec<(String, String)> {
        parse_null_records(input, DEFAULT_MAX_INPUT).unwrap()
    }

    #[test]
    fn parses_simple_records() {
        assert_eq!(
            parse(b"A=1\0B=two words\0"),
            vec![("A".into(), "1".into()), ("B".into(), "two words".into())]
        );
    }

    #[test]
    fn empty_input_is_ok_but_bare_nul_is_a_bad_record() {
        assert!(parse(b"").is_empty());
        // A lone NUL is an empty (missing '=') record, not valid input.
        assert!(parse_null_records(b"\0", DEFAULT_MAX_INPUT).is_err());
    }

    #[test]
    fn values_are_verbatim() {
        let value = "a b\nc\t\"q\" $dollar `tick` = and emoji 🎉";
        assert_eq!(parse(format!("K={value}\0").as_bytes())[0].1, value);
    }

    #[test]
    fn missing_separator_is_rejected() {
        let err = parse_null_records(b"NOSEPARATOR\0", DEFAULT_MAX_INPUT).unwrap_err();
        assert!(err.to_string().contains("missing '='"), "{err}");
        assert!(err.to_string().contains("record 0"), "{err}");
    }

    #[test]
    fn empty_key_is_rejected() {
        assert!(
            parse_null_records(b"=v\0", DEFAULT_MAX_INPUT)
                .unwrap_err()
                .to_string()
                .contains("empty variable name")
        );
    }

    #[test]
    fn invalid_name_is_rejected() {
        let err = parse_null_records(b"1BAD=v\0", DEFAULT_MAX_INPUT).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("invalid variable name"), "{msg}");
        assert!(!msg.contains("=v"), "diagnostic must not contain the value");
    }

    #[test]
    fn name_char_class_is_enforced() {
        assert!(is_valid_name("_private1"));
        assert!(is_valid_name("a"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("1a"));
        assert!(!is_valid_name("a-b"));
        assert!(!is_valid_name("a.b"));
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        assert!(parse_null_records(b"K=\xff\0", DEFAULT_MAX_INPUT).is_err());
        assert!(parse_null_records(b"\xffK=v\0", DEFAULT_MAX_INPUT).is_err());
    }

    #[test]
    fn size_limit_is_enforced() {
        assert!(parse_null_records(b"K=v\0", 3).is_err());
        assert!(parse_null_records(b"K=v\0", 4).is_ok());
    }

    #[test]
    fn policy_denies_loader_and_shell_names() {
        for name in [
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
            "PATH",
            "IFS",
            "BASH_ENV",
        ] {
            assert!(is_unsafe_name(name), "{name}");
        }
        assert!(!is_unsafe_name("DATABASE_URL"));
    }

    #[test]
    fn policy_accept_is_restrictive_and_never_lifts_denylist() {
        let denied = vec![("LD_LIBRARY_PATH".to_string(), String::new())];
        let allowed = vec![("DATABASE_URL".to_string(), String::new())];

        // Receiver default: denylist active, no allowlist.
        let strict = NamePolicy::new(&[], true);
        assert!(check_name_policy(&denied, &strict).is_err());
        assert!(check_name_policy(&allowed, &strict).is_ok());

        // --accept alone never lifts the denylist...
        let accept = NamePolicy::new(&["LD_LIBRARY_*".to_string()], true);
        assert!(check_name_policy(&denied, &accept).is_err());
        // ...but it restricts: a safe name matching no glob is refused.
        assert!(check_name_policy(&allowed, &accept).is_err());
        let narrow = NamePolicy::new(&["DATABASE_*".to_string()], true);
        assert!(check_name_policy(&allowed, &narrow).is_ok());

        // --allow-unsafe-names lifts the denylist; --accept still narrows.
        let hatch = NamePolicy::new(&[], false);
        assert!(check_name_policy(&denied, &hatch).is_ok());
        let hatch_narrow = NamePolicy::new(&["LD_LIBRARY_*".to_string()], false);
        assert!(check_name_policy(&denied, &hatch_narrow).is_ok());
        assert!(check_name_policy(&allowed, &hatch_narrow).is_err());
    }

    #[test]
    fn truncated_final_record_is_rejected() {
        let err = parse_null_records(b"A=1\0B=2", DEFAULT_MAX_INPUT).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("truncated"), "{msg}");
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        let err = parse_null_records(b"A=1\0B=2\0A=3\0", DEFAULT_MAX_INPUT).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("duplicate variable name 'A'"), "{msg}");
        assert!(msg.contains("record 2"), "{msg}");
        assert!(!msg.contains("=3"), "diagnostic must not contain the value");
    }
}
