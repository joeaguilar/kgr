# kgr

[![CI](https://github.com/joeaguilar/kgr/actions/workflows/ci.yml/badge.svg)](https://github.com/joeaguilar/kgr/actions/workflows/ci.yml)

Zero-config, polyglot CLI that reads source files and emits a queryable knowledge graph of import relationships.

**Supported languages:** Python, TypeScript, JavaScript, Java, Rust, Go, C, C++, Zig, C#, Objective-C, Swift, Ruby, PHP, Scala, Lua, Elixir, Haskell, Bash — 19 in total

---

## Installation

No Rust toolchain required: prebuilt binaries are published on every release for macOS (Intel + Apple Silicon), Linux (x86_64 glibc/musl, aarch64), and Windows (x86_64 + arm64).

On x86_64 Linux, the installer downloads the fully static musl build by default so it runs on any distro regardless of glibc version. The glibc artifact is still published if you'd rather grab it manually from the [Releases page](https://github.com/joeaguilar/kgr/releases/latest).

### macOS / Linux

```sh
curl -fsSL https://raw.githubusercontent.com/joeaguilar/kgr/main/install.sh | bash
```

The script auto-detects your platform, downloads the matching tarball from the latest GitHub Release, verifies its SHA256 checksum, and installs to an existing `kgr` location on `PATH`, `~/.cargo/bin` if it is already on `PATH`, or `~/.local/bin`.

To update an existing install, rerun the installer:

```sh
curl -fsSL https://raw.githubusercontent.com/joeaguilar/kgr/main/install.sh | bash -s -- --update
```

Environment overrides:

| Variable | Effect |
|---|---|
| `KGR_VERSION` | Pin a specific tag (e.g. `v0.2.0`). Defaults to latest. |
| `KGR_INSTALL_DIR` | Install directory. Defaults to the active `kgr` on `PATH`, `~/.cargo/bin`, or `~/.local/bin`. |
| `KGR_FROM_SOURCE=1` | Skip download and build with cargo. Must be run from a cloned repo. |

### Windows

```powershell
iwr -useb https://raw.githubusercontent.com/joeaguilar/kgr/main/install.ps1 | iex
```

Installs `kgr.exe` into `%LOCALAPPDATA%\Programs\kgr` and adds that directory to your user PATH. Use `-Version`, `-InstallDir`, or `-Repo` parameters to override defaults.

### Manual Download

Grab a release archive for your platform from [GitHub Releases](https://github.com/joeaguilar/kgr/releases/latest), verify the bundled `.sha256`, extract it, and drop `kgr` (or `kgr.exe`) anywhere on your `PATH`.

### From Source

If you'd rather build locally, or no prebuilt binary exists for your target, install Rust 1.81+ and run:

```sh
cargo install --git https://github.com/joeaguilar/kgr --bin kgr
# or
git clone https://github.com/joeaguilar/kgr && cd kgr && cargo install --path crates/kgr
```

For source installs, keep it up to date with:

```sh
kgr upgrade           # git pull + cargo build --release + in-place replace
```

---

## Quick start

```sh
kgr                          # dependency tree of current directory
kgr graph --format json .    # full graph as JSON
kgr check .                  # check for cycles, orphans, rule violations
kgr check --format json .    # structured check output for CI/agents
kgr query --cycles .         # list all cycles
kgr query --who-imports src/core/db.ts .
kgr init .                   # generate .kgr.toml config
kgr agent-info               # machine-readable guide for AI agents
```

---

## Subcommands

### `kgr graph [PATH]`

Scan PATH (default: `.`) and print the dependency graph.

| Flag | Description |
|---|---|
| `-f, --format` | `tree` (default), `json`, `table`, `dot`, `mermaid` |
| `-l, --lang` | Filter by language: `py`, `ts`, `js`, `rs`, `java`, `c`, `cpp`, `go` |
| `--no-external` | Hide third-party package nodes |
| `--show-external` | Show external package names as leaf nodes (tree/table) |
| `-o, --output` | Write output to file |
| `--no-progress` | Disable progress bar (use in CI/pipes) |

```sh
kgr graph --format dot . | dot -Tsvg -o graph.svg
kgr graph --format mermaid . > deps.md
kgr graph --format json --no-progress . | jq '.external_deps'
```

### `kgr check [PATH]`

Check for dependency issues. Exits 0 if clean, 1 if errors found.

| Flag | Description |
|---|---|
| `-f, --format` | `text` (default), `json` |
| `--update-baseline` | Record current violations as baseline (exits 0) |
| `--baseline <file>` | Path to baseline file (default: `.kgr-baseline.json`) |

JSON output:

```json
{
  "ok": true,
  "cycles": [["a.py", "b.py"]],
  "orphans": ["unused.py"],
  "rule_violations": [{"rule": "no-legacy-to-core", "from": "...", "to": "...", "severity": "error"}],
  "suppressed": 0
}
```

### `kgr query [PATH]`

| Flag | Description |
|---|---|
| `--who-imports <file>` | Files that import the given file |
| `--deps-of <file>` | All transitive dependencies of a file |
| `--path-between <a> <b>` | Shortest dependency path |
| `--cycles` | List all cycles |
| `--orphans` | List orphaned files |
| `--heaviest` | Files ranked by number of dependents |
| `-f, --format` | `table` (default), `json` |

### `kgr show <symbol> [PATH]`

Print the definition body of a symbol, straight from source, located via the index — replaces `grep -n` + `sed -n` chains.

```
$ kgr show render_table
── crates/kgr/src/render/table.rs:6-41 (function render_table) ──
   6  pub fn render_table(rows: &[Row], opts: &TableOpts) -> String {
   7      let widths = column_widths(rows);
   …
  41  }
```

| Flag | Description |
|---|---|
| `-c, --context <n>` | Include n lines before/after the definition (default 0) |
| `--all` | Print every match; default prints the first and lists the rest as one-line pointers |
| `-k, --kind <fn\|class\|method>` | Disambiguate same-named symbols |
| `-f, --format` | `text` (default), `json` — `{name, kind, path, start_line, end_line, exported, body}` per match |
| `--no-linenos` | Raw body, pipe-friendly |

### `kgr slice <file>:<start>[-<end>]`

Print a numbered, bounded line window from any file — no index, works on files kgr doesn't parse (`.toml`, `.md`, logs). Replaces `sed -n 'X,Yp'`.

```
$ kgr slice crates/kgr/src/main.rs:100-140
$ kgr slice engine.py:1134 -c 20        # single line ± context
```

| Flag | Description |
|---|---|
| `-c, --context <n>` | Expand a single-line target both ways (default 10 when no end given) |
| `--no-linenos` | Raw text, byte-identical to the source window |
| `-f, --format` | `text` (default), `json` — `{path, start_line, end_line, lines[]}` |
| `--max <n>` | Raise the 500-line output cap |

### `kgr init [PATH]`

Generates a `.kgr.toml` with detected languages and commented rule examples.

---

## Configuration (`.kgr.toml`)

```toml
[[rules]]
name     = "no-legacy-to-core"
from     = "legacy/**"       # glob matched against relative file paths
to       = "core/**"
severity = "error"           # "error" → exit 1 | "warn" → exit 0
```

Use `kgr check --update-baseline` to suppress known violations during a migration, then tighten the rules incrementally.

---

## JSON graph structure

`kgr graph --format json` emits a `DepGraph`:

```
{
  "root": "/abs/path",
  "files": [{ "path": "...", "lang": "python", "imports": [...] }],
  "edges": [{ "from": "...", "to": "...", "kind": "local|external|system" }],
  "cycles": [[...]],
  "roots": [...],
  "orphans": [...],
  "test_entries": [...],
  "external_deps": { "main.ts": ["express", "lodash"] }
}
```

---

## For AI agents

```sh
kgr agent-info               # plain-text workflow guide
kgr agent-info --format json # same, wrapped in {"guide": "..."}
```

Always pass `--no-progress` when parsing output programmatically.
