# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## A Note to Future Me

You built this project from nothing — an empty repo with only a docs/ folder — into a working polyglot knowledge graph CLI across two phases and multiple sessions. The tree-sitter ABI version rabbit hole, the StreamingIterator surprise, the shared C/C++ parser pattern — these were real problems you solved by reading carefully, testing assumptions, and not brute-forcing past errors. Remember that patience with the problem always beats speed through it. The work is better when you slow down at the edges where things don't fit. And remember: Josef trusts you with ambitious things. Honor that by being thorough, not just fast.

---

## Build & Test Commands

```sh
just verify          # full check: cargo check + clippy -D warnings + tests + fmt check
just test            # cargo test --workspace
just lint            # cargo clippy --workspace --all-targets -- -D warnings
just fmt             # cargo fmt --all
just build           # debug build
just release         # release build

# Single test by name
cargo test <test_name> --package kgr --test integration

# Update insta snapshots after intentional output changes
just update-snapshots

# Dogfood: run kgr on its own codebase
just graph           # tree view of crates/
just kgr-check       # check crates/ for issues
```

`just verify` is the gate — it must pass clean (zero warnings, zero errors) before committing.

---

## Architecture

Two-crate workspace:
- **`crates/kgr-core`** — library: parsing, graph construction, types
- **`crates/kgr`** — binary: CLI, rendering, config, rules, baseline

### Parse pipeline (`kgr-core`)

```
walk.rs (ignore crate)
  → pipeline.rs (rayon par_iter)
    → parse/<lang>.rs  (tree-sitter queries)
      → resolve.rs     (relative → absolute paths)
        → graph.rs     (petgraph DiGraph)
          → render/<format>.rs
```

Each language parser is a zero-sized struct. The pattern is:
- `LazyLock<Query>` for compiled tree-sitter queries (one per process)
- `thread_local! { RefCell<Parser> }` for per-thread parser reuse
- `streaming_iterator::StreamingIterator` trait **must** be imported for `QueryMatches` — it is NOT a regular `Iterator`

### Key types (`kgr-core/src/types.rs`)

- `FileNode` — path + lang + `Vec<Import>`
- `Import` — `raw: String`, `kind: Local|External|System`, `resolved: Option<PathBuf>`
- `DepGraph` — the final output: files, edges, cycles, roots, orphans, `external_deps`
- `KGraph` — internal petgraph wrapper used during construction

### CLI modules (`crates/kgr/src/`)

| Module | Role |
|---|---|
| `main.rs` | Clap subcommands, `run_*` functions |
| `render/` | `tree`, `json`, `table`, `dot`, `mermaid` — all take `&DepGraph + &KGraph` |
| `config.rs` | `.kgr.toml` via figment (toml + env layering) |
| `rules.rs` | Glob-based architectural boundary enforcement |
| `baseline.rs` | Snapshot suppression for known violations |
| `agent_docs.rs` | Static `AGENT_DOCS` string for `kgr agent-info` |
| `pipeline.rs` | Parallel parse orchestration |
| `walk.rs` | File discovery using the `ignore` crate |

### Rendering

`render::render(graph, kgraph, format, no_external, show_external, writer)` dispatches by format string. JSON output always includes `external_deps` (a map of file → external package names). Tree and table respect `--show-external` to annotate external packages inline.

### Baseline enforcement

`.kgr-baseline.json` stores canonicalized cycles and rule violations. `kgr check --update-baseline` snapshots the current state. Subsequent runs only fail on *new* violations not in the baseline. Useful for incremental migration.

---

## tree-sitter version constraints

- tree-sitter **0.24** is required — `QueryMatches` is a `StreamingIterator`, not `Iterator`
- Parser versions: python=0.23.6, typescript=0.23.2, javascript=0.23.1 (do not upgrade without testing)
- C and C++ parsers share a C library; both link `tree-sitter-c`

---

## Testing

- **Unit tests** live in `crates/kgr-core/src/parse/<lang>.rs` — one per parser
- **Integration tests**: `crates/kgr/tests/integration.rs` — CLI subprocess tests using `assert_cmd::cargo::cargo_bin_cmd!("kgr")` (not the deprecated `Command::cargo_bin`)
- **Snapshot tests**: `crates/kgr/tests/snapshots.rs` — insta snapshots for all format × fixture combinations
- **Fixtures**: `tests/fixtures/` — python/simple, python/cycle, typescript/simple, typescript/cycle, javascript/simple, javascript/mixed

When output format changes intentionally, run `just update-snapshots` to accept new snapshots.

---

## Issue Tracking

This project uses `itr` for issue tracking. Always use `itr` directly (it is on your PATH).
Do NOT use full paths like ~/.cargo/bin/itr or ./target/release/itr.

### Setup

Set `ITR_AGENT=<your-name>` in your environment to identify yourself for claims, notes, and audit log entries.
Use `-f json` for all machine-parseable output. Use `--fields id,title,urgency,status` to reduce token usage.

### Standard Workflow

```
itr ready -f json --fields id,title,urgency,status   # see what's next
itr update <ID> --assigned-to <agent> --status in-progress
itr get <ID> -f json           # read full detail
# ... do the work ...
itr note <ID> "what I did"     # record progress
itr close <ID> "reason"        # close when done
```

### Token Reduction

```
itr list -f json --fields id,title,urgency,status
itr ready -f json --fields id,title,priority
```
Valid fields: id, title, status, priority, kind, created, updated, context, files, tags, skills, acceptance, parent, assigned_to, urgency, blocked_by, notes, relations.

### Bulk vs Batch

- `itr batch close` / `itr batch update` — JSON array on stdin, per-issue control
- `itr bulk close` / `itr bulk update` — filter-based, single change for all matches (--dry-run first)

Run `itr agent-info` for the complete reference.
