//! xes CLI binary.

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
