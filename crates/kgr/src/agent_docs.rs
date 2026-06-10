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
    -l, --lang <lang>      Filter by language: py, ts, js, rs, java, c, cpp, go,
                           zig, cs, objc, swift, rb, php, scala, lua, ex, hs, sh
        --no-external      Hide external (third-party) dependencies
        --show-external    Show external package names as leaf nodes (tree/table)
        --symbols          Include symbol definitions in each file node (JSON only)
    -o, --output <file>    Write output to file instead of stdout
        --no-progress      Disable progress bar (recommended for CI/pipes)

kgr check [PATH] [FLAGS]
  Check for dependency issues: cycles, orphans, rule violations.
  Exit 0 = clean; exit 1 = errors found.
  Flags:
    -f, --format <fmt>     Output format: text (default), json
    -l, --lang <lang>      Filter by language
        --no-progress      Disable progress bar
        --update-baseline  Record current violations as baseline (exits 0)
        --baseline <file>  Path to baseline file (default: .kgr-baseline.json)
        --syntax           Include tree-sitter ERROR/MISSING parse diagnostics
  JSON output shape:
    {
      "ok": bool,
      "cycles": [["a.py", "b.py"], ...],
      "orphans": ["unused.py", ...],
      "rule_violations": [{"rule": "...", "from": "...", "to": "...", "severity": "error|warn"}],
      "suppressed": <int>
    }
    With --syntax, JSON also includes:
      "syntax_errors": [{"file": "bad.py", "message": "...", "line": 3, "column": 8}]

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
    -l, --lang <lang>      Filter by language
        --no-progress      Disable progress bar

kgr symbols [PATH] [FLAGS]
  List all symbol definitions (functions, classes, methods) in the scanned files.
  Flags:
    -f, --format <fmt>     Output format: table (default), json
    -l, --lang <lang>      Filter by language
        --no-progress      Disable progress bar
  JSON output shape:
    [{
      "file": "src/utils.py",
      "symbols": [
        {"name": "normalize", "kind": "function", "line": 5, "exported": true},
        {"name": "MyClass", "kind": "class", "line": 12, "exported": true}
      ]
    }, ...]

kgr refs <NAME> [PATH] [FLAGS]
  Find all definitions and call-site references for a symbol by name.
  Flags:
    -f, --format <fmt>     Output format: table (default), json
    -l, --lang <lang>      Filter by language
        --no-progress      Disable progress bar
  JSON output shape:
    {
      "symbol": "normalize",
      "definitions": [{"file": "utils.py", "line": 5, "kind": "function"}],
      "references": [{"file": "app.py", "line": 3, "context": "  result = normalize(data)", "kind": "call"}]
    }

kgr dead <NAME> [PATH] [FLAGS]
  Check if a symbol is dead code (defined but never referenced).
  Flags:
    -f, --format <fmt>     Output format: table (default), json
    -l, --lang <lang>      Filter by language
        --no-progress      Disable progress bar
  JSON output shape:
    {
      "symbol": "old_helper",
      "found": true,
      "dead": true,
      "definitions": [{"file": "utils.py", "line": 42, "kind": "function"}],
      "references": []
    }
  "definitions" lists every definition of the name — a symbol defined in
  several files yields several entries, and "dead"/"references" are computed
  across all of them.
  If the symbol is not defined anywhere, "found" is false and "dead" is null
  (NOT true) — a not-found symbol is never a removable verdict:
    {"symbol": "no_such", "found": false, "dead": null, "definitions": [], "references": []}

kgr skeleton [PATH] [FLAGS]
  Emit a token-minimal skeleton of each file: signatures only, bodies elided.
  Flags:
    -f, --format <fmt>     Output format: text (default), json, table
    -l, --lang <lang>      Filter by language
        --no-progress      Disable progress bar
  JSON output shape:
    [{
      "file": "src/service.py",
      "skeleton": [
        {"name": "fetch_users", "kind": "function", "line": 8, "exported": true, "signature": "def fetch_users(): ..."}
      ]
    }, ...]

kgr orient [PATH] [FLAGS]
  Print a one-shot codebase overview: file counts, languages, entry points,
  heaviest files, cycles, orphans, and external packages.
  Flags:
    -f, --format <fmt>     Output format: text (default), json
    -l, --lang <lang>      Filter by language
        --no-progress      Disable progress bar
  JSON output shape:
    {
      "files": 12,
      "languages": {"rust": 8, "python": 4},
      "edges": 18,
      "entry_points": ["src/main.rs"],
      "cycles": 0,
      "largest_cycle_size": 0,
      "orphans": 1,
      "external_packages": ["serde"],
      "heaviest": [{"file": "src/lib.rs", "dependents": 4}]
    }

kgr impact <NAME> [PATH] [FLAGS]
  Show the transitive blast radius of a symbol change.
  Flags:
    -f, --format <fmt>     Output format: text (default), json
    -l, --lang <lang>      Filter by language
    -d, --depth <n>        Maximum dependent depth to traverse
        --no-progress      Disable progress bar
  JSON output shape:
    {
      "symbol": "query",
      "found": true,
      "definitions": [{"file": "db.ts", "line": 3, "kind": "function"}],
      "impact": [{"file": "service.ts", "depth": 1, "calls_symbol": true}]
    }
  "definitions" lists every definition of the name; "impact" is the union of
  transitive dependents of every defining file (minimum depth wins when a
  dependent is reachable from several definitions).
  If the symbol is not found:
    {"symbol": "x", "found": false, "definitions": [], "impact": [], "error": "Symbol 'x' not found"}

kgr hotspots [PATH] [FLAGS]
  Rank files by function count and average function length.
  Flags:
    -f, --format <fmt>     Output format: table (default), json, text
    -l, --lang <lang>      Filter by language
    -t, --top <n>          Number of files to show (default: 20)
        --no-progress      Disable progress bar
  JSON output shape:
    [{
      "file": "src/service.py",
      "functions": 5,
      "avg_length": 12,
      "max_length": 31,
      "score": 60
    }, ...]

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
  5. Use `kgr refs <name> --format json --no-progress .` to find all usages
     of a function/class — replaces multi-step grep+read workflows.
  6. Use `kgr dead <name> --format json --no-progress .` to check if a
     symbol is safe to remove before deleting it. Check "found" first:
     "found": false means the symbol does not exist in the parsed project
     (typo or unparsed language) — it does NOT mean the symbol is removable.
  7. Use `kgr symbols --format json --no-progress .` to get a table of
     contents of all definitions — useful for orientation in unfamiliar code.
  8. Always pass --no-progress when parsing output programmatically.
"#;
