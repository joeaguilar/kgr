use predicates::prelude::*;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures")
}

/// Build a `kgr` command with the parse cache disabled (`KGR_NO_CACHE=1`), so
/// every invocation re-parses sources and never reads or writes
/// `.kgr-cache.json`. This keeps fixture-driven tests hermetic: a stale warm
/// cache under tests/fixtures/ can never mask a parser regression, and test
/// runs leave no cache files behind. Tests that exercise the cache itself
/// build their command with `cargo_bin_cmd!` directly.
fn kgr() -> assert_cmd::Command {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("kgr");
    cmd.env("KGR_NO_CACHE", "1");
    cmd
}

#[test]
fn version_flag() {
    kgr()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("kgr"));
}

#[test]
fn python_simple_json() {
    let fixture = fixtures_dir().join("python/simple");
    let output = kgr()
        .args(["graph", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success();

    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert!(json["files"].is_array());
    assert!(json["edges"].is_array());
    assert!(json["roots"].is_array());
    assert!(json["cycles"].is_array());
}

#[test]
fn typescript_simple_json() {
    let fixture = fixtures_dir().join("typescript/simple");
    let output = kgr()
        .args(["graph", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success();

    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let files = json["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
}

#[test]
fn typescript_cycle_check_exits_1() {
    let fixture = fixtures_dir().join("typescript/cycle");
    kgr()
        .args(["check", "--no-progress"])
        .arg(&fixture)
        .assert()
        .failure()
        .stderr(predicate::str::contains("cycle"));
}

#[test]
fn javascript_simple_json() {
    let fixture = fixtures_dir().join("javascript/simple");
    let output = kgr()
        .args(["graph", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success();

    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let files = json["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
}

#[test]
fn tree_output_format() {
    let fixture = fixtures_dir().join("typescript/simple");
    kgr()
        .args(["graph", "--format", "tree", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("main.ts"));
}

#[test]
fn tree_output_lists_cycles_when_graph_has_no_roots() {
    let fixture = fixtures_dir().join("typescript/cycle");
    let output = kgr()
        .args(["graph", "--format", "tree", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success();

    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(
        !stdout.contains("(no entry points found)"),
        "fully-cyclic tree must not bail out:\n{stdout}"
    );
    assert!(
        stdout.contains("Cycles:"),
        "missing Cycles section:\n{stdout}"
    );
    for member in ["a.ts", "b.ts", "c.ts"] {
        assert!(
            stdout.contains(member),
            "missing cycle member {member}:\n{stdout}"
        );
    }
}

#[test]
fn dot_output_format() {
    let fixture = fixtures_dir().join("typescript/simple");
    kgr()
        .args(["graph", "--format", "dot", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("digraph kgr"));
}

#[test]
fn graph_help_lists_all_formats() {
    kgr()
        .args(["graph", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Output format: tree, json, table, dot, mermaid",
        ));
}

#[test]
fn graph_unknown_format_exits_nonzero_on_stderr() {
    let fixture = fixtures_dir().join("typescript/simple");
    kgr()
        .args(["graph", "--format", "bogus", "--no-progress"])
        .arg(&fixture)
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("Unknown format: bogus"));
}

#[test]
fn graph_json_with_symbols_includes_external_deps() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("app.py"),
        "import requests\n\n\ndef main():\n    return requests.get('https://example.com')\n",
    )
    .unwrap();

    let output = kgr()
        .args(["graph", "--format", "json", "--symbols", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let external_deps = json["external_deps"]
        .as_object()
        .expect("external_deps should be present");
    assert!(
        external_deps.values().any(|deps| {
            deps.as_array()
                .unwrap()
                .iter()
                .any(|dep| dep.as_str() == Some("requests"))
        }),
        "external_deps should include requests"
    );
    let files = json["files"].as_array().unwrap();
    assert!(
        files.iter().any(|file| file["symbols"]
            .as_array()
            .is_some_and(|symbols| !symbols.is_empty())),
        "files should still include symbol arrays"
    );
}

#[test]
fn init_creates_config() {
    let tmp = tempfile::tempdir().unwrap();
    // Create a dummy .py file so init detects python
    std::fs::write(tmp.path().join("test.py"), "import os\n").unwrap();

    kgr()
        .args(["init"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(".kgr.toml"));

    assert!(tmp.path().join(".kgr.toml").exists());
    let content = std::fs::read_to_string(tmp.path().join(".kgr.toml")).unwrap();
    assert!(content.contains("py"));
    assert!(content.contains("[[rules]]"));
}

#[test]
fn malformed_config_exits_nonzero_and_reports_path() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("main.py"), "import os\n").unwrap();
    std::fs::write(tmp.path().join(".kgr.toml"), "max_file_size_kb = \"500\"\n").unwrap();

    kgr()
        .args(["graph", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to load config"))
        .stderr(predicate::str::contains(".kgr.toml"))
        .stderr(predicate::str::contains("max_file_size_kb"));
}

// ── Rule system ──────────────────────────────────────────────────────────────

fn make_ts_fixture(tmp: &tempfile::TempDir) {
    // core/db.ts  (no imports)
    std::fs::create_dir_all(tmp.path().join("core")).unwrap();
    std::fs::write(tmp.path().join("core/db.ts"), "export const db = {};\n").unwrap();

    // legacy/repo.ts  imports core/db.ts
    std::fs::create_dir_all(tmp.path().join("legacy")).unwrap();
    std::fs::write(
        tmp.path().join("legacy/repo.ts"),
        "import { db } from '../core/db';\n",
    )
    .unwrap();
}

#[test]
fn rule_violation_exits_1() {
    let tmp = tempfile::tempdir().unwrap();
    make_ts_fixture(&tmp);

    // Forbid legacy -> core
    std::fs::write(
        tmp.path().join(".kgr.toml"),
        r#"
[[rules]]
name = "no-legacy-to-core"
from = "legacy/**"
to   = "core/**"
severity = "error"
"#,
    )
    .unwrap();

    kgr()
        .args(["check", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("no-legacy-to-core"));
}

#[test]
fn rule_warn_severity_exits_0() {
    let tmp = tempfile::tempdir().unwrap();
    make_ts_fixture(&tmp);

    std::fs::write(
        tmp.path().join(".kgr.toml"),
        r#"
[[rules]]
name = "warn-legacy-to-core"
from = "legacy/**"
to   = "core/**"
severity = "warn"
"#,
    )
    .unwrap();

    kgr()
        .args(["check", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("warn-legacy-to-core"));
}

#[test]
fn rule_no_match_exits_0() {
    let tmp = tempfile::tempdir().unwrap();
    make_ts_fixture(&tmp);

    // Rule that doesn't match the actual edges
    std::fs::write(
        tmp.path().join(".kgr.toml"),
        r#"
[[rules]]
name = "no-api-to-db"
from = "api/**"
to   = "db/**"
severity = "error"
"#,
    )
    .unwrap();

    kgr()
        .args(["check", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("All checks passed."));
}

#[test]
fn invalid_rule_glob_exits_nonzero_and_reports_rule() {
    let tmp = tempfile::tempdir().unwrap();
    make_ts_fixture(&tmp);

    std::fs::write(
        tmp.path().join(".kgr.toml"),
        r#"
[[rules]]
name = "bad-glob"
from = "legacy/["
to   = "core/**"
severity = "error"
"#,
    )
    .unwrap();

    kgr()
        .args(["check", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("warning[kgr::rule-config]"))
        .stderr(predicate::str::contains("bad-glob"))
        .stderr(predicate::str::contains("legacy/["));
}

// ── Baseline enforcement ──────────────────────────────────────────────────────

#[test]
fn baseline_update_exits_0_and_writes_file() {
    let tmp = tempfile::tempdir().unwrap();
    make_ts_fixture(&tmp);
    std::fs::write(
        tmp.path().join(".kgr.toml"),
        "[[rules]]\nname=\"no-legacy\"\nfrom=\"legacy/**\"\nto=\"core/**\"\nseverity=\"error\"\n",
    )
    .unwrap();

    // Running with a violation normally exits 1, but --update-baseline should exit 0
    kgr()
        .args(["check", "--no-progress", "--update-baseline"])
        .arg(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("Baseline updated"));

    assert!(tmp.path().join(".kgr-baseline.json").exists());
}

#[test]
fn baseline_suppresses_known_violation() {
    let tmp = tempfile::tempdir().unwrap();
    make_ts_fixture(&tmp);
    std::fs::write(
        tmp.path().join(".kgr.toml"),
        "[[rules]]\nname=\"no-legacy\"\nfrom=\"legacy/**\"\nto=\"core/**\"\nseverity=\"error\"\n",
    )
    .unwrap();

    // Record the violation
    kgr()
        .args(["check", "--no-progress", "--update-baseline"])
        .arg(tmp.path())
        .assert()
        .success();

    // Now check — should pass because all violations are in baseline
    kgr()
        .args(["check", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("suppressed by baseline"));
}

#[test]
fn baseline_fails_on_new_violation() {
    let tmp = tempfile::tempdir().unwrap();
    make_ts_fixture(&tmp);

    // Baseline with a rule that matches nothing (empty baseline effectively)
    std::fs::write(
        tmp.path().join(".kgr.toml"),
        "[[rules]]\nname=\"no-api\"\nfrom=\"api/**\"\nto=\"db/**\"\nseverity=\"error\"\n",
    )
    .unwrap();
    kgr()
        .args(["check", "--no-progress", "--update-baseline"])
        .arg(tmp.path())
        .assert()
        .success();

    // Swap in a rule that DOES match — new violation not in baseline
    std::fs::write(
        tmp.path().join(".kgr.toml"),
        "[[rules]]\nname=\"no-legacy\"\nfrom=\"legacy/**\"\nto=\"core/**\"\nseverity=\"error\"\n",
    )
    .unwrap();
    kgr()
        .args(["check", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("no-legacy"));
}

// ── JSON format for check ─────────────────────────────────────────────────────

#[test]
fn check_json_ok_no_violations() {
    let tmp = tempfile::tempdir().unwrap();
    make_ts_fixture(&tmp);

    let output = kgr()
        .args(["check", "--format", "json", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["ok"], true);
    assert!(json["cycles"].as_array().unwrap().is_empty());
    assert!(json["rule_violations"].as_array().unwrap().is_empty());
}

#[test]
fn check_json_rule_violation_exits_1() {
    let tmp = tempfile::tempdir().unwrap();
    make_ts_fixture(&tmp);

    std::fs::write(
        tmp.path().join(".kgr.toml"),
        "[[rules]]\nname=\"no-legacy-to-core\"\nfrom=\"legacy/**\"\nto=\"core/**\"\nseverity=\"error\"\n",
    )
    .unwrap();

    let output = kgr()
        .args(["check", "--format", "json", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["ok"], false);
    let violations = json["rule_violations"].as_array().unwrap();
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0]["rule"], "no-legacy-to-core");
    assert_eq!(violations[0]["severity"], "error");
}

#[test]
fn check_json_orphans_reported() {
    let fixture = fixtures_dir().join("python/simple");

    let output = kgr()
        .args(["check", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["ok"], true);
    let orphans = json["orphans"].as_array().unwrap();
    assert!(!orphans.is_empty());
}

// ── Single-file PATH support ──────────────────────────────────────────────────

#[test]
fn graph_single_file_json_contains_the_file() {
    let fixture = fixtures_dir().join("python/simple/main.py");
    let output = kgr()
        .args(["graph", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let files = json["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "single-file scan should contain one file");
    assert_eq!(files[0]["path"], "main.py");
}

#[test]
fn graph_single_file_tree_is_not_empty() {
    let fixture = fixtures_dir().join("typescript/simple/main.ts");
    kgr()
        .args(["graph", "--format", "tree", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("main.ts"));
}

#[test]
fn symbols_single_file_lists_symbols() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("app.py");
    std::fs::write(&file, "def handler():\n    return 42\n").unwrap();

    kgr()
        .args(["symbols", "--no-progress"])
        .arg(&file)
        .assert()
        .success()
        .stdout(predicate::str::contains("app.py"))
        .stdout(predicate::str::contains("handler"));
}

#[test]
fn skeleton_single_file_emits_stubs() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("app.py");
    std::fs::write(&file, "def handler():\n    return 42\n").unwrap();

    kgr()
        .args(["skeleton", "--no-progress"])
        .arg(&file)
        .assert()
        .success()
        .stdout(predicate::str::contains("app.py"))
        .stdout(predicate::str::contains("def handler"));
}

#[test]
fn check_single_file_runs_checks() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("app.py");
    std::fs::write(&file, "import os\n").unwrap();

    kgr()
        .args(["check", "--no-progress"])
        .arg(&file)
        .assert()
        .success()
        .stderr(predicate::str::contains("All checks passed."));
}

#[test]
fn single_file_unsupported_extension_rejected_with_error() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("notes.txt");
    std::fs::write(&file, "hello\n").unwrap();

    kgr()
        .args(["graph", "--no-progress"])
        .arg(&file)
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("unsupported file type"));
}

#[test]
fn single_file_lang_filter_mismatch_rejected_with_error() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("app.py");
    std::fs::write(&file, "def handler():\n    return 42\n").unwrap();

    kgr()
        .args(["symbols", "--no-progress", "--lang", "ts"])
        .arg(&file)
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("excluded by --lang filter"));
}

// ── Cache hermeticity ─────────────────────────────────────────────────────────

#[test]
fn no_cache_env_prevents_cache_write() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("main.py"), "import helper\n").unwrap();
    std::fs::write(tmp.path().join("helper.py"), "x = 1\n").unwrap();

    kgr()
        .args(["graph", "--format", "json", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success();

    assert!(
        !tmp.path().join(".kgr-cache.json").exists(),
        "a KGR_NO_CACHE=1 run must not write a cache file"
    );
}

/// Demonstrates the masking hole is closed: a poisoned warm cache steers the
/// output of a cache-enabled run (hits skip parsing — by design), but a
/// KGR_NO_CACHE run re-parses sources and reports the truth. Since every
/// fixture-driven test runs with KGR_NO_CACHE, a stale cache can never mask a
/// parser regression in the test suite.
#[test]
fn poisoned_warm_cache_cannot_mask_parser_regression_when_cache_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("main.py"), "import helper\n").unwrap();
    std::fs::write(tmp.path().join("helper.py"), "x = 1\n").unwrap();

    let edge_count = |stdout: &[u8]| -> usize {
        let json: serde_json::Value = serde_json::from_slice(stdout).unwrap();
        json["edges"].as_array().unwrap().len()
    };
    let graph_json = |cache_enabled: bool| -> Vec<u8> {
        let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("kgr");
        if cache_enabled {
            cmd.env_remove("KGR_NO_CACHE");
        } else {
            cmd.env("KGR_NO_CACHE", "1");
        }
        cmd.args(["graph", "--format", "json", "--no-progress"])
            .arg(tmp.path())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone()
    };

    // 1. Warm the cache with a correct parse.
    let out = graph_json(true);
    assert_eq!(
        edge_count(&out),
        1,
        "fresh parse should see main.py -> helper.py"
    );
    let cache_path = tmp.path().join(".kgr-cache.json");
    assert!(
        cache_path.exists(),
        "cache-enabled run should write a cache"
    );

    // 2. Poison the cache: erase every cached import while keeping the
    //    (mtime, size) keys valid — simulating a stale cache that no longer
    //    matches what the parser would produce.
    let mut cache: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&cache_path).unwrap()).unwrap();
    for entry in cache["entries"].as_object_mut().unwrap().values_mut() {
        entry["imports"] = serde_json::json!([]);
    }
    std::fs::write(&cache_path, serde_json::to_vec(&cache).unwrap()).unwrap();

    // 3. A cache-enabled run follows the poisoned cache — the masking hazard
    //    this suite must never be exposed to.
    let out = graph_json(true);
    assert_eq!(
        edge_count(&out),
        0,
        "cache hit skips parsing (by design), so the poisoned data wins"
    );

    // 4. KGR_NO_CACHE defeats the stale cache: sources are re-parsed.
    let out = graph_json(false);
    assert_eq!(
        edge_count(&out),
        1,
        "a KGR_NO_CACHE run must re-parse and report the real import"
    );
}

/// The dev-machine scenario from the bug report: a warm cache left behind by
/// a *different build* of the same kgr package version. The cache version tag
/// embeds a binary fingerprint, so such a cache is discarded outright and the
/// sources are re-parsed — even with caching enabled and no version bump.
#[test]
fn stale_build_cache_is_discarded_even_with_cache_enabled() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("main.py"), "import helper\n").unwrap();
    std::fs::write(tmp.path().join("helper.py"), "x = 1\n").unwrap();

    let graph_edges = || -> usize {
        let out = assert_cmd::cargo::cargo_bin_cmd!("kgr")
            .env_remove("KGR_NO_CACHE")
            .args(["graph", "--format", "json", "--no-progress"])
            .arg(tmp.path())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
        json["edges"].as_array().unwrap().len()
    };

    // Warm the cache, then rewrite it as a poisoned cache from an "older
    // build": same package version, different build fingerprint, no imports.
    assert_eq!(graph_edges(), 1);
    let cache_path = tmp.path().join(".kgr-cache.json");
    let mut cache: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&cache_path).unwrap()).unwrap();
    cache["version"] = serde_json::json!(format!("{}+stale-build", env!("CARGO_PKG_VERSION")));
    for entry in cache["entries"].as_object_mut().unwrap().values_mut() {
        entry["imports"] = serde_json::json!([]);
    }
    std::fs::write(&cache_path, serde_json::to_vec(&cache).unwrap()).unwrap();

    assert_eq!(
        graph_edges(),
        1,
        "a cache from a different build must be discarded, not followed"
    );
}

// ── agent-info subcommand ─────────────────────────────────────────────────────

#[test]
fn agent_info_text() {
    kgr()
        .arg("agent-info")
        .assert()
        .success()
        .stdout(predicate::str::contains("SUBCOMMANDS"))
        .stdout(predicate::str::contains("kgr check"))
        .stdout(predicate::str::contains("RECOMMENDED AGENT WORKFLOW"));
}

#[test]
fn agent_info_documents_current_cli_surface() {
    kgr()
        .arg("agent-info")
        .assert()
        .success()
        .stdout(predicate::str::contains("kgr orient"))
        .stdout(predicate::str::contains("kgr impact <NAME>"))
        .stdout(predicate::str::contains("kgr hotspots"))
        .stdout(predicate::str::contains("kgr skeleton"))
        .stdout(predicate::str::contains("zig, cs, objc, swift"))
        .stdout(predicate::str::contains("--syntax"))
        .stdout(predicate::str::contains("syntax_errors"))
        .stdout(predicate::str::contains("-l, --lang <lang>"));
}

#[test]
fn agent_info_json() {
    let output = kgr()
        .args(["agent-info", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let guide = json["guide"].as_str().unwrap();
    assert!(guide.contains("SUBCOMMANDS"));
    assert!(guide.contains("kgr graph [PATH] [FLAGS]"));
    assert!(guide.contains("Bare `kgr` is equivalent to `kgr graph .`"));
    assert!(!guide.contains("kgr [graph] [PATH] [FLAGS]"));
}

// ── query subcommand ──────────────────────────────────────────────────────────

#[test]
fn query_who_imports_lists_direct_importer() {
    // typescript/simple: main.ts is the only direct importer of utils.ts.
    let fixture = fixtures_dir().join("typescript/simple");
    kgr()
        .args([
            "query",
            "--who-imports",
            "utils.ts",
            "-f",
            "table",
            "--no-progress",
        ])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("Files that import utils.ts"))
        .stdout(predicate::str::contains("main.ts"));
}

#[test]
fn query_who_imports_excludes_transitive_dependents() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("a.ts"),
        "import { b } from './b';\nexport const a = b;\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("b.ts"),
        "import { c } from './c';\nexport const b = c;\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("c.ts"), "export const c = 1;\n").unwrap();

    let output = kgr()
        .args([
            "query",
            "--who-imports",
            "c.ts",
            "-f",
            "json",
            "--no-progress",
        ])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let importers: Vec<&str> = json
        .as_array()
        .expect("who-imports JSON should be an array")
        .iter()
        .map(|entry| entry.as_str().unwrap())
        .collect();
    assert_eq!(importers, vec!["b.ts"]);
}

#[test]
fn query_deps_of_lists_dependency() {
    let fixture = fixtures_dir().join("typescript/simple");
    kgr()
        .args([
            "query",
            "--deps-of",
            "main.ts",
            "-f",
            "table",
            "--no-progress",
        ])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("Dependencies of main.ts"))
        .stdout(predicate::str::contains("utils.ts"));
}

#[test]
fn query_path_between_prints_import_chain() {
    // typescript/cycle: a.ts -> b.ts -> c.ts is the shortest path from a to c.
    // (python/cycle is unusable here until #16 lands: `from . import X`
    // resolves to __init__.py, so no a->b->c edges exist.)
    let fixture = fixtures_dir().join("typescript/cycle");
    kgr()
        .arg("query")
        .arg(&fixture)
        .args([
            "--path-between",
            "a.ts",
            "c.ts",
            "-f",
            "table",
            "--no-progress",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Shortest path from a.ts to c.ts"))
        .stdout(predicate::str::contains("a.ts -> b.ts -> c.ts"));
}

#[test]
fn query_cycles_lists_cycle_members() {
    let fixture = fixtures_dir().join("typescript/cycle");
    kgr()
        .args(["query", "--cycles", "-f", "table", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("Cycles found: 1"))
        .stdout(predicate::str::contains("a.ts"))
        .stdout(predicate::str::contains("b.ts"))
        .stdout(predicate::str::contains("c.ts"));
}

#[test]
fn query_orphans_lists_unconnected_file() {
    // A file with no imports that nothing imports is an orphan. Built in a
    // tempdir because the python fixtures' orphan sets are distorted by #16.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.ts"), "import { x } from './b';\n").unwrap();
    std::fs::write(tmp.path().join("b.ts"), "export const x = 1;\n").unwrap();
    std::fs::write(tmp.path().join("lonely.ts"), "export const y = 2;\n").unwrap();

    kgr()
        .args(["query", "--orphans", "-f", "table", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Orphaned files:"))
        .stdout(predicate::str::contains("lonely.ts"));
}

#[test]
fn query_empty_json_results_are_parseable() {
    let fixture = fixtures_dir().join("typescript/simple");
    let cases: &[&[&str]] = &[
        &["query", "--cycles", "-f", "json", "--no-progress"],
        &["query", "--orphans", "-f", "json", "--no-progress"],
        &[
            "query",
            "--who-imports",
            "main.ts",
            "-f",
            "json",
            "--no-progress",
        ],
    ];

    for args in cases {
        let output = kgr()
            .args(args.iter().copied())
            .arg(&fixture)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json, serde_json::json!([]));
    }
}

#[test]
fn query_empty_json_singleton_results_are_parseable() {
    let fixture = fixtures_dir().join("typescript/simple");
    let cases: &[&[&str]] = &[
        &["query", "--largest-cycle", "-f", "json", "--no-progress"],
        &[
            "query",
            "--path-between",
            "utils.ts",
            "main.ts",
            "-f",
            "json",
            "--no-progress",
        ],
    ];

    for args in cases {
        let output = kgr()
            .args(args.iter().copied())
            .arg(&fixture)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json, serde_json::Value::Null);
    }
}

#[test]
fn query_nonexistent_target_exits_nonzero() {
    let fixture = fixtures_dir().join("typescript/simple");
    kgr()
        .args([
            "query",
            "--who-imports",
            "missing.ts",
            "-f",
            "table",
            "--no-progress",
        ])
        .arg(&fixture)
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("unknown query target"))
        .stderr(predicate::str::contains("--who-imports"));
}

#[test]
fn query_nonexistent_target_json_reports_found_false() {
    let fixture = fixtures_dir().join("typescript/simple");
    let output = kgr()
        .args([
            "query",
            "--who-imports",
            "missing.ts",
            "-f",
            "json",
            "--no-progress",
        ])
        .arg(&fixture)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unknown query target"))
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["found"], false);
    assert_eq!(json["selector"], "who-imports");
    assert_eq!(json["error"], "unknown query target");
}

#[test]
fn query_target_paths_normalize_to_graph_keys() {
    let fixture = fixtures_dir().join("typescript/simple");
    kgr()
        .args([
            "query",
            "--who-imports",
            "./utils.ts",
            "-f",
            "table",
            "--no-progress",
        ])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("Files that import utils.ts"))
        .stdout(predicate::str::contains("main.ts"));

    kgr()
        .arg("query")
        .arg(&fixture)
        .arg("--deps-of")
        .arg(fixture.join("main.ts"))
        .args(["-f", "table", "--no-progress"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dependencies of main.ts"))
        .stdout(predicate::str::contains("utils.ts"));
}

#[test]
fn query_heaviest_json_ranks_files_by_dependents() {
    // typescript/simple: main.ts -> utils.ts is a real resolved edge
    // (python/simple's edges are distorted by #16).
    let fixture = fixtures_dir().join("typescript/simple");
    let output = kgr()
        .args(["query", "--heaviest", "-f", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let entries = json.as_array().expect("heaviest JSON should be an array");
    assert_eq!(entries.len(), 2, "both fixture files should be ranked");

    let dependents_of = |path: &str| -> u64 {
        entries
            .iter()
            .find(|e| e["path"] == path)
            .unwrap_or_else(|| panic!("{path} missing from heaviest output"))["dependents"]
            .as_u64()
            .unwrap()
    };
    assert_eq!(dependents_of("utils.ts"), 1, "main.ts imports utils.ts");
    assert_eq!(dependents_of("main.ts"), 0, "nothing imports main.ts");
}

#[test]
fn query_heaviest_top_flag_limits_json_entries() {
    let tmp = tempfile::tempdir().unwrap();
    for i in 0..25 {
        std::fs::write(
            tmp.path().join(format!("file{i:02}.ts")),
            format!("export const value{i} = {i};\n"),
        )
        .unwrap();
    }

    let entries = |extra_args: &[&str]| -> Vec<serde_json::Value> {
        let mut cmd = kgr();
        cmd.args(["query", "--heaviest", "-f", "json", "--no-progress"]);
        cmd.args(extra_args);
        cmd.arg(tmp.path());
        let output = cmd.assert().success().get_output().stdout.clone();

        serde_json::from_slice(&output).unwrap()
    };

    assert_eq!(entries(&[]).len(), 20, "default heaviest limit is 20");
    assert_eq!(entries(&["--top", "3"]).len(), 3);
}

#[test]
fn query_largest_cycle_lists_all_members() {
    let fixture = fixtures_dir().join("typescript/cycle");
    kgr()
        .args(["query", "--largest-cycle", "-f", "table", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("Largest cycle (3 files):"))
        .stdout(predicate::str::contains("a.ts"))
        .stdout(predicate::str::contains("b.ts"))
        .stdout(predicate::str::contains("c.ts"));
}

#[test]
fn query_without_mode_flag_exits_2_with_guidance() {
    let fixture = fixtures_dir().join("typescript/simple");
    kgr()
        .args(["query", "-f", "table", "--no-progress"])
        .arg(&fixture)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Usage:"))
        .stderr(predicate::str::contains("--who-imports"));
}

#[test]
fn query_rejects_multiple_selector_flags() {
    let fixture = fixtures_dir().join("typescript/simple");
    kgr()
        .args(["query", "--cycles", "--orphans", "--no-progress"])
        .arg(&fixture)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be used with"));
}

// ── CLI flag coverage ─────────────────────────────────────────────────────────

#[test]
fn graph_output_flag_writes_file_instead_of_stdout() {
    let fixture = fixtures_dir().join("typescript/simple");
    let tmp = tempfile::tempdir().unwrap();
    let out_path = tmp.path().join("graph.json");

    kgr()
        .args(["graph", "-f", "json", "--no-progress", "--output"])
        .arg(&out_path)
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    let content = std::fs::read_to_string(&out_path).expect("--output must create the file");
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();
    let files = json["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert!(
        files.iter().any(|f| f["path"] == "main.ts"),
        "written graph should contain main.ts"
    );
}

#[test]
fn graph_no_external_flag_accepted_with_tree_output() {
    // KNOWN BUG #20: --no-external is currently a no-op for tree output
    // (externals never appear as tree nodes unless --show-external). Assert
    // only that the flag is accepted and the local tree still renders — do
    // NOT assert external filtering here until #20 is fixed.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("main.py"),
        "import requests\nimport helper\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("helper.py"), "x = 1\n").unwrap();

    kgr()
        .args(["graph", "-f", "tree", "--no-external", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("main.py"))
        .stdout(predicate::str::contains("helper.py"));
}

#[test]
fn graph_show_external_annotates_packages_in_tree() {
    // Unlike --no-external (#20), the --show-external annotation works today.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("main.py"),
        "import requests\nimport helper\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("helper.py"), "x = 1\n").unwrap();

    kgr()
        .args(["graph", "-f", "tree", "--show-external", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("main.py"))
        .stdout(predicate::str::contains("requests [ext]"));
}

#[test]
fn lang_filter_restricts_scan_to_requested_language() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("app.py"), "import os\n").unwrap();
    std::fs::write(tmp.path().join("app.ts"), "export const x = 1;\n").unwrap();

    let output = kgr()
        .args(["graph", "-f", "json", "--lang", "py", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let files = json["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "--lang py must exclude the .ts file");
    assert_eq!(files[0]["path"], "app.py");
}

#[test]
fn check_syntax_flag_reports_parse_errors() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("broken.py"), "def broken(:\n    pass\n").unwrap();

    let output = kgr()
        .args(["check", "-f", "json", "--syntax", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let syntax_errors = json["syntax_errors"]
        .as_array()
        .expect("--syntax must add a syntax_errors array to JSON output");
    assert!(
        !syntax_errors.is_empty(),
        "malformed python should yield at least one syntax error"
    );
    assert!(
        syntax_errors.iter().all(|e| e["file"] == "broken.py"),
        "every reported error should point at broken.py"
    );
    assert!(syntax_errors[0]["line"].is_number());
}

#[test]
fn check_baseline_flag_uses_custom_path() {
    let tmp = tempfile::tempdir().unwrap();
    make_ts_fixture(&tmp);
    std::fs::write(
        tmp.path().join(".kgr.toml"),
        "[[rules]]\nname=\"no-legacy\"\nfrom=\"legacy/**\"\nto=\"core/**\"\nseverity=\"error\"\n",
    )
    .unwrap();
    let baseline_path = tmp.path().join("custom-baseline.json");

    // Record the violation into the custom baseline location.
    kgr()
        .args([
            "check",
            "-f",
            "text",
            "--no-progress",
            "--update-baseline",
            "--baseline",
        ])
        .arg(&baseline_path)
        .arg(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("Baseline updated"));

    assert!(
        baseline_path.exists(),
        "--baseline must control where the baseline is written"
    );
    assert!(
        !tmp.path().join(".kgr-baseline.json").exists(),
        "default baseline path must stay untouched when --baseline is given"
    );

    // Reading through --baseline suppresses the known violation...
    kgr()
        .args(["check", "-f", "text", "--no-progress", "--baseline"])
        .arg(&baseline_path)
        .arg(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("suppressed by baseline"));

    // ...while a run without --baseline (no file at the default path) fails,
    // proving the suppression came from the custom location.
    kgr()
        .args(["check", "-f", "text", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("no-legacy"));
}

#[test]
fn impact_depth_flag_limits_blast_radius() {
    // db.ts defines `query`; service.ts imports db.ts (depth 1) and app.ts
    // imports service.ts (depth 2). Built in a tempdir with TS relative
    // imports because the python fixtures' edges are distorted by #16/#17.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("db.ts"),
        "export function query(): string {\n    return \"rows\";\n}\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("service.ts"),
        "import { query } from './db';\nexport const s = query();\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("app.ts"),
        "import { s } from './service';\nconsole.log(s);\n",
    )
    .unwrap();

    kgr()
        .args(["impact", "query", "-f", "text", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Impact: 2 files affected"))
        .stdout(predicate::str::contains("service.ts"))
        .stdout(predicate::str::contains("app.ts"));

    kgr()
        .args([
            "impact",
            "query",
            "-f",
            "text",
            "--no-progress",
            "--depth",
            "1",
        ])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Impact: 1 files affected"))
        .stdout(predicate::str::contains("service.ts"))
        .stdout(predicate::str::contains("app.ts").not());
}

#[test]
fn dead_unknown_symbol_json_distinguishes_not_found_from_dead() {
    // A symbol that was never found must not yield a machine-readable
    // "removable" verdict: found=false, dead=null — never dead=true.
    let fixture = fixtures_dir().join("python/simple");
    let output = kgr()
        .args(["dead", "nosuchsymbol", "-f", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["symbol"], "nosuchsymbol");
    assert_eq!(
        json["found"], false,
        "unknown symbol must report found=false"
    );
    assert!(
        json["dead"].is_null(),
        "dead must be null (not true) for a symbol that was never found, got {}",
        json["dead"]
    );
    assert!(json["definitions"].as_array().unwrap().is_empty());
    assert!(json["references"].as_array().unwrap().is_empty());

    // Text mode agrees with JSON: not found, no dead verdict.
    kgr()
        .args(["dead", "nosuchsymbol", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("not found in project"))
        .stdout(predicate::str::contains("Dead").not());
}

#[test]
fn dead_reports_every_definition_of_a_multiply_defined_symbol() {
    // `helper` is defined in two files and never called: both definitions
    // must appear, instead of silently reporting only the first one.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("alpha.py"), "def helper():\n    return 1\n").unwrap();
    std::fs::write(tmp.path().join("beta.py"), "def helper():\n    return 2\n").unwrap();

    let output = kgr()
        .args(["dead", "helper", "-f", "json", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["found"], true);
    assert_eq!(json["dead"], true);
    let definitions = json["definitions"].as_array().unwrap();
    assert_eq!(definitions.len(), 2, "both definitions must be reported");
    let files: Vec<&str> = definitions
        .iter()
        .map(|d| d["file"].as_str().unwrap())
        .collect();
    assert!(files.iter().any(|f| f.contains("alpha.py")));
    assert!(files.iter().any(|f| f.contains("beta.py")));

    // Text mode lists both definition sites too.
    kgr()
        .args(["dead", "helper", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Dead"))
        .stdout(predicate::str::contains("alpha.py"))
        .stdout(predicate::str::contains("beta.py"));
}

#[test]
fn impact_multiply_defined_symbol_unions_dependents_of_all_definitions() {
    // `query` is defined in db.ts AND api.ts; service.ts depends on db.ts,
    // client.ts depends on api.ts. The blast radius must cover dependents of
    // BOTH definitions, not just the first file found.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("db.ts"),
        "export function query(): string {\n    return \"rows\";\n}\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("api.ts"),
        "export function query(): string {\n    return \"remote\";\n}\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("service.ts"),
        "import { query } from './db';\nexport const s = query();\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("client.ts"),
        "import { query } from './api';\nexport const c = query();\n",
    )
    .unwrap();

    let output = kgr()
        .args(["impact", "query", "-f", "json", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["found"], true);

    let definitions = json["definitions"].as_array().unwrap();
    assert_eq!(definitions.len(), 2, "both definitions must be listed");
    let def_files: Vec<&str> = definitions
        .iter()
        .map(|d| d["file"].as_str().unwrap())
        .collect();
    assert!(def_files.iter().any(|f| f.contains("db.ts")));
    assert!(def_files.iter().any(|f| f.contains("api.ts")));

    let impact = json["impact"].as_array().unwrap();
    let impact_files: Vec<&str> = impact.iter().map(|e| e["file"].as_str().unwrap()).collect();
    assert!(
        impact_files.iter().any(|f| f.contains("service.ts")),
        "impact must include the dependent of db.ts, got {impact_files:?}"
    );
    assert!(
        impact_files.iter().any(|f| f.contains("client.ts")),
        "impact must include the dependent of api.ts, got {impact_files:?}"
    );

    // Text mode surfaces all definition sites and the union of dependents.
    kgr()
        .args(["impact", "query", "-f", "text", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Defined in 2 locations"))
        .stdout(predicate::str::contains("db.ts"))
        .stdout(predicate::str::contains("api.ts"))
        .stdout(predicate::str::contains("Impact: 2 files affected"))
        .stdout(predicate::str::contains("service.ts"))
        .stdout(predicate::str::contains("client.ts"));
}

#[test]
fn hotspots_top_flag_limits_entries() {
    let fixture = fixtures_dir().join("python/calls");

    let entries = |top: &str| -> Vec<serde_json::Value> {
        let out = kgr()
            .args(["hotspots", "-f", "json", "--no-progress", "--top", top])
            .arg(&fixture)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        serde_json::from_slice::<serde_json::Value>(&out)
            .unwrap()
            .as_array()
            .unwrap()
            .clone()
    };

    let all = entries("10");
    assert!(
        all.len() > 2,
        "python/calls should rank more than two files with functions"
    );

    let limited = entries("2");
    assert_eq!(
        limited.len(),
        2,
        "--top 2 must truncate the ranking to two entries"
    );
    assert!(limited
        .iter()
        .all(|e| e["file"].is_string() && e["score"].is_number()));
}

#[test]
fn bare_kgr_renders_tree_of_current_directory() {
    // The default arm (no subcommand) runs graph with tree format on the
    // cwd. The tempdir has no .kgr.toml, so defaults stay stable.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("main.py"), "import helper\n").unwrap();
    std::fs::write(tmp.path().join("helper.py"), "x = 1\n").unwrap();

    kgr()
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("main.py"))
        .stdout(predicate::str::contains("[entry]"))
        .stdout(predicate::str::contains("helper.py"));
}

// ── Unknown --format rejection ────────────────────────────────────────────────
//
// Every subcommand must reject an unrecognized --format with exit 2 and an
// error naming its valid formats, instead of silently falling through to the
// default text/table branch (a typo like `-f josn` used to feed text to JSON
// parsers everywhere except `graph`).

#[test]
fn all_subcommands_reject_unknown_format() {
    let fixture = fixtures_dir().join("typescript/simple");
    let cases: &[(&[&str], &str)] = &[
        (
            &["graph", "--no-progress"],
            "expected: tree, json, table, dot, mermaid",
        ),
        (&["check", "--no-progress"], "expected: text, json"),
        (
            &["query", "--cycles", "--no-progress"],
            "expected: table, json",
        ),
        (&["symbols", "--no-progress"], "expected: table, json"),
        (&["refs", "greet", "--no-progress"], "expected: table, json"),
        (&["dead", "greet", "--no-progress"], "expected: table, json"),
        (
            &["skeleton", "--no-progress"],
            "expected: text, json, table",
        ),
        (&["orient", "--no-progress"], "expected: text, json"),
        (
            &["impact", "greet", "--no-progress"],
            "expected: text, json",
        ),
        (
            &["hotspots", "--no-progress"],
            "expected: table, json, text",
        ),
    ];

    for (args, expected) in cases {
        kgr()
            .args(*args)
            .args(["--format", "josn"])
            .arg(&fixture)
            .assert()
            .code(2)
            .stdout(predicate::str::is_empty())
            .stderr(predicate::str::contains("Unknown format: josn"))
            .stderr(predicate::str::contains(*expected));
    }
}

#[test]
fn agent_info_rejects_unknown_format() {
    kgr()
        .args(["agent-info", "--format", "josn"])
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "Unknown format: josn (expected: text, json)",
        ));
}

/// A bad format coming from the config `format` field (no CLI flag at all)
/// must fail identically to a bad CLI flag.
#[test]
fn config_sourced_bad_format_rejected_by_check() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("app.py"), "import helper\n").unwrap();
    std::fs::write(tmp.path().join("helper.py"), "x = 1\n").unwrap();
    std::fs::write(tmp.path().join(".kgr.toml"), "format = \"yaml\"\n").unwrap();

    kgr()
        .args(["check", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "Unknown format: yaml (expected: text, json)",
        ));
}
