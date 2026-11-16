//! Differential test harness comparing `xesubst` against real GNU `bash`.

use std::process::Command;

fn run_bash(env_vars: &[(&str, &str)], script: &str) -> Option<String> {
    let mut cmd = Command::new("bash");
    cmd.arg("-c").arg(script);
    for (k, v) in env_vars {
        cmd.env(k, v);
    }
    let output = cmd.output().ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

fn run_xes(env_vars: &[(&str, &str)], template: &str) -> String {
    let bin_path = env!("CARGO_BIN_EXE_xes");
    let mut cmd = Command::new(bin_path);
    for (k, v) in env_vars {
        cmd.env(k, v);
    }
    use std::io::Write;
    use std::process::Stdio;
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn xes");
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(template.as_bytes()).unwrap();
    }
    let output = child.wait_with_output().expect("xes failed");
    assert!(
        output.status.success(),
        "xes failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn assert_matches_bash(env_vars: &[(&str, &str)], template: &str) {
    let bash_script = format!("printf \"%s\" \"{template}\"");
    let bash_out = match run_bash(env_vars, &bash_script) {
        Some(out) => out,
        None => {
            eprintln!("Skipping differential test: bash not available or command failed");
            return;
        }
    };
    let xes_out = run_xes(env_vars, template);
    assert_eq!(
        xes_out, bash_out,
        "Mismatch for template '{template}' with env {env_vars:?}"
    );
}

#[test]
fn test_differential_defaults() {
    assert_matches_bash(&[("V", "")], "${V:-fallback}");
    assert_matches_bash(&[("V", "")], "${V-fallback}");
    assert_matches_bash(&[], "${V:-fallback}");
    assert_matches_bash(&[], "${V-fallback}");
    assert_matches_bash(&[("V", "hello")], "${V:-fallback}");
    assert_matches_bash(&[("V", "hello")], "${V-fallback}");
    assert_matches_bash(&[("V", "hello")], "${V:+alt}");
    assert_matches_bash(&[("V", "")], "${V:+alt}");
    assert_matches_bash(&[("V", "")], "${V+alt}");
}

#[test]
fn test_differential_substrings() {
    let env = [("S", "abcdefghijklmnopqrstuvwxyz")];
    assert_matches_bash(&env, "${S:5}");
    assert_matches_bash(&env, "${S:5:4}");
    assert_matches_bash(&env, "${S: -5}");
    assert_matches_bash(&env, "${S:(-5)}");
    assert_matches_bash(&env, "${S:5:-5}");
    assert_matches_bash(&env, "${S:1+2:2*3}");
}

#[test]
fn test_differential_stripping() {
    let env = [("P", "/home/user/workspace/repo/file.txt")];
    assert_matches_bash(&env, "${P#*/}");
    assert_matches_bash(&env, "${P##*/}");
    assert_matches_bash(&env, "${P%/*}");
    assert_matches_bash(&env, "${P%%/*}");
}

#[test]
fn test_differential_replacements() {
    let env = [("MSG", "apple banana cherry banana date")];
    assert_matches_bash(&env, "${MSG/banana/orange}");
    assert_matches_bash(&env, "${MSG//banana/orange}");
    assert_matches_bash(&env, "${MSG/#apple/red-apple}");
    assert_matches_bash(&env, "${MSG/%date/end-date}");
}

#[test]
fn test_differential_case_mod() {
    let env = [("NAME", "fooBARbaz")];
    assert_matches_bash(&env, "${NAME^}");
    assert_matches_bash(&env, "${NAME^^}");
    assert_matches_bash(&env, "${NAME,}");
    assert_matches_bash(&env, "${NAME,,}");
}

#[test]
fn test_differential_transforms() {
    let env = [(
        "T",
        "test string with 'single quotes' and \"double quotes\"",
    )];
    assert_matches_bash(&env, "${T@Q}");
    assert_matches_bash(&env, "${T@U}");
    assert_matches_bash(&env, "${T@u}");
    assert_matches_bash(&env, "${T@L}");
}

#[test]
fn test_differential_nested_quoting() {
    let env = [("AZURE_TENANT_ID", "corp-azure-999")];
    assert_matches_bash(&env, "${BC_TENANT_ID:-\"$AZURE_TENANT_ID\"}");
    assert_matches_bash(&env, "${A:-${B:-$AZURE_TENANT_ID}}");
}
