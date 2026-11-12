//! xes CLI binary.

mod ssh;

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use clap::Parser;
use xesubst::ModeConfig;
use xesubst::dotenv::{DotenvFormat, DotenvOptions, eval_dotenv, format_dotenv_output};
use xesubst::env::EnvMap;
use xesubst::error::XesubstError;
use xesubst::eval::{expand_str, expand_stream};
use xesubst::parser::extract_shell_format_variables;
use xesubst::span::format_diagnostic;

#[derive(Parser, Debug)]
#[command(
    name = "xes",
    version,
    about = "A Bash-compatible envsubst replacement in Rust",
    long_about = "Drop-in replacement for GNU envsubst with full Bash parameter expansion, arithmetic, and dotenv evaluation."
)]
struct Cli {
    /// Print the variables referenced in SHELL-FORMAT, one per line.
    #[arg(short = 'v', long = "variables")]
    variables: bool,

    /// Strict GNU envsubst compatibility (disables all Bash extensions).
    #[arg(long = "compat")]
    compat: bool,

    /// Enable integer arithmetic expansion $(( ... )).
    #[arg(long = "arith")]
    arith: bool,

    /// Treat unset variables as an error when substituting (set -u).
    #[arg(short = 'u', long = "nounset")]
    nounset: bool,

    /// Strict security mode (reject command sub, special/positional params, arrays, CRLF).
    #[arg(long = "strict")]
    strict: bool,

    /// Enable dotenv mode (sequential key=value evaluation).
    #[arg(short = 'd', long = "dotenv")]
    dotenv: bool,

    /// Preload variables from other dotenv files before evaluation.
    #[arg(long = "env-file", value_name = "FILE")]
    env_file: Vec<PathBuf>,

    /// In dotenv mode, do not overwrite variables already present in the environment.
    #[arg(long = "no-override")]
    no_override: bool,

    /// Output format in dotenv mode: dotenv, export, json, null.
    #[arg(long = "format", default_value = "dotenv")]
    format: DotenvFormat,

    /// In dotenv mode, only output specified keys.
    #[arg(long = "only", value_name = "KEY")]
    only: Vec<String>,

    /// In dotenv mode, list keys only.
    #[arg(long = "keys")]
    keys: bool,

    /// Write output to FILE instead of standard output.
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    output: Option<PathBuf>,

    /// Edit files in place (atomic write via temporary file + rename).
    #[arg(short = 'i', long = "in-place")]
    in_place: bool,

    /// Positional arguments: [SHELL-FORMAT] [FILE...] or [FILE...].
    #[arg(value_name = "ARGS")]
    args: Vec<String>,

    /// Read NUL-delimited KEY=VALUE records (the output of --format null)
    /// from FILE arguments or stdin instead of evaluating a template.
    #[arg(long = "from-null")]
    from_null: bool,

    /// Replace the current process with CMD (after `--`), adding the
    /// resulting variables to its environment. Nothing is printed.
    #[arg(long = "exec")]
    exec: bool,

    /// Accept an empty record payload instead of failing closed.
    #[arg(long = "allow-empty")]
    allow_empty: bool,

    /// Allowlist glob for names otherwise refused in receiver mode (repeatable).
    #[arg(long = "accept", value_name = "GLOB")]
    accept: Vec<String>,

    /// Disable the default unsafe-name denylist in receiver mode.
    #[arg(long = "allow-unsafe-names")]
    allow_unsafe_names: bool,

    /// Maximum accepted input size for --from-null, in bytes.
    #[arg(long = "max-input", default_value_t = xesubst::nullrec::DEFAULT_MAX_INPUT)]
    max_input: usize,

    /// Command to execute after `--` (used with --exec).
    #[arg(last = true, value_name = "CMD")]
    cmd: Vec<String>,

    /// Connect to a Unix socket and read the NUL-delimited payload from it
    /// (receiver side of --ssh), then proceed as --from-null.
    #[arg(long = "from-socket", value_name = "PATH", conflicts_with_all = ["from_null", "dotenv", "in_place"])]
    from_socket: Option<PathBuf>,

    /// Evaluate the template locally and deliver the variables to CMD on a
    /// remote host over ssh, via a forwarded Unix socket. Requires -d.
    #[arg(long = "ssh", value_name = "DEST", requires = "dotenv", conflicts_with_all = ["exec", "from_null", "from_socket", "in_place", "keys", "output"])]
    ssh_dest: Option<String>,

    /// Extra option passed verbatim to ssh as `-o OPT` (repeatable).
    #[arg(long = "ssh-option", value_name = "OPT")]
    ssh_option: Vec<String>,

    /// Path of the xes binary on the remote host (default: `xes`).
    #[arg(long = "remote-xes", default_value = "xes")]
    remote_xes: String,
}

fn main() {
    let cli = Cli::parse();

    if let Err(err) = run(cli) {
        let code = err.exit_code();
        eprintln!("{err}");
        process::exit(code);
    }
}

fn run(cli: Cli) -> Result<(), XesubstError> {
    // Check -v / --variables mode
    if cli.variables {
        let format_arg = match cli.args.first() {
            Some(arg) => arg,
            None => {
                eprintln!("xes: missing arguments");
                return Err(XesubstError::Usage(
                    "missing arguments for --variables".to_string(),
                ));
            }
        };
        let vars = extract_shell_format_variables(format_arg);
        for v in vars {
            println!("{v}");
        }
        return Ok(());
    }

    if let Some(dest) = cli.ssh_dest.clone() {
        return run_ssh(cli, dest);
    }
    if let Some(ref socket_path) = cli.from_socket {
        return run_from_socket(&cli, socket_path);
    }

    if cli.exec || cli.from_null {
        return run_exec(cli);
    }

    // Initialize environment from process env
    let mut env = EnvMap::from_process_env();

    let mut config = ModeConfig {
        compat: cli.compat,
        shell_format_vars: None,
        nounset: cli.nounset,
        strict: cli.strict,
        arith: cli.arith,
        filename: "<stdin>".to_string(),
    };

    // Preload --env-file variables
    for env_path in &cli.env_file {
        let content = std::fs::read_to_string(env_path).map_err(|e| {
            eprintln!("xes: cannot read env-file '{}': {e}", env_path.display());
            XesubstError::Io(e)
        })?;
        let sub_config = ModeConfig {
            filename: env_path.display().to_string(),
            ..config.clone()
        };
        eval_dotenv(&content, &mut env, &sub_config, &DotenvOptions::default())?;
    }

    // Partition positional arguments into optional SHELL-FORMAT and input files
    let (shell_format, input_files) = partition_args(&cli.args);

    if let Some(format_str) = shell_format {
        config.shell_format_vars = Some(extract_shell_format_variables(&format_str));
    }

    if cli.in_place {
        if cli.output.is_some() {
            return Err(XesubstError::Usage(
                "cannot combine --in-place with --output".to_string(),
            ));
        }
        if input_files.is_empty() || (input_files.len() == 1 && input_files[0] == "-") {
            return Err(XesubstError::Usage(
                "cannot use --in-place with standard input".to_string(),
            ));
        }
    }

    // Determine target output writer if not in-place
    let mut stdout_writer = io::stdout().lock();
    let mut file_writer: Option<BufWriter<File>> = None;

    if !cli.in_place
        && let Some(ref out_path) = cli.output
    {
        let f = File::create(out_path).map_err(XesubstError::Io)?;
        file_writer = Some(BufWriter::new(f));
    }

    // Process inputs
    let files_to_process = if input_files.is_empty() {
        vec!["-".to_string()]
    } else {
        input_files
    };

    for file_arg in &files_to_process {
        let is_stdin = file_arg == "-";
        config.filename = if is_stdin {
            "<stdin>".to_string()
        } else {
            file_arg.clone()
        };

        if cli.dotenv {
            // Dotenv evaluation mode
            let content = if is_stdin {
                let mut buf = String::new();
                io::stdin()
                    .read_to_string(&mut buf)
                    .map_err(XesubstError::Io)?;
                buf
            } else {
                std::fs::read_to_string(file_arg).map_err(|e| {
                    eprintln!("xes: cannot read '{}': {e}", file_arg);
                    XesubstError::Io(e)
                })?
            };

            let options = DotenvOptions {
                no_override: cli.no_override,
                format: cli.format,
                only: if cli.only.is_empty() {
                    None
                } else {
                    Some(cli.only.clone())
                },
                keys_only: cli.keys,
            };

            let results = match eval_dotenv(&content, &mut env, &config, &options) {
                Ok(r) => r,
                Err(err) => {
                    handle_diagnostic(&config.filename, &content, &err);
                    return Err(err);
                }
            };

            let formatted = format_dotenv_output(&results, &options)?;

            if cli.in_place {
                write_atomic(Path::new(file_arg), formatted.as_bytes())?;
            } else if let Some(ref mut fw) = file_writer {
                fw.write_all(formatted.as_bytes())
                    .map_err(XesubstError::Io)?;
            } else {
                stdout_writer
                    .write_all(formatted.as_bytes())
                    .map_err(XesubstError::Io)?;
            }
        } else {
            // Template expansion mode
            if cli.in_place {
                let content = std::fs::read_to_string(file_arg).map_err(|e| {
                    eprintln!("xes: cannot read '{}': {e}", file_arg);
                    XesubstError::Io(e)
                })?;
                let expanded = match expand_str(&content, &mut env, &config) {
                    Ok(exp) => exp,
                    Err(err) => {
                        handle_diagnostic(&config.filename, &content, &err);
                        return Err(err);
                    }
                };
                write_atomic(Path::new(file_arg), expanded.as_bytes())?;
            } else if is_stdin {
                let stdin = io::stdin().lock();
                if let Some(ref mut fw) = file_writer {
                    expand_stream(stdin, fw, &mut env, &config)?;
                } else {
                    expand_stream(stdin, &mut stdout_writer, &mut env, &config)?;
                }
            } else {
                let f = File::open(file_arg).map_err(|e| {
                    eprintln!("xes: cannot open '{}': {e}", file_arg);
                    XesubstError::Io(e)
                })?;
                let reader = BufReader::new(f);
                if let Some(ref mut fw) = file_writer {
                    expand_stream(reader, fw, &mut env, &config)?;
                } else {
                    expand_stream(reader, &mut stdout_writer, &mut env, &config)?;
                }
            }
        }
    }

    if let Some(mut fw) = file_writer {
        fw.flush().map_err(XesubstError::Io)?;
    }
    stdout_writer.flush().map_err(XesubstError::Io)?;

    Ok(())
}

fn partition_args(args: &[String]) -> (Option<String>, Vec<String>) {
    if args.is_empty() {
        return (None, Vec::new());
    }

    // If first argument contains $ or is not an existing file, it can be SHELL-FORMAT
    let first = &args[0];
    if first == "-" {
        return (None, args.to_vec());
    }

    if Path::new(first).exists() {
        return (None, args.to_vec());
    }

    if first.contains('$') || args.len() > 1 {
        // First is SHELL-FORMAT, remainder are files
        (Some(first.clone()), args[1..].to_vec())
    } else {
        // Single argument that might be a non-existent file or SHELL-FORMAT
        (Some(first.clone()), Vec::new())
    }
}

fn handle_diagnostic(file: &str, source: &str, err: &XesubstError) {
    if let Some(span) = err.span() {
        let diag = format_diagnostic(file, source, span, &err.to_string());
        eprintln!("{diag}");
    }
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<(), XesubstError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("tmp");
    let pid = process::id();
    let temp_name = format!(".{file_name}.xes_tmp.{pid}");
    let temp_path = parent.join(temp_name);

    {
        let mut f = File::create(&temp_path).map_err(XesubstError::Io)?;
        f.write_all(data).map_err(XesubstError::Io)?;
        f.flush().map_err(XesubstError::Io)?;
    }

    // Preserve permissions if existing file
    if let Ok(metadata) = path.metadata() {
        let _ = std::fs::set_permissions(&temp_path, metadata.permissions());
    }

    std::fs::rename(&temp_path, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp_path);
        XesubstError::Io(e)
    })?;

    Ok(())
}

/// Receiver / exec mode: `--from-null [--exec -- CMD]` or `-d --exec -- CMD`.
fn run_exec(cli: Cli) -> Result<(), XesubstError> {
    if cli.exec && cli.cmd.is_empty() {
        return Err(XesubstError::Usage(
            "--exec requires a command after `--`".to_string(),
        ));
    }
    if cli.keys && cli.exec {
        return Err(XesubstError::Usage(
            "--keys cannot be combined with --exec".to_string(),
        ));
    }

    let pairs: Vec<(String, String)> = if cli.from_null {
        // Receiver mode: the input is untrusted transport data.
        if cli.dotenv
            || cli.arith
            || cli.in_place
            || cli.variables
            || cli.output.is_some()
            || !cli.env_file.is_empty()
        {
            return Err(XesubstError::Usage(
                "--from-null cannot be combined with template evaluation options \
                 (-d, -i, --arith, --output, --env-file)"
                    .to_string(),
            ));
        }

        // Positional arguments are FILEs; a SHELL-FORMAT is a conflict.
        let (shell_format, input_files) = partition_args(&cli.args);
        if shell_format.is_some() {
            return Err(XesubstError::Usage(
                "--from-null cannot be combined with a SHELL-FORMAT".to_string(),
            ));
        }

        let mut input = Vec::new();
        if input_files.is_empty() || (input_files.len() == 1 && input_files[0] == "-") {
            io::stdin()
                .lock()
                .read_to_end(&mut input)
                .map_err(XesubstError::Io)?;
        } else {
            for file_arg in &input_files {
                if file_arg == "-" {
                    io::stdin()
                        .lock()
                        .read_to_end(&mut input)
                        .map_err(XesubstError::Io)?;
                } else {
                    File::open(file_arg)
                        .and_then(|mut f| f.read_to_end(&mut input))
                        .map_err(|e| {
                            eprintln!("xes: cannot read '{file_arg}': {e}");
                            XesubstError::Io(e)
                        })?;
                }
            }
        }

        match handle_received(&cli, &input)? {
            Some(pairs) => pairs,
            None => return Ok(()), // --keys: names printed
        }
    } else {
        // Dotenv mode: expansion happened locally, so the denylist is off
        // unless --accept narrows the accepted names explicitly.
        if !cli.dotenv {
            return Err(XesubstError::Usage(
                "--exec requires --dotenv or --from-null".to_string(),
            ));
        }
        if cli.in_place || cli.output.is_some() || cli.keys {
            return Err(XesubstError::Usage(
                "--exec cannot be combined with --in-place, --output, or --keys".to_string(),
            ));
        }

        eval_dotenv_cli(&cli)?
    };

    // Dotenv mode: expansion happened locally, so the denylist is off unless
    // --accept narrows the accepted names explicitly. Receiver mode already
    // applied its (always-on unless --allow-unsafe-names) policy above; the
    // re-check below is idempotent.
    let denylist = if cli.from_null {
        !cli.allow_unsafe_names
    } else {
        !cli.accept.is_empty() && !cli.allow_unsafe_names
    };
    let policy = xesubst::nullrec::NamePolicy::new(&cli.accept, denylist);
    xesubst::nullrec::check_name_policy(&pairs, &policy)?;

    let filtered: Vec<(String, String)> = pairs
        .into_iter()
        .filter(|(k, _)| cli.only.is_empty() || cli.only.contains(k))
        .collect();

    // Names already validated upstream; policy re-check is cheap and keeps
    // this final gate independent of the source branch.
    exec_replace(&cli.cmd, &filtered, cli.no_override)
}

/// Shared receiver post-processing for `--from-null` and `--from-socket`:
/// fail-closed parsing, name policy, `--keys` output. Returns `None` when
/// `--keys` has been handled.
fn handle_received(cli: &Cli, input: &[u8]) -> Result<Option<Vec<(String, String)>>, XesubstError> {
    let pairs = xesubst::nullrec::parse_null_records(input, cli.max_input)?;
    if pairs.is_empty() && !cli.allow_empty {
        return Err(XesubstError::NullRecord {
            index: 0,
            name: None,
            message: "no records received (a failed sender must not leave the \
                      command unconfigured; use --allow-empty to override)"
                .to_string(),
        });
    }
    let policy = xesubst::nullrec::NamePolicy::new(&cli.accept, !cli.allow_unsafe_names);
    xesubst::nullrec::check_name_policy(&pairs, &policy)?;

    if cli.keys {
        // Debugging aid: names only, never values.
        let mut out = io::stdout().lock();
        for (name, _) in &pairs {
            writeln!(out, "{name}").map_err(XesubstError::Io)?;
        }
        return Ok(None);
    }
    if !cli.exec {
        return Err(XesubstError::Usage(
            "--from-null/--from-socket requires --exec (or --keys to list names)".to_string(),
        ));
    }

    Ok(Some(pairs))
}

/// Evaluates the `-d` template inputs (files or stdin, plus `--env-file`
/// preloads) locally and returns the resulting pairs.
fn eval_dotenv_cli(cli: &Cli) -> Result<Vec<(String, String)>, XesubstError> {
    let mut env = EnvMap::from_process_env();
    let mut config = ModeConfig {
        compat: cli.compat,
        shell_format_vars: None,
        nounset: cli.nounset,
        strict: cli.strict,
        arith: cli.arith,
        filename: "<stdin>".to_string(),
    };

    for env_path in &cli.env_file {
        let content = std::fs::read_to_string(env_path).map_err(|e| {
            eprintln!("xes: cannot read env-file '{}': {e}", env_path.display());
            XesubstError::Io(e)
        })?;
        let sub_config = ModeConfig {
            filename: env_path.display().to_string(),
            ..config.clone()
        };
        eval_dotenv(&content, &mut env, &sub_config, &DotenvOptions::default())?;
    }

    let (shell_format, input_files) = partition_args(&cli.args);
    if let Some(format_str) = shell_format {
        config.shell_format_vars = Some(extract_shell_format_variables(&format_str));
    }

    let files_to_process = if input_files.is_empty() {
        vec!["-".to_string()]
    } else {
        input_files
    };

    let mut assigned_order: Vec<String> = Vec::new();
    let mut assigned_map: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    for file_arg in &files_to_process {
        let is_stdin = file_arg == "-";
        config.filename = if is_stdin {
            "<stdin>".to_string()
        } else {
            file_arg.clone()
        };
        let content = if is_stdin {
            let mut buf = String::new();
            io::stdin()
                .lock()
                .read_to_string(&mut buf)
                .map_err(XesubstError::Io)?;
            buf
        } else {
            std::fs::read_to_string(file_arg).map_err(|e| {
                eprintln!("xes: cannot read '{file_arg}': {e}");
                XesubstError::Io(e)
            })?
        };

        let results = match eval_dotenv(&content, &mut env, &config, &DotenvOptions::default()) {
            Ok(r) => r,
            Err(err) => {
                handle_diagnostic(&config.filename, &content, &err);
                return Err(err);
            }
        };
        for (k, v) in results {
            if !assigned_map.contains_key(&k) {
                assigned_order.push(k.clone());
            }
            assigned_map.insert(k, v);
        }
    }
    Ok(assigned_order
        .into_iter()
        .map(|k| {
            let v = assigned_map.get(&k).unwrap().clone();
            (k, v)
        })
        .collect())
}

/// Receiver side of `--ssh`: read the payload from a Unix socket, unlink it,
/// then proceed exactly as `--from-null`. Fail closed: an empty, truncated,
/// or early-closed payload never reaches the command.
#[cfg(unix)]
fn run_from_socket(cli: &Cli, socket_path: &Path) -> Result<(), XesubstError> {
    use std::io::Read;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    if cli.exec && cli.cmd.is_empty() {
        return Err(XesubstError::Usage(
            "--exec requires a command after `--`".to_string(),
        ));
    }
    if cli.keys && cli.exec {
        return Err(XesubstError::Usage(
            "--keys cannot be combined with --exec".to_string(),
        ));
    }
    if !cli.exec && !cli.keys {
        return Err(XesubstError::Usage(
            "--from-socket requires --exec (or --keys to list names)".to_string(),
        ));
    }

    let mut stream = UnixStream::connect(socket_path).map_err(|e| {
        eprintln!("xes: cannot connect to '{}': {e}", socket_path.display());
        XesubstError::Io(e)
    })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(XesubstError::Io)?;

    // Read bounded by --max-input; a hostile sender cannot grow memory.
    let mut input = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = stream.read(&mut chunk).map_err(XesubstError::Io)?;
        if n == 0 {
            break;
        }
        input.extend_from_slice(&chunk[..n]);
        if input.len() > cli.max_input {
            return Err(XesubstError::NullRecord {
                index: 0,
                name: None,
                message: format!(
                    "payload exceeds the {} byte limit (see --max-input)",
                    cli.max_input
                ),
            });
        }
    }

    // The socket served its purpose; remove it regardless of what follows.
    let _ = std::fs::remove_file(socket_path);

    if let Some(pairs) = handle_received(cli, &input)? {
        let filtered: Vec<(String, String)> = pairs
            .into_iter()
            .filter(|(k, _)| cli.only.is_empty() || cli.only.contains(k))
            .collect();
        exec_replace(&cli.cmd, &filtered, cli.no_override)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn run_from_socket(_cli: &Cli, _socket_path: &Path) -> Result<(), XesubstError> {
    Err(XesubstError::Usage(
        "--from-socket is only supported on Unix platforms".to_string(),
    ))
}

/// Sender side: evaluate the template locally, expose the serialized pairs
/// on a one-shot local Unix socket, forward it over ssh, and propagate the
/// ssh exit status.
#[cfg(unix)]
fn run_ssh(cli: Cli, dest: String) -> Result<(), XesubstError> {
    use std::io::IsTerminal;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::time::{Duration, Instant};

    const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

    let pairs = eval_dotenv_cli(&cli)?;
    let filtered: Vec<(String, String)> = pairs
        .into_iter()
        .filter(|(k, _)| cli.only.is_empty() || cli.only.contains(k))
        .collect();
    let options = DotenvOptions {
        no_override: cli.no_override,
        format: DotenvFormat::Null,
        only: None,
        keys_only: false,
    };
    let payload = format_dotenv_output(&filtered, &options)?;

    // Private runtime directory with an unpredictable socket name.
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let private_dir = runtime_dir.join(format!(".xes-{}", ssh::random_hex()?));
    std::fs::create_dir(&private_dir).map_err(XesubstError::Io)?;
    std::fs::set_permissions(&private_dir, std::fs::Permissions::from_mode(0o700))
        .map_err(XesubstError::Io)?;
    let local_socket = private_dir.join("env.sock");
    let listener = UnixListener::bind(&local_socket).map_err(XesubstError::Io)?;

    let remote_socket = format!("/tmp/xes-{}", ssh::random_hex()?);
    let remote_command = ssh::build_remote_command(&cli.remote_xes, &remote_socket, &cli.cmd);
    let allocate_tty = std::io::stdin().is_terminal();
    let args = ssh::build_ssh_args(
        &dest,
        &remote_socket,
        &local_socket.to_string_lossy(),
        allocate_tty,
        &cli.ssh_option,
        &remote_command,
    );

    let mut ssh_child = process::Command::new("ssh")
        .args(&args)
        .spawn()
        .map_err(|e| {
            eprintln!("xes: cannot execute ssh: {e}");
            cleanup_socket(&local_socket, &private_dir);
            XesubstError::Io(e)
        })?;

    // Serve exactly one connection, with a timeout.
    listener.set_nonblocking(true).map_err(XesubstError::Io)?;
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let connection = loop {
        match listener.accept() {
            Ok((conn, _)) => break conn,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if let Some(status) = ssh_child.try_wait().map_err(XesubstError::Io)? {
                    cleanup_socket(&local_socket, &private_dir);
                    return Err(XesubstError::Usage(format!(
                        "ssh exited before connecting back (status {status})"
                    )));
                }
                if Instant::now() > deadline {
                    let _ = ssh_child.kill();
                    cleanup_socket(&local_socket, &private_dir);
                    return Err(XesubstError::Usage(
                        "remote did not connect back within the timeout".to_string(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                cleanup_socket(&local_socket, &private_dir);
                return Err(XesubstError::Io(e));
            }
        }
    };

    connection
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(XesubstError::Io)?;
    {
        use std::io::Write;
        let mut w = connection;
        if let Err(e) = w.write_all(payload.as_bytes()).and_then(|_| w.flush()) {
            eprintln!("xes: failed to send payload: {e}");
            let _ = ssh_child.kill();
            cleanup_socket(&local_socket, &private_dir);
            return Err(XesubstError::Io(e));
        }
    }
    drop(listener);

    let status = ssh_child.wait().map_err(XesubstError::Io)?;
    cleanup_socket(&local_socket, &private_dir);
    process::exit(status.code().unwrap_or(1));
}

#[cfg(not(unix))]
fn run_ssh(_cli: Cli, _dest: String) -> Result<(), XesubstError> {
    Err(XesubstError::Usage(
        "--ssh is only supported on Unix platforms".to_string(),
    ))
}

/// Removes the one-shot socket and its private directory.
#[cfg(unix)]
fn cleanup_socket(local_socket: &Path, private_dir: &Path) {
    let _ = std::fs::remove_file(local_socket);
    let _ = std::fs::remove_dir(private_dir);
}

/// Replaces the current process with CMD, adding `pairs` to its environment.
/// Exit codes follow env(1): 126 if CMD cannot be invoked, 127 if not found.
#[cfg(unix)]
fn exec_replace(
    cmd: &[String],
    pairs: &[(String, String)],
    no_override: bool,
) -> Result<(), XesubstError> {
    use std::os::unix::process::CommandExt;

    let mut command = process::Command::new(&cmd[0]);
    command.args(&cmd[1..]);
    for (key, value) in pairs {
        if no_override && std::env::var_os(key).is_some() {
            continue;
        }
        command.env(key, value);
    }

    let err = command.exec();
    let code = match err.kind() {
        std::io::ErrorKind::NotFound => 127,
        _ => 126,
    };
    eprintln!("xes: cannot execute '{}': {err}", cmd[0]);
    process::exit(code);
}

#[cfg(not(unix))]
fn exec_replace(
    _cmd: &[String],
    _pairs: &[(String, String)],
    _no_override: bool,
) -> Result<(), XesubstError> {
    Err(XesubstError::Usage(
        "--exec is only supported on Unix platforms".to_string(),
    ))
}
