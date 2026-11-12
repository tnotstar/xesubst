//! Pure helpers for `--ssh` orchestration. No process spawning here: the
//! argument vectors built in this module are unit-testable and never contain
//! variable values — only the random socket path and the user's own command.

use xesubst::dotenv::safe_quote_shell;

/// Random bytes used for socket and directory names (128 bits).
const RANDOM_BYTES: usize = 16;

/// Maximum usable Unix socket path length. Linux allows 108 bytes and
/// macOS 104; 104 is the safe portability bound.
pub const MAX_SOCKET_PATH: usize = 104;

/// Suffixes appended to the private directory name.
const DIR_PREFIX: &str = ".xes-";
const SOCK_NAME: &str = "env.sock";
const REMOTE_TEMPLATE: &str = "/tmp/xes-";

/// Reads `RANDOM_BYTES` bytes from the operating system CSPRNG
/// (`/dev/urandom`) and returns them as a lowercase hex string.
pub fn random_hex() -> std::io::Result<String> {
    use std::io::Read;
    let mut buf = [0u8; RANDOM_BYTES];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Builds the local private directory name under `root` and checks that
/// both the local socket path and the remote socket path fit within
/// `MAX_SOCKET_PATH` bytes. Returns the directory name (not the full
/// path) plus the remote socket path.
pub fn socket_names(root: &std::path::Path, rand_hex: &str) -> Result<(String, String), String> {
    let dir_name = format!("{DIR_PREFIX}{rand_hex}");
    let local_len = root
        .as_os_str()
        .len()
        + 1 // path separator
        + dir_name.len()
        + 1
        + SOCK_NAME.len();
    let remote_socket = format!("{REMOTE_TEMPLATE}{rand_hex}");
    if local_len > MAX_SOCKET_PATH {
        return Err(format!(
            "local socket path would be {local_len} bytes (limit {MAX_SOCKET_PATH}); \
             set XDG_RUNTIME_DIR to a shorter directory"
        ));
    }
    if remote_socket.len() > MAX_SOCKET_PATH {
        return Err(format!(
            "remote socket path would be {} bytes (limit {MAX_SOCKET_PATH})",
            remote_socket.len()
        ));
    }
    Ok((dir_name, remote_socket))
}

/// Builds the remote command string executed by ssh:
/// `<remote-xes> --from-socket SOCK --exec -- CMD...`, with every CMD
/// argument quoted for the remote shell. When `cmd` is empty the remote
/// login shell (`$SHELL`, expanded by the remote shell) is used.
pub fn build_remote_command(remote_xes: &str, socket: &str, cmd: &[String]) -> String {
    let mut out = format!(
        "{quoted} --from-socket {socket} --exec --",
        quoted = safe_quote_shell(remote_xes)
    );
    if cmd.is_empty() {
        out.push_str(" $SHELL");
    } else {
        for arg in cmd {
            out.push(' ');
            out.push_str(&safe_quote_shell(arg));
        }
    }
    out
}

/// Builds the full ssh argument vector. `payload` never appears here: the
/// variables travel through the forwarded Unix socket inside the encrypted
/// channel.
pub fn build_ssh_args(
    dest: &str,
    remote_socket: &str,
    local_socket: &str,
    allocate_tty: bool,
    ssh_options: &[String],
    remote_command: &str,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-o".into(),
        "ExitOnForwardFailure=yes".into(),
        "-R".into(),
        format!("{remote_socket}:{local_socket}"),
    ];
    if allocate_tty {
        args.push("-t".into());
    }
    for opt in ssh_options {
        args.push("-o".into());
        args.push(opt.clone());
    }
    args.push(dest.to_string());
    args.push(remote_command.to_string());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_command_quotes_everything() {
        let cmd = vec![
            "program".to_string(),
            "arg with spaces".to_string(),
            "it's".to_string(),
            "$HOME".to_string(),
        ];
        let out = build_remote_command("xes", "/tmp/s", &cmd);
        assert_eq!(
            out,
            "'xes' --from-socket /tmp/s --exec -- 'program' 'arg with spaces' 'it'\\''s' '$HOME'",
        );
        // No shell metacharacter may remain active inside user arguments.
        assert!(out.contains("'arg with spaces'"));
    }

    #[test]
    fn remote_command_defaults_to_login_shell() {
        assert_eq!(
            build_remote_command("xes", "/tmp/s", &[]),
            "'xes' --from-socket /tmp/s --exec -- $SHELL"
        );
    }

    #[test]
    fn remote_xes_path_is_quoted() {
        assert!(
            build_remote_command("/opt/bin/xes", "/s", &["c".into()]).starts_with("'/opt/bin/xes'")
        );
    }

    #[test]
    fn ssh_args_shape() {
        let args = build_ssh_args(
            "user@host",
            "/tmp/remote-sock",
            "/run/user/1000/xes/local-sock",
            true,
            &["ServerAliveInterval=30".to_string()],
            "remote-cmd",
        );
        assert_eq!(args[0], "-o");
        assert_eq!(args[1], "ExitOnForwardFailure=yes");
        assert_eq!(args[2], "-R");
        assert_eq!(args[3], "/tmp/remote-sock:/run/user/1000/xes/local-sock");
        assert_eq!(args[4], "-t");
        assert_eq!(args[5], "-o");
        assert_eq!(args[6], "ServerAliveInterval=30");
        assert_eq!(args[7], "user@host");
        assert_eq!(args[8], "remote-cmd");
        assert_eq!(args.len(), 9);
    }

    #[test]
    fn ssh_args_without_tty_and_options() {
        let args = build_ssh_args("h", "/r", "/l", false, &[], "cmd");
        assert!(!args.contains(&"-t".to_string()));
        assert_eq!(*args.last().unwrap(), "cmd");
    }

    #[test]
    fn random_hex_is_long_and_unique() {
        let a = random_hex().unwrap();
        let b = random_hex().unwrap();
        assert_eq!(a.len(), 32); // 16 bytes -> 32 hex chars (128 bits)
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn socket_names_fit_short_runtime_dirs() {
        // XDG_RUNTIME_DIR typical on Linux: /run/user/1000 (13 bytes).
        let (dir, remote) = socket_names(std::path::Path::new("/run/user/1000"), &"a".repeat(32))
            .expect("short root must pass");
        assert_eq!(dir, format!(".xes-{}", "a".repeat(32)));
        assert_eq!(remote, format!("/tmp/xes-{}", "a".repeat(32)));
        // Local: 13 + 1 + 37 + 1 + 8 = 60 bytes; remote: 42 bytes.
        assert!(remote.len() <= MAX_SOCKET_PATH);
    }

    #[test]
    fn socket_names_accept_macos_tmpdir_fallback() {
        // macOS temp fallback is $TMPDIR, e.g.
        // /var/folders/1q/abcdefgh_tnotstar/T/ (34 bytes here; longer
        // user hashes still stay under the limit with 128-bit names).
        let root = std::path::Path::new("/var/folders/1q/abcdefgh_tnotstar/T");
        assert!(socket_names(root, &"a".repeat(32)).is_ok());
        // A root long enough to exceed 104 bytes must fail clearly.
        let long_root = std::path::PathBuf::from(
            "/var/folders/1q/abcdefgh_tnotstar/T/".to_string() + &"x".repeat(80),
        );
        let err = socket_names(&long_root, &"a".repeat(32)).unwrap_err();
        assert!(err.contains("local socket path"), "{err}");
        assert!(err.contains("XDG_RUNTIME_DIR"), "{err}");
    }

    #[test]
    fn socket_names_reject_overlong_paths() {
        // Not reachable via random_hex today (names are 32 hex chars), but
        // the guards must hold if the name length or templates ever grow.
        // With an overlong name both checks trip; the local one reports
        // first, so accept either message.
        let err =
            socket_names(std::path::Path::new("/tmp"), &"a".repeat(MAX_SOCKET_PATH)).unwrap_err();
        assert!(err.contains("socket path"), "{err}");
    }
}
