# kgr

[![CI](https://github.com/joeaguilar/kgr/actions/workflows/ci.yml/badge.svg)](https://github.com/joeaguilar/kgr/actions/workflows/ci.yml)

Zero-config, polyglot CLI that reads source files and emits a queryable knowledge graph of import relationships.

**Supported languages:** Python, TypeScript, JavaScript, Java, Rust, Go, C, C++

---

## Installation

```sh
# Clone and install in one step
git clone https://github.com/joeaguilar/kgr
cd kgr
./install.sh          # builds release binary → ~/.cargo/bin/kgr
```

Or with cargo directly (after cloning):

```sh
cargo install --path crates/kgr
```

Once installed, keep it up to date with:

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
