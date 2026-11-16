//! Acceptance fixture test comparing xesubst --dotenv against Bash source.

use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

const FIXTURE_CONTENT: &str = r#"RUNTIME_ENVIRON="dev"
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

fn run_bash_fixture(content: &str) -> BTreeMap<String, String> {
    let script = format!(
        r#"
temp_file=$(mktemp)
cat << 'EOF' > "$temp_file"
{content}
EOF
set -a
source "$temp_file"
rm -f "$temp_file"
env -0
"#
    );

    let output = Command::new("env")
        .arg("-i")
        .arg("HOME=/home/user")
        .arg("bash")
        .arg("-c")
        .arg(&script)
        .output()
        .expect("failed to run bash");

    assert!(output.status.success());
    parse_nul_env(&output.stdout)
}

fn run_xes_fixture(content: &str) -> BTreeMap<String, String> {
    let bin_path = env!("CARGO_BIN_EXE_xes");
    let mut cmd = Command::new("env");
    cmd.arg("-i")
        .arg("HOME=/home/user")
        .arg(bin_path)
        .arg("--dotenv")
        .arg("--format")
        .arg("null")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn xes");
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(content.as_bytes()).unwrap();
    }
    let output = child.wait_with_output().expect("xes failed");
    assert!(
        output.status.success(),
        "xes failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_nul_env(&output.stdout)
}

fn parse_nul_env(bytes: &[u8]) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for entry in bytes.split(|&b| b == 0) {
        if entry.is_empty() {
            continue;
        }
        if let Some(pos) = entry.iter().position(|&b| b == b'=') {
            let key = String::from_utf8_lossy(&entry[..pos]).to_string();
            let val = String::from_utf8_lossy(&entry[pos + 1..]).to_string();
            map.insert(key, val);
        }
    }
    map
}

#[test]
fn test_acceptance_fixture_e2e_lf_and_crlf() {
    let fixture_lf = FIXTURE_CONTENT.to_string();
    let fixture_crlf = FIXTURE_CONTENT.replace('\n', "\r\n");

    let expected_keys = [
        "RUNTIME_ENVIRON",
        "BC_TENANT_ID",
        "MSSQL_USERNAME",
        "MSSQL_PASSWORD",
        "XDATA_ENVIRON",
        "XDATA_USERNAME",
        "XDATA_HOSTNAME",
        "XDATA_DATABASE",
        "OUTPOST_DEBUG",
        "UTA_CACHE_DIR",
        "DAGSTER_PG_PASSWORD",
        "LOG_DIR",
        "LOG_RETENTION_DAYS",
        "DEBUGINFOD_URLS",
    ];

    // Canonical ground truth from Bash on standard LF fixture
    let bash_map = run_bash_fixture(&fixture_lf);

    for (label, content) in [("LF", &fixture_lf), ("CRLF", &fixture_crlf)] {
        let xes_map = run_xes_fixture(content);

        for key in &expected_keys {
            let expected_val = bash_map
                .get(*key)
                .unwrap_or_else(|| panic!("Bash missing {key}"));
            let actual_val = xes_map
                .get(*key)
                .unwrap_or_else(|| panic!("xes missing {key} in {label}"));
            assert_eq!(
                actual_val, expected_val,
                "Value mismatch for key '{key}' on line ending {label}"
            );
        }
    }
}
