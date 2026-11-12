# Changelog

All notable changes to this project are documented in this file. The format
is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-11-12

### Added

- Receiver mode `--from-null`: parse NUL-delimited `KEY=VALUE` records (the
  exact output of `--format null`) from positional FILEs or stdin. Values
  are never expanded or interpreted. Fail-closed: empty payloads (unless
  `--allow-empty`), truncated final records, duplicate keys, invalid UTF-8,
  invalid names, and inputs over `--max-input` (default 1 MiB) are rejected
  before anything runs. `--from-null --keys` prints names only.
- `--exec -- CMD [ARGS...]`: replace the current process with CMD whose
  environment gains the resulting variables (usable with `-d` or
  `--from-null`/`--from-socket`; honours `--only` and `--no-override`;
  exit codes follow `env(1)`: 126/127). Unix only.
- Unsafe-name policy for untrusted transports: `LD_*`, `DYLD_*`, `PATH`,
  `IFS`, `BASH_ENV`, `BASH_FUNC_*`, interpreter paths (`PYTHONPATH`,
  `RUBYOPT`, `NODE_OPTIONS`, ...), `HOME`, `SHELL` and similar names are
  refused by default. `--accept GLOB` (repeatable) is a restrictive
  allowlist — a name matching no glob is an error — and never lifts the
  denylist; only `--allow-unsafe-names` does. With `-d --exec` the denylist
  is inactive unless `--accept` narrows the set.
- `--ssh DEST` (with `-d`): evaluate the template locally and deliver the
  variables to a remote program over ssh via a one-shot forwarded Unix
  socket (private 0700 directory under `$XDG_RUNTIME_DIR`, unpredictable
  socket paths, `-o ExitOnForwardFailure=yes -R`, `-t` only on a TTY,
  remote command arguments shell-quoted, ssh exit status propagated).
  Options: `--ssh-option OPT`, `--remote-xes PATH`.
- `--from-socket PATH`: receiver side of `--ssh`; same validation, name
  policy, and fail-closed semantics as `--from-null`, then the socket is
  unlinked.
- README: "Passing environment over SSH" section with threat model,
  pipeline `pipefail` guidance, and the `-d --exec` stdin-EOF caveat;
  updated feature matrix and exit-code table.

### Security

- No secret value is ever placed in a process argument list, a temp file, a
  log line, a panic message, or a `Debug` rendering; receiver diagnostics
  carry record index and variable name only.

## [0.1.0]

- Initial release: GNU `envsubst`-compatible substitution, full Bash
  parameter expansion, `--arith`, sequential dotenv evaluation (`-d`) with
  `dotenv`/`export`/`json`/`null` output formats, in-place editing, and the
  `xesubst` core library.
