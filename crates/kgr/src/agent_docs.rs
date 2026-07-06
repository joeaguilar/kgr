/// Static plain-text guide for AI agents. Embedded at compile time.
pub const AGENT_DOCS: &str = r#"
kgr — polyglot source dependency knowledge graph
-------------------------------------------------

SUBCOMMANDS
-----------

kgr graph [PATH] [FLAGS]
  Scan PATH (default: .) and emit the full dependency graph.
  Bare `kgr` is equivalent to `kgr graph .` with default flags.
  Flags:
    -f, --format <fmt>     Output format: tree (default), json, table, dot, mermaid
    -l, --lang <lang>      Filter by language: py, ts, js, rs, java, c, cpp, go,
                           zig, cs, objc, swift, rb, php, scala, lua, ex, hs, sh
        --no-external      Hide external (third-party) dependencies
        --show-external    Show external package names as leaf nodes (tree/table)
        --first-party      Exclude vendored paths from the orphan summary
                           (files stay in the graph; see FIRST-PARTY FILTERING)
        --symbols          Include symbol definitions in each file node (JSON only)
    -o, --output <file>    Write output to file instead of stdout
        --no-progress      Disable progress bar (recommended for CI/pipes)

kgr check [PATH] [FLAGS]
  Check for dependency issues: cycles, orphans, rule violations.
  Exit 0 = clean; exit 1 = errors found; exit 2 = operational failure
  (bad path, invalid format, broken config/baseline).
  Flags:
    -f, --format <fmt>     Output format: text (default), json
    -l, --lang <lang>      Filter by language
        --first-party      Exclude vendored paths from the orphan summary
                           (cycles and rule violations always report in full)
        --no-progress      Disable progress bar
        --update-baseline  Record current violations as baseline (exits 0)
        --baseline <file>  Path to baseline file (default: .kgr-baseline.json)
        --syntax           Include tree-sitter ERROR/MISSING parse diagnostics
        --exit-zero        Report-only mode: identical diagnostics and JSON
                           (including "ok": false), but exit 0 even when
                           cycles or error-severity rule violations are found.
                           Operational failures still exit nonzero. Use this
                           when batching check with other commands — no need
                           for `|| true` shell workarounds.
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
    With first-party filtering active, JSON also includes:
      "first_party_filter": {"vendor_globs": [...], "excluded_orphans": <int>}
    When the walk skipped unsupported files, JSON also includes
      "skipped_unsupported" (see SKIPPED UNSUPPORTED FILES)

kgr query [PATH] [FLAGS]
  Query the graph without printing the full structure.
  Flags:
    --who-imports <file>   List files that import the given file
    --deps-of <file>       List all transitive dependencies of a file
    --path-between <a> <b> Shortest dependency path between two files
    --cycles               List all cycles
    --orphans              List orphaned files
    --heaviest             List files ranked by number of dependents
    -t, --top <n>          Show top N files for --heaviest (default: 20)
    --largest-cycle        Show the largest cycle
    --first-party          Exclude vendored paths from --orphans and --heaviest
    -f, --format <fmt>     Output format: table (default), json
    -l, --lang <lang>      Filter by language
        --no-progress      Disable progress bar
  JSON shape note: --orphans and --heaviest emit a bare array by default.
  With first-party filtering active they switch to an object:
    {"orphans": [...], "first_party_filter": {"vendor_globs": [...], "excluded_orphans": <int>}}
    {"heaviest": [...], "first_party_filter": {"vendor_globs": [...], "excluded_files": <int>}}

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
        {"name": "normalize", "kind": "function", "line": 5, "end_line": 18, "exported": true},
        {"name": "MyClass", "kind": "class", "line": 22, "end_line": 40, "exported": true}
      ]
    }, ...]
  "line".."end_line" is the definition extent: signature through body,
  including Python decorators. Leading attribute/doc-comment lines that sit
  OUTSIDE the definition node (e.g. Rust #[attr] and ///) are not included —
  use `kgr show -c <n>` to pull them in.

kgr refs <NAME> [PATH] [FLAGS]
  Find all definitions and call-site references for a symbol by name.
  Flags:
    -f, --format <fmt>     Output format: table (default), json
    -l, --lang <lang>      Filter by language
        --no-progress      Disable progress bar
  JSON output shape:
    {
      "symbol": "normalize",
      "definitions": [{"file": "utils.py", "line": 5, "end_line": 18, "kind": "function"}],
      "references": [{"file": "app.py", "line": 3, "context": "  result = normalize(data)", "kind": "call"}]
    }

kgr show <NAME> [PATH] [FLAGS]
  Print the definition body of a symbol, straight from source, located via
  the index. Replaces grep -n + sed -n chains.
  Flags:
    -c, --context <n>      Include n lines before/after the definition (default 0)
        --all              Print every match (default: first match + one-line
                           pointers like `also: src/other.rs:88 (method)`)
    -k, --kind <kind>      Disambiguate same-named symbols: fn, class, method
    -f, --format <fmt>     Output format: text (default), json
        --no-linenos       Raw body, pipe-friendly
  Exit 1 with near-miss suggestions when the symbol is not found.
  Printed paths (the header and `also:` pointers) are relative to the
  scanned PATH, while `kgr slice` resolves its file argument against the
  current directory — when scanning a subdirectory, prefix pointers with
  that PATH before slicing them.
  JSON output shape (array, one entry per match; "body" is null for matches
  not printed under the default first-match mode):
    [{"name": "render_table", "kind": "function", "path": "src/render/table.rs",
      "start_line": 6, "end_line": 41, "exported": true, "body": "pub fn ..."}]

kgr slice <FILE>:<START>[-<END>] [FLAGS]
  Print a numbered, bounded line window from any file — no index, works on
  files kgr does not parse (.toml, .md, logs). Replaces sed -n 'X,Yp'.
  Also accepts `kgr slice <file> <start> [<end>]` positionally.
  Flags:
    -c, --context <n>      Expand a single-line target both ways (default 10
                           when no end given)
        --no-linenos       Raw text (byte-identical to the source window)
    -f, --format <fmt>     Output format: text (default), json
        --max <n>          Raise the output cap (default 500 lines)
  Out-of-range end clamps to EOF with a note; nonexistent file exits 2.
  JSON output shape:
    {"path": "src/lib.rs", "start_line": 10, "end_line": 40, "lines": ["...", ...]}

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
  When the walk skipped unsupported files, JSON also includes
  "skipped_unsupported" and text output adds a one-line summary
  (see SKIPPED UNSUPPORTED FILES).

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
           roots, external_deps map (plus skipped_unsupported when the walk
           skipped unsupported files — see SKIPPED UNSUPPORTED FILES)
  table    Per-file summary: lang, local-in, local-out, ext count, status
  dot      Graphviz DOT — pipe to `dot -Tsvg` for a visual
  mermaid  Mermaid flowchart — paste into mermaid.live

FIRST-PARTY FILTERING
---------------------
  `--first-party` (on graph, check, and query) excludes vendored paths from
  orphan and heaviest summaries. Vendored files STAY in the graph — only the
  summaries are filtered. Cycles and rule violations always report in full.
  Default vendor globs: **/vendor/**, **/third_party/**, **/external/**
  Config equivalents (.kgr.toml):
    first_party  = true                  # same as passing --first-party
    vendor_globs = ["third_party/**"]    # custom list REPLACES the defaults
  The "first_party_filter" JSON key appears ONLY when the filter is active,
  so default JSON output shapes are unchanged.

SKIPPED UNSUPPORTED FILES
-------------------------
  Files in languages kgr cannot parse (Kotlin, Perl, Vue, ...) are skipped,
  summarized on stderr, and reported in graph/check/orient JSON as
    "skipped_unsupported": [{"group": "kt", "count": 4, "sample": ["a.kt", ...]}]
  grouped by extension ("(no extension)" for extensionless text files),
  largest group first, with a small bounded path sample per group. The key
  appears ONLY when at least one unsupported file was skipped, so
  fully-supported repos keep their JSON shapes unchanged. Obvious non-source
  files (data/config, docs, lockfiles, images, archives, binaries) are never
  reported. Files excluded by --lang are filtered, not skipped — a supported
  language outside your filter never shows up here. Graph coverage is
  partial whenever this key is present: fall back to grep for content inside
  the listed files.

.kgr.toml CONFIGURATION
------------------------
  [[rules]]
  name     = "no-legacy-to-core"
  from     = "legacy/**"          # glob matched against relative paths
  to       = "core/**"
  severity = "error"              # or "warn"

  Severity "error" → exit 1; "warn" → exit 0 but printed to stderr.
  Use --update-baseline to suppress known violations during migration.

  Top-level keys (selection): first_party = true, vendor_globs = [...]
  (see FIRST-PARTY FILTERING above).

RECOMMENDED AGENT WORKFLOW
--------------------------
  1. Run `kgr check --format json --no-progress .` to get structured status.
     Parse "ok" to branch: if true, no action needed.
     When batching check with other commands (or running under a harness
     that aborts on nonzero exits), add --exit-zero: same JSON, exit 0 on
     findings. Do NOT use `|| true` — it also masks real operational errors.
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
  8. Use `kgr show <name>` to print a definition body instead of
     grep -n + sed -n; use `kgr slice <file>:<start>-<end>` for arbitrary
     line windows (works on any file, not just parsed languages).
  9. Always pass --no-progress when parsing output programmatically.
"#;
