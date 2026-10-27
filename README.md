# xes

`xes` is a fast, robust, production-quality drop-in replacement for GNU gettext's `envsubst` written in pure Rust (2024 edition).

It faithfully replicates GNU `envsubst`'s command-line interface and default substitution behavior, while adding:
1. **Full Bash Parameter Expansion** for `${...}` syntax (following Bash reference manual section 3.5.3).
2. **Bash Arithmetic Expansion** (`$(( ... ))`) behind `--arith`.
3. **Sequential `.env` Evaluation** (`--dotenv` / `-d`), allowing variables defined earlier in a file to be referenced by later lines without ever executing arbitrary code.

The workspace is cleanly divided into:
- **`xesubst`** (`crates/xesubst`): The core library providing parsing, AST, evaluation, globbing, and `.env` handling.
- **`xes`** (`crates/xes`): The command-line binary.

---

## Installation

### From Source (Cargo)

```bash
# Clone the repository
git clone https://github.com/tnotstar/xesubst.git
cd xesubst

# Build release binaries
cargo build --release

# Install binary locally
cargo install --path crates/xes
```

The compiled binary will be placed at `target/release/xes`.

### Using Make

```bash
make            # Default: build debug binaries (cargo build --workspace)
make test       # Run all tests (cargo test --workspace)
make lint       # Run linter (cargo clippy --workspace --all-targets -- -D warnings)
make format     # Format code (cargo fmt)
make release    # Full release pipeline: distclean => format => lint => debug build => test => release build => clean
make clean      # Remove all build artifacts except the binary executable
make distclean  # Remove all derivative assets (cargo clean)
```

---

## Feature Matrix

| Feature | GNU `envsubst` | `bash -c` | `xes` (default) | `xes --compat` | `xes --dotenv` |
|---|:---:|:---:|:---:|:---:|:---:|
| `$VAR` & `${VAR}` from environment | ✅ | ✅ | ✅ | ✅ | ✅ |
| `SHELL-FORMAT` filtering | ✅ | ❌ | ✅ | ✅ | ❌ |
| `-v` / `--variables` list | ✅ | ❌ | ✅ | ✅ | ❌ |
| `-` (stdin) / file arguments | Stdin only | ❌ | ✅ | ✅ | ✅ |
| In-place editing (`-i` / `--in-place`) | ❌ | ❌ | ✅ | ✅ | ✅ |
| Output file redirection (`-o`) | ❌ | ❌ | ✅ | ✅ | ✅ |
| Default value: `${v:-w}`, `${v-w}` | ❌ | ✅ | ✅ | ❌ | ✅ |
| Assign default: `${v:=w}`, `${v=w}` | ❌ | ✅ | ✅ | ❌ | ✅ |
| Error if unset: `${v:?w}`, `${v?w}` | ❌ | ✅ | ✅ | ❌ | ✅ |
| Alternative value: `${v:+w}`, `${v+w}` | ❌ | ✅ | ✅ | ❌ | ✅ |
| String length: `${#v}` | ❌ | ✅ | ✅ | ❌ | ✅ |
| Substrings: `${v:off}`, `${v:off:len}` | ❌ | ✅ | ✅ | ❌ | ✅ |
| Prefix stripping: `${v#p}`, `${v##p}` | ❌ | ✅ | ✅ | ❌ | ✅ |
| Suffix stripping: `${v%p}`, `${v%%p}` | ❌ | ✅ | ✅ | ❌ | ✅ |
| Pattern replace: `${v/p/r}`, `${v//p/r}`, `${v/#p/r}`, `${v/%p/r}` | ❌ | ✅ | ✅ | ❌ | ✅ |
| Case modification: `${v^}`, `${v^^}`, `${v,}`, `${v,,}` | ❌ | ✅ | ✅ | ❌ | ✅ |
| Indirect expansion: `${!v}` | ❌ | ✅ | ✅ | ❌ | ✅ |
| Prefix name listing: `${!prefix*}` / `${!prefix@}` | ❌ | ✅ | ✅ | ❌ | ✅ |
| Transformation operators: `@Q`, `@E`, `@A`, `@U`, `@u`, `@L` | ❌ | ✅ | ✅ | ❌ | ✅ |
| Integer arithmetic `$(( ... ))` | ❌ | ✅ | With `--arith` | ❌ | With `--arith` |
| Sequential line-by-line `.env` evaluation | ❌ | Via `source` | ❌ | ❌ | ✅ |
| Multi-format `.env` output (`dotenv`, `export`, `json`, `null`) | ❌ | ❌ | ❌ | ❌ | ✅ |
| Safe sandboxing (no process execution / RCE) | ✅ | ❌ (evaluates code) | ✅ | ✅ | ✅ |

---

## Usage Examples

### 1. Drop-in Replacement for GNU `envsubst`

By default, `xes` reads from stdin and writes to stdout:

```bash
export USER="alice"
export HOST="example.com"

# Standard substitution
echo 'Connecting $USER to ${HOST}...' | xes
# Output: Connecting alice to example.com...

# Restrict substitution to variables mentioned in SHELL-FORMAT
echo 'Deploying $USER to $HOST with secret $TOKEN' | xes '$USER $HOST'
# Output: Deploying alice to example.com with secret $TOKEN

# List variable names from a format string
xes -v '$DATABASE_URL and ${REDIS_HOST} and $PORT'
# Output:
# DATABASE_URL
# REDIS_HOST
# PORT

# Exact GNU envsubst compatibility mode (disables Bash extensions)
echo '${USER:-guest}' | xes --compat
# Output: ${USER:-guest}
```

### 2. Bash Parameter Expansion

`xes` natively expands all standard Bash parameter expansion constructs without requiring `bash`:

```bash
# Defaults (colon tests for unset or empty; non-colon tests for unset only)
echo "${DATABASE_PORT:-5432}" | xes

# Quoting inside words is stripped just like in Bash:
export AZURE_TENANT_ID="acme-corp"
echo '${BC_TENANT_ID:-"$AZURE_TENANT_ID"}' | xes
# Output: acme-corp

# Recursive expansions
echo '${PRIMARY_URL:-${SECONDARY_URL:-"http://localhost:8080"}}' | xes

# String length (Unicode scalar values / characters)
export GREETING="café 👋"
echo '${#GREETING}' | xes
# Output: 6

# Substrings (offsets support negative values and arithmetic)
export ALPHABET="abcdefghijklmnop"
echo '${ALPHABET:2:4}' | xes      # Output: cdef
echo '${ALPHABET: -4}' | xes       # Output: mnop
echo '${ALPHABET:2:-2}' | xes      # Output: cdefghijklmn

# Path and pattern stripping (shortest # / %, longest ## / %%)
export FILEPATH="/var/log/nginx/access.log"
echo '${FILEPATH##*/}' | xes       # Output: access.log
echo '${FILEPATH%/*}' | xes        # Output: /var/log/nginx

# Pattern replacement (first, all, anchored prefix, anchored suffix)
export MSG="foo-bar-baz-bar"
echo '${MSG/bar/qux}' | xes        # Output: foo-qux-baz-bar
echo '${MSG//bar/qux}' | xes       # Output: foo-qux-baz-qux
echo '${MSG/#foo/start}' | xes     # Output: start-bar-baz-bar
echo '${MSG/%bar/end}' | xes       # Output: foo-bar-baz-end

# Case modifications
export SERVICE="auth-service"
echo '${SERVICE^^}' | xes          # Output: AUTH-SERVICE

# Shell transformations
export CONFIG="path with spaces and 'quotes'"
echo '${CONFIG@Q}' | xes           # Output: 'path with spaces and '\''quotes'\'''
```

### 3. Arithmetic Expansion (`--arith`)

```bash
export BASE=100
echo 'Offset: $(( BASE * 2 + 10 ))' | xes --arith
# Output: Offset: 210
```

### 4. Sequential Dotenv Mode (`-d` / `--dotenv`)

Evaluate `.env` files top-to-bottom so that later lines can reference variables assigned earlier in the same file:

```bash
cat << 'EOF' > app.env
RUNTIME_ENVIRON="prod"
APP_NAME="portal"
APP_HOST="app-${RUNTIME_ENVIRON}.internal"
DATABASE_URL="postgres://${APP_NAME}@${APP_HOST}:5432/db"
LOG_DIR=
EOF

# Evaluate and print in safely quoted dotenv format:
xes -d app.env

# Exportable shell format:
xes -d --format export app.env

# JSON format:
xes -d --format json app.env

# NUL-delimited format (for xargs -0 or env -0):
xes -d --format null app.env

# Output only specific keys:
xes -d --only DATABASE_URL app.env

# List keys only:
xes -d --keys app.env

# Preload existing environment files:
xes -d --env-file base.env app.env

# Protect existing environment variables from being overwritten:
xes -d --no-override app.env
```

### 5. In-Place File Editing (`-i` / `--in-place`)

Safely modifies files in place using atomic write + rename semantics:

```bash
xes -i config.yaml
xes -d -i .env
```

---

## Diagnostic Messages

Every error outputs `file:line:column`, the offending snippet, and a pointer caret:

```text
config.env:7:16: unterminated '${'
XDATA_USERNAME=${MSSQL_USERNAME
               ^
```

### Exit Codes

| Code | Meaning |
|:---:|---|
| `0` | Success |
| `1` | Expansion error (`:?` assertion failure, `-u` / `--nounset` unset variable, `--strict` violation) |
| `2` | Usage or CLI configuration error |
| `3` | I/O error (file not found, permission denied, etc.) |

---

## Security and Unsupported Constructs

`xes` is explicitly designed as a safe configuration preprocessor. It never executes code, never spawns subshells, and never invokes arbitrary commands.

The following shell constructs are deliberately **unsupported**:

1. **Command substitution (`$(...)` and backticks `` `...` ``)**:
   - *Why*: Executing arbitrary subprocesses during template expansion creates significant security risks (Remote Code Execution, arbitrary command execution in untrusted environments).
   - *Behavior*: Copied literally by default; causes an immediate error when `--strict` is enabled.
2. **Process substitution (`<(...)`, `>(...)`)**:
   - *Why*: Requires creating named pipes or file descriptors tied to running processes.
   - *Behavior*: Copied literally; rejected with `--strict`.
3. **Positional and special shell parameters (`$1`, `$$`, `$?`, `$@`, `$*`)**:
   - *Why*: These represent ephemeral shell execution state that does not exist in a configuration and environment substitution engine.
   - *Behavior*: Copied literally; rejected with `--strict`.
4. **Shell arrays and subscripting (`${arr[0]}`)**:
   - *Why*: Environment variables in POSIX operating systems are string-to-string mappings without native array structures.
   - *Behavior*: Rejected with syntax diagnostic.
5. **Prompt (`@P`) and array attribute (`@K`, `@a`) transformations**:
   - *Why*: Prompt strings evaluate terminal escape sequences and shell-internal prompt expansions (`\u`, `\h`), while `@K` and `@a` operate on associative arrays.
   - *Behavior*: Rejected with a clear descriptive error.

---

## Architecture

The project is structured as a clean Cargo workspace:

- **`crates/xesubst`**: Core library crate containing:
  - `lexer.rs` & `parser.rs`: Streaming recursive-descent parser producing AST nodes with source spans.
  - `ast.rs`: Typed representation of expansions, operators, and word parts.
  - `glob.rs`: Self-contained Bash glob pattern engine supporting wildcards (`*`, `?`), ranges, character classes, and POSIX classes with shortest and longest match semantics.
  - `arith.rs`: Integer arithmetic expression evaluator.
  - `env.rs`: `VariableTable` trait with `EnvMap` and `OverlayEnv` implementations.
  - `eval.rs`: Evaluator over `VariableTable`.
  - `dotenv.rs`: Sequential `.env` evaluator and multi-format serializer.
  - `span.rs` & `error.rs`: Source location tracking, diagnostic rendering, and exit code mapping.
- **`crates/xes`**: A thin CLI wrapper binary crate using `clap` (derive), managing streams, file I/O, and atomic writes.

---

## License

Dual-licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE) or http://opensource.org/licenses/MIT)

at your option.
