//! Integration tests for receiver mode: `--from-null` / `--exec`.

use std::io::Write;
use std::process::{Command, Stdio};

const XES: &str = env!("CARGO_BIN_EXE_xes");

/// Runs `xes --from-null --exec -- env -0`, feeding `records` on stdin, and
/// returns the child's stdout split into NUL-delimited lines.
fn env0(records: &[u8], extra: &[&str]) -> Vec<String> {
    let mut child = Command::new(XES)
        .arg("--from-null")
        .args(extra)
        .arg("--exec")
        .arg("--")
        .arg("/usr/bin/env")
        .arg("-0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn xes");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(records)
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait xes");
    assert!(
        out.status.success(),
        "xes failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn run_xes(args: &[&str], stdin_data: &[u8]) -> (i32, String, String) {
    let mut child = Command::new(XES)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn xes");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin_data)
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait xes");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn null_records(pairs: &[(&str, &str)]) -> Vec<u8> {
    let mut buf = Vec::new();
    for (k, v) in pairs {
        buf.extend_from_slice(k.as_bytes());
        buf.push(b'=');
        buf.extend_from_slice(v.as_bytes());
        buf.push(0);
    }
    buf
}

#[test]
fn exec_env_sees_values_verbatim() {
    let value = "line one\nline two\twith 'quotes', \"dq\", $dollar `tick` = eq and 🎉";
    let out = env0(&null_records(&[("K", value), ("Z", "z")]), &[]);
    assert!(
        out.iter().any(|l| *l == format!("K={value}")),
        "got {out:?}"
    );
    assert!(out.iter().any(|l| l == "Z=z"));
}

#[test]
fn null_format_round_trip_through_child_environment() {
    // Produce records with the exact `--format null` serializer, consume
    // with `--from-null`: arbitrary values survive byte-for-byte.
    let pairs = vec![
        ("SPACES".to_string(), "a b c".to_string()),
        ("NEWLINE".to_string(), "x\ny".to_string()),
        ("QUOTES".to_string(), "it's \"quoted\"".to_string()),
        ("DOLLAR".to_string(), "$HOME ${NOT_SET} `tick`".to_string()),
        ("EQUALS".to_string(), "a=b=c".to_string()),
        ("UNICODE".to_string(), "café 🎉".to_string()),
    ];
    let options = xesubst::dotenv::DotenvOptions {
        format: xesubst::dotenv::DotenvFormat::Null,
        ..Default::default()
    };
    let records = xesubst::dotenv::format_dotenv_output(&pairs, &options).unwrap();

    let out = env0(records.as_bytes(), &[]);
    for (k, v) in &pairs {
        let pair = format!("{k}={v}");
        assert!(out.contains(&pair), "missing {pair:?} in {out:?}");
    }
}

#[test]
fn only_filters_keys() {
    let out = env0(
        &null_records(&[("A", "1"), ("B", "2"), ("C", "3")]),
        &["--only", "B"],
    );
    assert!(out.iter().any(|l| l == "B=2"));
    assert!(!out.iter().any(|l| l.starts_with("A=")));
    assert!(!out.iter().any(|l| l.starts_with("C=")));
}

#[test]
fn no_override_respects_existing_environment() {
    let mut cmd = Command::new(XES)
        .args(["--from-null", "--no-override", "--exec", "--", "env", "-0"])
        .env("PREEXISTING", "kept")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    cmd.stdin
        .as_mut()
        .unwrap()
        .write_all(&null_records(&[
            ("PREEXISTING", "clobbered"),
            ("FRESH", "new"),
        ]))
        .unwrap();
    let out = cmd.wait_with_output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("PREEXISTING=kept"), "{text:?}");
    assert!(text.contains("FRESH=new"));
}

#[test]
fn deny_by_default_accept_by_glob_escape_hatch() {
    let records = null_records(&[("LD_PRELOAD", "/evil"), ("PATH", "/evil")]);

    let (code, _, stderr) = run_xes(&["--from-null", "--exec", "--", "env"], &records);
    assert_eq!(code, 2);
    assert!(stderr.contains("LD_PRELOAD"), "{stderr}");
    assert!(
        !stderr.contains("/evil"),
        "diagnostic leaked a value: {stderr}"
    );

    // Accepted names still land in the child environment; an absolute path
    // keeps PATH overriding from breaking command resolution, and empty
    // LD_PRELOAD keeps the loader quiet.
    let out = env0(
        &null_records(&[("LD_PRELOAD", ""), ("PATH", "/nonexistent")]),
        &["--accept", "LD_*", "--accept", "PATH"],
    );
    assert!(out.iter().any(|l| l == "LD_PRELOAD="), "{out:?}");
    assert!(out.iter().any(|l| l == "PATH=/nonexistent"), "{out:?}");

    let out = env0(
        &null_records(&[("LD_PRELOAD", ""), ("PATH", "/nonexistent")]),
        &["--allow-unsafe-names"],
    );
    assert!(out.iter().any(|l| l == "LD_PRELOAD="), "{out:?}");
}

#[test]
fn dotenv_exec_denies_only_when_accept_given() {
    // -d --exec: denylist off by default (local evaluation is trusted)...
    let (code, _, stderr) = run_xes(
        &["-d", "--exec", "--", "true"],
        b"LD_LIBRARY_PATH=/opt/lib\nOK=1\n",
    );
    assert_eq!(code, 0, "{stderr}");

    // ...but on when --accept narrows it.
    let (code, _, stderr) = run_xes(
        &["-d", "--accept", "OK", "--exec", "--", "true"],
        b"LD_LIBRARY_PATH=/opt/lib\nOK=1\n",
    );
    assert_eq!(code, 2);
    assert!(stderr.contains("LD_LIBRARY_PATH"), "{stderr}");
    assert!(
        !stderr.contains("/opt/lib"),
        "diagnostic leaked a value: {stderr}"
    );
}

#[test]
fn malformed_input_rejected_with_index_and_name_only() {
    for (records, expected) in [
        (b"no-equal-sign\0".to_vec(), "missing '='"),
        (b"=value\0".to_vec(), "empty variable name"),
        (b"1BAD=value\0".to_vec(), "invalid variable name"),
    ] {
        let (code, _, stderr) = run_xes(&["--from-null", "--exec", "--", "true"], &records);
        assert_eq!(code, 2, "{stderr}");
        assert!(
            stderr.contains(expected),
            "want {expected:?}, got {stderr:?}"
        );
        assert!(
            !stderr.contains("value"),
            "diagnostic leaked a value: {stderr}"
        );
    }

    // Invalid UTF-8 in the value.
    let mut bad = b"K=\xff\xfe\0".to_vec();
    let (code, _, stderr) = run_xes(&["--from-null", "--exec", "--", "true"], &bad);
    assert_eq!(code, 2, "{stderr}");
    bad = b"\xffK=x\0".to_vec();
    let (code, _, _) = run_xes(&["--from-null", "--exec", "--", "true"], &bad);
    assert_eq!(code, 2);
}

#[test]
fn size_limit_enforced_and_overridable() {
    let mut big = vec![b'K'; 4];
    big.push(b'=');
    big.extend(std::iter::repeat_n(b'v', 1024));
    big.push(0);

    let (code, _, stderr) = run_xes(
        &["--from-null", "--max-input", "100", "--exec", "--", "true"],
        &big,
    );
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("exceeding"), "{stderr}");

    let (code, _, _) = run_xes(
        &["--from-null", "--max-input", "2000", "--exec", "--", "true"],
        &big,
    );
    assert_eq!(code, 0);
}

#[test]
fn exec_exit_codes_126_and_127() {
    let good = null_records(&[("A", "1")]);
    let (code, _, _) = run_xes(
        &["--from-null", "--exec", "--", "xes-definitely-missing"],
        &good,
    );
    assert_eq!(code, 127);
    let (code, _, _) = run_xes(&["--from-null", "--exec", "--", "/etc/hostname"], &good);
    assert_eq!(code, 126);
}

#[test]
fn usage_conflicts_rejected() {
    let empty: &[u8] = b"";
    for args in [
        vec!["--from-null", "-d", "--exec", "--", "true"],
        vec!["--from-null", "--arith", "--exec", "--", "true"],
        vec!["--from-null", "-i", "--exec", "--", "true"],
        vec!["--from-null"],
        vec!["--exec", "--"],
        vec!["--exec", "--", "true"],
    ] {
        let (code, _, stderr) = run_xes(&args, empty);
        assert_eq!(code, 2, "args {args:?}: {stderr}");
    }
}

mod prop {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Any value round-trips byte-for-byte through `--format null`
        /// records into the child environment, with no re-expansion.
        #[test]
        fn value_round_trip(value in "\\PC*") {
            let records = null_records(&[("K", &value)]);
            let out = env0(&records, &[]);
            prop_assert!(out.iter().any(|l| *l == format!("K={value}")), "got {out:?}");
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::*;

    #[test]
    fn from_null_reads_from_file() {
        let mut path = std::env::temp_dir();
        path.push(format!("xes-test-{}.null", std::process::id()));
        std::fs::write(&path, null_records(&[("FROMFILE", "yes")])).unwrap();
        let out = Command::new(XES)
            .arg("--from-null")
            .arg(&path)
            .arg("--exec")
            .arg("--")
            .arg("printenv")
            .arg("FROMFILE")
            .output()
            .unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "yes");
    }
}
