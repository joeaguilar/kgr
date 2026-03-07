/// Static plain-text guide for AI agents. Embedded at compile time.
pub const AGENT_DOCS: &str = r#"
kgr — polyglot source dependency knowledge graph
-------------------------------------------------

SUBCOMMANDS
-----------

kgr [graph] [PATH] [FLAGS]
  Scan PATH (default: .) and emit the full dependency graph.
  Flags:
    -f, --format <fmt>     Output format: tree (default), json, table, dot, mermaid
    -l, --lang <lang>      Filter by language: py, ts, js, rs, java, c, cpp, go
        --no-external      Hide external (third-party) dependencies
        --show-external    Show external package names as leaf nodes (tree/table)
    -o, --output <file>    Write output to file instead of stdout
        --no-progress      Disable progress bar (recommended for CI/pipes)

kgr check [PATH] [FLAGS]
  Check for dependency issues: cycles, orphans, rule violations.
  Exit 0 = clean; exit 1 = errors found.
  Flags:
    -f, --format <fmt>     Output format: text (default), json
        --no-progress      Disable progress bar
        --update-baseline  Record current violations as baseline (exits 0)
        --baseline <file>  Path to baseline file (default: .kgr-baseline.json)
  JSON output shape:
    {
      "ok": bool,
      "cycles": [["a.py", "b.py"], ...],
      "orphans": ["unused.py", ...],
      "rule_violations": [{"rule": "...", "from": "...", "to": "...", "severity": "error|warn"}],
      "suppressed": <int>
    }

kgr query [PATH] [FLAGS]
  Query the graph without printing the full structure.
  Flags:
    --who-imports <file>   List files that import the given file
    --deps-of <file>       List all transitive dependencies of a file
    --path-between <a> <b> Shortest dependency path between two files
    --cycles               List all cycles
    --orphans              List orphaned files
    --heaviest             List files ranked by number of dependents
    --largest-cycle        Show the largest cycle
    -f, --format <fmt>     Output format: table (default), json

kgr init [PATH]
  Generate a .kgr.toml config skeleton in PATH (default: .).
  Detects languages present and emits commented rule examples.

kgr upgrade
  Pull latest source from git and rebuild kgr in-place.
  Requires the repo to be present at the path baked in at compile time.

kgr agent-info [FLAGS]
  Print this guide.
  Flags:
    -f, --format <fmt>     Output format: text (default), json

OUTPUT FORMATS (kgr graph)
--------------------------
  tree     ASCII tree rooted at entry points (default)
  json     Full DepGraph as JSON; includes files, edges, cycles, orphans,
           roots, external_deps map
  table    Per-file summary: lang, local-in, local-out, ext count, status
  dot      Graphviz DOT — pipe to `dot -Tsvg` for a visual
  mermaid  Mermaid flowchart — paste into mermaid.live

.kgr.toml CONFIGURATION
------------------------
  [[rules]]
  name     = "no-legacy-to-core"
  from     = "legacy/**"          # glob matched against relative paths
  to       = "core/**"
  severity = "error"              # or "warn"

  Severity "error" → exit 1; "warn" → exit 0 but printed to stderr.
  Use --update-baseline to suppress known violations during migration.

RECOMMENDED AGENT WORKFLOW
--------------------------
  1. Run `kgr check --format json --no-progress .` to get structured status.
     Parse "ok" to branch: if true, no action needed.
  2. Inspect "cycles" and "rule_violations" arrays for specific violations.
  3. Use `kgr query --who-imports <file>` or `--deps-of <file>` to trace
     dependency paths relevant to a violation.
  4. Use `kgr graph --format json --no-progress .` to get the full graph
     for broader analysis; the "external_deps" map lists third-party packages
     per file.
  5. Always pass --no-progress when parsing output programmatically.
"#;
