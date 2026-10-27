//! Compatibility tests comparing `xesubst --compat` against GNU `envsubst`.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_gnu_envsubst(env: &[(&str, &str)], args: &[&str], input: &str) -> Option<String> {
    let mut cmd = Command::new("envsubst");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.args(args);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let output = child.wait_with_output().ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

fn run_xes_compat(env: &[(&str, &str)], args: &[&str], input: &str) -> String {
    let bin_path = env!("CARGO_BIN_EXE_xes");
    let mut cmd = Command::new(bin_path);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.arg("--compat");
    cmd.args(args);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn xes");
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes()).unwrap();
    }
    let output = child.wait_with_output().expect("xes failed");
    assert!(
        output.status.success(),
        "xes failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn assert_matches_envsubst(env: &[(&str, &str)], args: &[&str], input: &str) {
    let gnu_out = match run_gnu_envsubst(env, args, input) {
        Some(out) => out,
        None => {
            eprintln!("Skipping: GNU envsubst not found on system");
            return;
        }
    };
    let xes_out = run_xes_compat(env, args, input);
    assert_eq!(
        xes_out, gnu_out,
        "Mismatch for input '{input}' with args {args:?}"
    );
}

#[test]
fn test_compat_plain_substitution() {
    let env = [("USER", "alice"), ("HOST", "example.com")];
    assert_matches_envsubst(&env, &[], "User: $USER on ${HOST} ($UNSET)\n");
}

#[test]
fn test_compat_unexpanded_bash_extensions() {
    let env = [("VAR", "value")];
    // In GNU envsubst, ${VAR:-x} and ${VAR#prefix} are NOT expanded
    assert_matches_envsubst(&env, &[], "Default: ${VAR:-default} Suffix: ${VAR%lue}\n");
}

#[test]
fn test_compat_shell_format_filtering() {
    let env = [("A", "alpha"), ("B", "beta"), ("C", "gamma")];
    assert_matches_envsubst(&env, &["$A ${C}"], "A=$A B=$B C=$C\n");
}

#[test]
fn test_compat_variables_flag() {
    let bin_path = env!("CARGO_BIN_EXE_xes");
    let format_arg = "$FOO and ${BAR} and $BAZ";

    let gnu_output = Command::new("envsubst")
        .arg("-v")
        .arg(format_arg)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_else(|_| "FOO\nBAR\nBAZ\n".to_string());

    let xes_output = Command::new(bin_path)
        .arg("-v")
        .arg(format_arg)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .expect("xes failed");

    assert_eq!(xes_output, gnu_output);
}
