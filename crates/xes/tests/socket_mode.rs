//! Integration tests for `--from-socket` (receiver side of `--ssh`),
//! exercised against local Unix sockets without ssh.

#![cfg(unix)]

use std::io::Write;
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};
use std::thread;

const XES: &str = env!("CARGO_BIN_EXE_xes");

fn tmp_socket(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("xes-sock-{}-{name}.sock", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

/// Spawns `xes --from-socket SOCK --exec -- env -0` plus `extra` args and
/// returns the running child (write the payload to the socket, then wait).
fn spawn_receiver(sock: &std::path::Path, extra: &[&str], cmd: &[&str]) -> std::process::Child {
    Command::new(XES)
        .arg("--from-socket")
        .arg(sock)
        .args(extra)
        .arg("--exec")
        .arg("--")
        .args(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn xes receiver")
}

/// Serves `payload` to exactly one connection on `sock`.
fn serve_once(sock: &std::path::Path, payload: &[u8]) {
    let listener = UnixListener::bind(sock).expect("bind");
    let payload = payload.to_vec();
    thread::spawn(move || {
        if let Ok((mut conn, _)) = listener.accept() {
            let _ = conn.write_all(&payload);
            let _ = conn.flush();
            drop(conn);
        }
    });
    // Give the receiver a moment to connect and the server to finish.
    thread::sleep(std::time::Duration::from_millis(200));
}

#[test]
fn socket_payload_reaches_child_environment() {
    let sock = tmp_socket("happy");
    let value = "multi\nline $notexpanded 'quoted'";
    let payload = format!("GOOD={value}\0OTHER=x y\0");

    // Start the sender first: the receiver connects, then we serve.
    let listener = UnixListener::bind(&sock).unwrap();
    let child = spawn_receiver(&sock, &[], &["/usr/bin/env", "-0"]);
    let (mut conn, _) = listener.accept().unwrap();
    conn.write_all(payload.as_bytes()).unwrap();
    conn.flush().unwrap();
    drop(conn);

    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains(&format!("GOOD={value}")), "{text:?}");
    assert!(text.contains("OTHER=x y"));

    // Receiver must unlink the socket after connecting.
    assert!(!sock.exists(), "socket was not unlinked");
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn socket_fail_closed_on_empty_and_truncated_payloads() {
    for (name, payload) in [("empty", ""), ("truncated", "A=1\0B=2")] {
        let sock = tmp_socket(name);
        serve_once(&sock, payload.as_bytes());
        let child = spawn_receiver(&sock, &[], &["/bin/sh", "-c", "echo RAN"]);
        let out = child.wait_with_output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(2),
            "{name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "",
            "{name}: command ran on bad payload"
        );
        let _ = std::fs::remove_file(&sock);
    }
}

#[test]
fn socket_empty_payload_allowed_with_allow_empty() {
    let sock = tmp_socket("allow-empty");
    serve_once(&sock, b"");
    let child = Command::new(XES)
        .arg("--from-socket")
        .arg(&sock)
        .arg("--allow-empty")
        .arg("--exec")
        .arg("--")
        .arg("/bin/sh")
        .arg("-c")
        .arg("echo RAN")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "RAN");
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn socket_name_policy_applies() {
    let sock = tmp_socket("deny");
    serve_once(&sock, b"LD_PRELOAD=\0PATH=/evil\0GOOD=1\0");
    let child = spawn_receiver(&sock, &[], &["/bin/true"]);
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("LD_PRELOAD"), "{stderr}");
    assert!(!stderr.contains("/evil"), "leaked value: {stderr}");
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn socket_keys_prints_names_only() {
    let sock = tmp_socket("keys");
    serve_once(&sock, b"SECRET_VALUE=hunter2\0");
    let out = Command::new(XES)
        .arg("--from-socket")
        .arg(&sock)
        .arg("--keys")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "SECRET_VALUE");
    assert!(!stdout.contains("hunter2"), "keys output leaked a value");
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn socket_requires_exec_or_keys() {
    let sock = tmp_socket("usage");
    let out = Command::new(XES)
        .arg("--from-socket")
        .arg(&sock)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("requires --exec"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn ssh_requires_dotenv_and_conflicts_rejected() {
    for args in [
        vec!["--ssh", "host"],
        vec![
            "--from-null",
            "--from-socket",
            "/tmp/s",
            "--exec",
            "--",
            "true",
        ],
        vec!["-d", "--ssh", "host", "--exec", "--", "true"],
    ] {
        let out = Command::new(XES).args(&args).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(2),
            "args {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
