use predicates::prelude::*;
use std::path::{Path, PathBuf};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures")
}

fn graph_json(path: &Path) -> serde_json::Value {
    let output = kgr()
        .args(["graph", "--format", "json", "--no-progress"])
        .arg(path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    serde_json::from_slice(&output).unwrap()
}

fn query_orphans_json(path: &Path) -> Vec<String> {
    let output = kgr()
        .args(["query", "--orphans", "-f", "json", "--no-progress"])
        .arg(path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    serde_json::from_slice::<Vec<String>>(&output).unwrap()
}

/// Build a `kgr` command with the parse cache disabled (`KGR_NO_CACHE=1`), so
/// every invocation re-parses sources and never reads or writes
/// `.kgr-cache.json`. This keeps fixture-driven tests hermetic: a stale warm
/// cache under tests/fixtures/ can never mask a parser regression, and test
/// runs leave no cache files behind. Tests that exercise the cache itself
/// build from `kgr_without_host_kgr_env()` and opt in to cache behavior.
fn kgr() -> assert_cmd::Command {
    let mut cmd = kgr_without_host_kgr_env();
    cmd.env("KGR_NO_CACHE", "1");
    cmd
}

/// Build a `kgr` command that is insulated from host-level `KGR_*` config.
///
/// Tests may set the `KGR_*` variables they are explicitly exercising after
/// calling this helper. `KGR_NO_CACHE` is handled by `kgr()` above or by
/// cache-specific tests below.
fn kgr_without_host_kgr_env() -> assert_cmd::Command {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("kgr");
    strip_host_kgr_env(&mut cmd);
    cmd
}

fn strip_host_kgr_env(cmd: &mut assert_cmd::Command) {
    for key in std::env::vars_os()
        .map(|(key, _)| key)
        .filter(|key| key.to_string_lossy().starts_with("KGR_"))
    {
        cmd.env_remove(key);
    }
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

#[test]
fn explicit_kgr_env_layer_still_applies() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("main.py"), "import helper\n").unwrap();
    std::fs::write(tmp.path().join("helper.py"), "VALUE = 1\n").unwrap();

    let output = kgr()
        .env("KGR_FORMAT", "json")
        .args(["graph", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert!(json["files"].is_array());
    assert!(json["edges"].is_array());
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

#[test]
fn malformed_baseline_exits_2_and_reports_path() {
    let tmp = tempfile::tempdir().unwrap();
    make_ts_fixture(&tmp);
    let baseline_path = tmp.path().join(".kgr-baseline.json");
    std::fs::write(&baseline_path, "not json\n").unwrap();

    kgr()
        .args(["check", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("failed to load baseline"))
        .stderr(predicate::str::contains(".kgr-baseline.json"))
        .stderr(predicate::str::contains("malformed baseline JSON"));
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

#[test]
fn check_zero_files_json_exits_2_with_parseable_error() {
    let tmp = tempfile::tempdir().unwrap();

    let output = kgr()
        .args(["check", "--format", "json", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("No supported source files found"))
        .stderr(predicate::str::contains("--lang filter"))
        .stderr(predicate::str::contains("exclude settings"))
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"], "no supported source files found");
}

#[test]
fn check_update_baseline_zero_files_errors_without_writing_baseline() {
    let tmp = tempfile::tempdir().unwrap();

    kgr()
        .args(["check", "--no-progress", "--update-baseline"])
        .arg(tmp.path())
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("No supported source files found"))
        .stderr(predicate::str::contains("--lang filter"))
        .stderr(predicate::str::contains("exclude settings"));

    assert!(
        !tmp.path().join(".kgr-baseline.json").exists(),
        "zero-file --update-baseline must not create a stale baseline"
    );
}

// ── check --exit-zero (report-only mode) ──────────────────────────────────────

/// `check --exit-zero` is report-only for findings: on a cycle, the JSON is
/// byte-identical to the default run (including "ok": false and the cycle
/// itself) — only the exit code changes from 1 to 0.
#[test]
fn check_exit_zero_reports_cycle_json_and_exits_0() {
    let fixture = fixtures_dir().join("typescript/cycle");

    // Default behavior unchanged: findings exit 1.
    let default_stdout = kgr()
        .args(["check", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();

    let exit_zero_stdout = kgr()
        .args(["check", "--format", "json", "--exit-zero", "--no-progress"])
        .arg(&fixture)
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();

    assert_eq!(
        default_stdout, exit_zero_stdout,
        "--exit-zero must not change the JSON diagnostics, only the exit code"
    );

    let json: serde_json::Value = serde_json::from_slice(&exit_zero_stdout).unwrap();
    assert_eq!(json["ok"], false, "findings still reported as not ok");
    assert!(
        !json["cycles"].as_array().unwrap().is_empty(),
        "the cycle must still appear in JSON output"
    );
}

/// Text mode under --exit-zero keeps the error-severity diagnostics on
/// stderr (no fake "All checks passed.") while exiting 0.
#[test]
fn check_exit_zero_text_mode_keeps_error_diagnostics() {
    let fixture = fixtures_dir().join("typescript/cycle");

    kgr()
        .args(["check", "--exit-zero", "--no-progress"])
        .arg(&fixture)
        .assert()
        .code(0)
        .stderr(predicate::str::contains("error[kgr::cycle]"))
        .stderr(predicate::str::contains("All checks passed.").not());
}

/// --exit-zero composes with rule enforcement: an error-severity rule
/// violation still prints and appears in JSON, but the exit code is 0.
#[test]
fn check_exit_zero_rule_violation_exits_0() {
    let tmp = tempfile::tempdir().unwrap();
    make_ts_fixture(&tmp);
    std::fs::write(
        tmp.path().join(".kgr.toml"),
        "[[rules]]\nname=\"no-legacy-to-core\"\nfrom=\"legacy/**\"\nto=\"core/**\"\nseverity=\"error\"\n",
    )
    .unwrap();

    let output = kgr()
        .args(["check", "--format", "json", "--exit-zero", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["ok"], false);
    let violations = json["rule_violations"].as_array().unwrap();
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0]["severity"], "error");
}

/// --exit-zero applies to findings only. Operational failures — unreadable
/// or empty scan target, invalid --format, broken rule config — must still
/// exit nonzero so harnesses can distinguish "ran and found issues" from
/// "did not run".
#[test]
fn check_exit_zero_does_not_mask_operational_errors() {
    // Zero supported files: still exit 2.
    let empty = tempfile::tempdir().unwrap();
    kgr()
        .args(["check", "--format", "json", "--exit-zero", "--no-progress"])
        .arg(empty.path())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("No supported source files found"));

    // Invalid format: still exit 2.
    let tmp = tempfile::tempdir().unwrap();
    make_ts_fixture(&tmp);
    kgr()
        .args(["check", "--format", "josn", "--exit-zero", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Unknown format"));

    // Broken rule config (invalid glob): still exits nonzero.
    std::fs::write(
        tmp.path().join(".kgr.toml"),
        "[[rules]]\nname=\"bad-glob\"\nfrom=\"legacy/[\"\nto=\"core/**\"\nseverity=\"error\"\n",
    )
    .unwrap();
    kgr()
        .args(["check", "--exit-zero", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("warning[kgr::rule-config]"));
}

#[test]
fn zero_file_json_scan_commands_emit_parseable_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let cases: &[&[&str]] = &[
        &["graph", "--format", "json", "--no-progress"],
        &["query", "--cycles", "--format", "json", "--no-progress"],
        &["symbols", "--format", "json", "--no-progress"],
    ];

    for args in cases {
        let output = kgr()
            .args(args.iter().copied())
            .arg(tmp.path())
            .assert()
            .code(2)
            .stderr(predicate::str::contains("No supported source files found"))
            .get_output()
            .stdout
            .clone();

        let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["error"], "no supported source files found");
    }
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
        let mut cmd = kgr_without_host_kgr_env();
        if !cache_enabled {
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
        let out = kgr_without_host_kgr_env()
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
        .stdout(predicate::str::contains("--exit-zero"))
        .stdout(predicate::str::contains("--first-party"))
        .stdout(predicate::str::contains("FIRST-PARTY FILTERING"))
        .stdout(predicate::str::contains("first_party_filter"))
        .stdout(predicate::str::contains("vendor_globs"))
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
fn js_ts_orphans_exclude_vite_html_entry_and_package_scripts() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::create_dir_all(tmp.path().join("scripts")).unwrap();

    std::fs::write(
        tmp.path().join("index.html"),
        r#"<script type="module" src="/src/boot.ts"></script>"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("package.json"),
        r#"{"scripts":{"seed":"tsx scripts/seed.ts"}}"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("src/boot.ts"),
        "import { app } from './app';\nconsole.log(app);\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("src/app.ts"), "export const app = 1;\n").unwrap();
    std::fs::write(
        tmp.path().join("scripts/seed.ts"),
        "export const seed = 1;\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("src/lonely.ts"),
        "export const lonely = 1;\n",
    )
    .unwrap();

    let orphans = query_orphans_json(tmp.path());

    assert!(orphans.contains(&"src/lonely.ts".to_string()));
    assert!(
        !orphans.contains(&"src/boot.ts".to_string()),
        "Vite HTML module entry should not be a real orphan: {orphans:?}"
    );
    assert!(
        !orphans.contains(&"scripts/seed.ts".to_string()),
        "package.json script target should not be a real orphan: {orphans:?}"
    );
}

#[test]
fn js_ts_orphans_exclude_config_setup_routes_and_stories() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("src/routes")).unwrap();
    std::fs::create_dir_all(tmp.path().join("src/components")).unwrap();

    std::fs::write(tmp.path().join("vite.config.ts"), "export default {};\n").unwrap();
    std::fs::write(
        tmp.path().join("src/setupTests.ts"),
        "export function setup(): void {}\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("src/routes/about.tsx"),
        "export default function About() { return <h1>About</h1>; }\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("src/components/Button.stories.tsx"),
        "export const Primary = {};\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("src/lonely.ts"),
        "export const lonely = 1;\n",
    )
    .unwrap();

    let orphans = query_orphans_json(tmp.path());

    assert!(orphans.contains(&"src/lonely.ts".to_string()));
    for entry in [
        "vite.config.ts",
        "src/setupTests.ts",
        "src/routes/about.tsx",
        "src/components/Button.stories.tsx",
    ] {
        assert!(
            !orphans.contains(&entry.to_string()),
            "{entry} should be classified as a structural JS/TS entry: {orphans:?}"
        );
    }
}

#[test]
fn js_ts_worker_url_construction_creates_dependency_edge() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(
        tmp.path().join("src/index.ts"),
        "new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' });\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("src/worker.ts"),
        "self.postMessage('ready');\n",
    )
    .unwrap();

    let json = graph_json(tmp.path());
    let edges = json["edges"].as_array().unwrap();
    assert!(
        edges
            .iter()
            .any(|edge| edge["from"] == "src/index.ts" && edge["to"] == "src/worker.ts"),
        "worker URL should produce an index.ts -> worker.ts edge: {edges:?}"
    );
    let orphans: Vec<String> = serde_json::from_value(json["orphans"].clone()).unwrap();
    assert!(
        !orphans.contains(&"src/worker.ts".to_string()),
        "worker file with a URL edge should not be orphaned: {orphans:?}"
    );
}

#[test]
fn js_ts_ambient_globals_are_classified_but_type_companions_are_linked() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(
        tmp.path().join("src/global.d.ts"),
        "declare global { interface Window { appReady: boolean } }\nexport {};\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("src/widget.ts"),
        "export const widget = 1;\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("src/widget.d.ts"),
        "export declare const widget: number;\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("src/lonely.ts"),
        "export const lonely = 1;\n",
    )
    .unwrap();

    let json = graph_json(tmp.path());
    let edges = json["edges"].as_array().unwrap();
    assert!(
        edges
            .iter()
            .any(|edge| edge["from"] == "src/widget.ts" && edge["to"] == "src/widget.d.ts"),
        "type companion declaration should be tied to its source edge: {edges:?}"
    );

    let orphans: Vec<String> = serde_json::from_value(json["orphans"].clone()).unwrap();
    assert!(orphans.contains(&"src/lonely.ts".to_string()));
    assert!(
        !orphans.contains(&"src/global.d.ts".to_string()),
        "ambient global declaration should not be a real orphan: {orphans:?}"
    );
    assert!(
        !orphans.contains(&"src/widget.d.ts".to_string()),
        "type companion should be linked, not classified as a loose global: {orphans:?}"
    );
}

#[test]
fn tsconfig_paths_load_from_scan_root_not_cwd() {
    // Scan target: a project whose tsconfig maps @app/* -> src/app/*.
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("src/app")).unwrap();
    std::fs::write(
        project.path().join("tsconfig.json"),
        r#"{"compilerOptions": {"paths": {"@app/*": ["src/app/*"]}}}"#,
    )
    .unwrap();
    std::fs::write(
        project.path().join("src/main.ts"),
        "import { util } from '@app/util';\n",
    )
    .unwrap();
    std::fs::write(
        project.path().join("src/app/util.ts"),
        "export const util = 1;\n",
    )
    .unwrap();

    // Process CWD: a different directory whose decoy tsconfig points the same
    // alias at a target that does not exist in the scanned project. Under the
    // old behavior (tsconfig.json loaded relative to the CWD, not the scan
    // root) the decoy would be loaded and the alias import would not resolve.
    let cwd = tempfile::tempdir().unwrap();
    std::fs::write(
        cwd.path().join("tsconfig.json"),
        r#"{"compilerOptions": {"paths": {"@app/*": ["decoy/*"]}}}"#,
    )
    .unwrap();

    let output = kgr()
        .current_dir(cwd.path())
        .args(["graph", "--format", "json", "--no-progress"])
        .arg(project.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

    let edges = json["edges"].as_array().unwrap();
    assert!(
        edges
            .iter()
            .any(|edge| edge["from"] == "src/main.ts" && edge["to"] == "src/app/util.ts"),
        "tsconfig alias from the scanned root should resolve @app/util: {edges:?}"
    );

    let external_deps = json["external_deps"].as_object().unwrap();
    assert!(
        !external_deps.values().any(|deps| {
            deps.as_array()
                .map(|d| d.iter().any(|name| name == "@app/util"))
                .unwrap_or(false)
        }),
        "resolved alias import must not surface as an external dep: {external_deps:?}"
    );
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

// ── Rust module & re-export edges ─────────────────────────────────────────────

/// Edge list from graph JSON as (from, to) string pairs.
fn graph_edges(path: &Path) -> Vec<(String, String)> {
    graph_json(path)["edges"]
        .as_array()
        .expect("graph JSON edges array")
        .iter()
        .map(|edge| {
            (
                edge["from"].as_str().unwrap().to_string(),
                edge["to"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn query_who_imports_json(root: &Path, target: &str) -> Vec<String> {
    let output = kgr()
        .args([
            "query",
            "--who-imports",
            target,
            "-f",
            "json",
            "--no-progress",
        ])
        .arg(root)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice::<Vec<String>>(&output).unwrap()
}

/// Nested modules + a re-export barrel: `pub use crate::models::*;` gives the
/// barrel out-edges, `use crate::prelude::*;` gives it in-edges, and a
/// consumer of a crate-root re-export (`use crate::User;`) links to lib.rs.
#[test]
fn rust_reexport_barrel_and_nested_modules_create_edges() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(src.join("models")).unwrap();
    std::fs::write(
        src.join("lib.rs"),
        "pub mod api;\npub mod models;\npub mod prelude;\npub use models::user::User;\n",
    )
    .unwrap();
    std::fs::write(src.join("models/mod.rs"), "pub mod user;\n").unwrap();
    std::fs::write(src.join("models/user.rs"), "pub struct User;\n").unwrap();
    std::fs::write(
        src.join("prelude.rs"),
        "pub use crate::models::*;\npub use crate::models::user::User;\n",
    )
    .unwrap();
    std::fs::write(
        src.join("api.rs"),
        "use crate::prelude::*;\nuse crate::User;\npub fn handler() -> User { User }\n",
    )
    .unwrap();

    let edges = graph_edges(tmp.path());
    let expect = [
        // mod declarations
        ("src/lib.rs", "src/api.rs"),
        ("src/lib.rs", "src/models/mod.rs"),
        ("src/lib.rs", "src/prelude.rs"),
        ("src/models/mod.rs", "src/models/user.rs"),
        // crate-root re-export of a nested module item (bare 2018 path)
        ("src/lib.rs", "src/models/user.rs"),
        // re-export barrel: glob and item re-exports both give out-edges
        ("src/prelude.rs", "src/models/mod.rs"),
        ("src/prelude.rs", "src/models/user.rs"),
        // consumers: glob import of the barrel, item via crate-root re-export
        ("src/api.rs", "src/prelude.rs"),
        ("src/api.rs", "src/lib.rs"),
    ];
    for (from, to) in expect {
        assert!(
            edges.contains(&(from.to_string(), to.to_string())),
            "missing edge {from} -> {to} in {edges:?}"
        );
    }

    // The barrel and the re-export consumer edges feed who-imports…
    assert_eq!(
        query_who_imports_json(tmp.path(), "src/prelude.rs"),
        vec!["src/api.rs".to_string(), "src/lib.rs".to_string()]
    );
    assert_eq!(
        query_who_imports_json(tmp.path(), "src/lib.rs"),
        vec!["src/api.rs".to_string()]
    );

    // …and heaviest rankings: lib.rs and the barrel have real dependents.
    let output = kgr()
        .args(["query", "--heaviest", "-f", "json", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let ranked: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let dependents_of = |path: &str| {
        ranked
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["path"] == path)
            .map(|e| e["dependents"].as_u64().unwrap())
            .unwrap_or(0)
    };
    assert_eq!(dependents_of("src/prelude.rs"), 2);
    assert_eq!(dependents_of("src/lib.rs"), 1);
    // user.rs: declared by models/mod.rs, re-exported by lib.rs and prelude.rs.
    assert_eq!(dependents_of("src/models/user.rs"), 3);
}

/// `use super::item;` from a nested module links the child to the file that
/// DEFINES the parent module — mod.rs and modern sibling layouts both.
#[test]
fn rust_super_item_import_links_child_to_parent_module_file() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(src.join("engine")).unwrap();
    std::fs::create_dir_all(src.join("store")).unwrap();
    std::fs::write(src.join("lib.rs"), "pub mod engine;\npub mod store;\n").unwrap();
    std::fs::write(
        src.join("engine/mod.rs"),
        "pub mod core;\npub fn config() {}\n",
    )
    .unwrap();
    std::fs::write(
        src.join("engine/core.rs"),
        "use super::config;\npub fn run() { config(); }\n",
    )
    .unwrap();
    // Modern layout: parent module file is the sibling store.rs.
    std::fs::write(src.join("store.rs"), "pub mod disk;\npub fn flush() {}\n").unwrap();
    std::fs::write(
        src.join("store/disk.rs"),
        "use super::flush;\npub fn sync() { flush(); }\n",
    )
    .unwrap();

    let edges = graph_edges(tmp.path());
    assert!(
        edges.contains(&("src/engine/core.rs".into(), "src/engine/mod.rs".into())),
        "super item import should link core.rs to engine/mod.rs: {edges:?}"
    );
    assert!(
        edges.contains(&("src/store/disk.rs".into(), "src/store.rs".into())),
        "super item import should link disk.rs to sibling store.rs: {edges:?}"
    );
}

/// The ubiquitous `#[cfg(test)] mod tests { use super::*; }` names the file's
/// OWN module: it must not draw a phantom edge to the parent module file and
/// must not surface as an external package.
#[test]
fn rust_test_module_super_glob_creates_no_phantom_edges() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("lib.rs"), "pub mod util;\n").unwrap();
    std::fs::write(
        src.join("util.rs"),
        "pub fn helper() {}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn works() {\n        helper();\n    }\n}\n",
    )
    .unwrap();

    let json = graph_json(tmp.path());
    let edges = graph_edges(tmp.path());
    assert!(
        !edges.contains(&("src/util.rs".into(), "src/lib.rs".into())),
        "test-module super glob must not point at the crate root: {edges:?}"
    );
    let externals = &json["external_deps"]["src/util.rs"];
    assert!(
        externals.is_null(),
        "test-module super glob must not be an external dep: {externals}"
    );
}

/// A crate root reachable only through its re-exports must not be reported
/// as an orphan: `use crate::Engine;` from a module file is an in-edge to
/// the lib.rs that re-exports Engine, even with no file-backed `mod` decls.
#[test]
fn rust_lib_rs_reachable_via_reexports_is_not_an_orphan() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("lib.rs"),
        "pub mod inner {\n    pub struct Engine;\n}\npub use inner::Engine;\n",
    )
    .unwrap();
    std::fs::write(
        src.join("main.rs"),
        "mod helper;\nfn main() { helper::run(); }\n",
    )
    .unwrap();
    std::fs::write(
        src.join("helper.rs"),
        "use crate::Engine;\npub fn run() { let _ = Engine; }\n",
    )
    .unwrap();

    let orphans = query_orphans_json(tmp.path());
    assert!(
        !orphans.contains(&"src/lib.rs".to_string()),
        "re-export-consumed lib.rs misreported as orphan: {orphans:?}"
    );
    assert_eq!(
        query_who_imports_json(tmp.path(), "src/lib.rs"),
        vec!["src/helper.rs".to_string()]
    );
}

/// Inline-nested `mod` declarations resolve into the inline chain's directory
/// (`mod outer { mod inner; }` in lib.rs -> src/outer/inner.rs).
#[test]
fn rust_nested_inline_mod_declaration_resolves_into_chain_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(src.join("outer")).unwrap();
    std::fs::write(src.join("lib.rs"), "mod outer {\n    pub mod inner;\n}\n").unwrap();
    std::fs::write(src.join("outer/inner.rs"), "pub fn f() {}\n").unwrap();

    let edges = graph_edges(tmp.path());
    assert!(
        edges.contains(&("src/lib.rs".into(), "src/outer/inner.rs".into())),
        "nested inline mod declaration should resolve into outer/: {edges:?}"
    );
}

fn structural_entries_map(json: &serde_json::Value) -> std::collections::BTreeMap<String, String> {
    json["structural_entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["path"].as_str().unwrap().to_string(),
                entry["reason"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// Cargo loads build scripts, binary/library roots, examples, benches, and
/// integration tests by convention, not by imports. They must be classified
/// as structural entry points (with a stable reason) instead of orphans,
/// while a genuinely unreferenced module stays an orphan candidate.
#[test]
fn rust_cargo_targets_classified_as_structural_entries_not_orphans() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("src/bin/multi")).unwrap();
    std::fs::create_dir_all(tmp.path().join("examples")).unwrap();
    std::fs::create_dir_all(tmp.path().join("benches")).unwrap();
    std::fs::create_dir_all(tmp.path().join("tests")).unwrap();

    std::fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("build.rs"), "fn main() {}\n").unwrap();
    std::fs::write(tmp.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(tmp.path().join("src/bin/tool.rs"), "fn main() {}\n").unwrap();
    std::fs::write(tmp.path().join("src/bin/multi/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(tmp.path().join("examples/demo.rs"), "fn main() {}\n").unwrap();
    std::fs::write(tmp.path().join("benches/perf.rs"), "fn main() {}\n").unwrap();
    std::fs::write(tmp.path().join("tests/smoke.rs"), "#[test]\nfn ok() {}\n").unwrap();
    std::fs::write(tmp.path().join("src/dead.rs"), "pub fn unused() {}\n").unwrap();

    let json = graph_json(tmp.path());

    let orphans: Vec<String> = serde_json::from_value(json["orphans"].clone()).unwrap();
    assert_eq!(
        orphans,
        vec!["src/dead.rs".to_string()],
        "only the truly unreferenced module is a real orphan"
    );

    let entries = structural_entries_map(&json);
    for (path, reason) in [
        ("build.rs", "cargo build script"),
        ("src/main.rs", "cargo binary target"),
        ("src/bin/tool.rs", "cargo binary target"),
        ("src/bin/multi/main.rs", "cargo binary target"),
        ("examples/demo.rs", "cargo example target"),
        ("benches/perf.rs", "cargo bench target"),
        ("tests/smoke.rs", "cargo test target"),
    ] {
        assert_eq!(
            entries.get(path).map(String::as_str),
            Some(reason),
            "{path} should classify as '{reason}': {entries:?}"
        );
    }
    assert!(
        !entries.contains_key("src/dead.rs"),
        "a plain unreferenced module must not be classified: {entries:?}"
    );

    // The Cargo test target gets the specific structural classification, not
    // the generic test-entry bucket.
    let test_entries: Vec<String> = serde_json::from_value(json["test_entries"].clone()).unwrap();
    assert!(
        !test_entries.contains(&"tests/smoke.rs".to_string()),
        "cargo test target should not double-report as a test entry: {test_entries:?}"
    );
}

/// Workspace member crate roots (lib.rs / main.rs next to a member
/// Cargo.toml) are loaded by the workspace build, not by imports.
#[test]
fn rust_workspace_member_roots_classified_not_orphans() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("crates/alpha/src")).unwrap();
    std::fs::create_dir_all(tmp.path().join("crates/beta/src")).unwrap();

    std::fs::write(
        tmp.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/alpha\", \"crates/beta\"]\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("crates/alpha/Cargo.toml"),
        "[package]\nname = \"alpha\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("crates/alpha/src/lib.rs"),
        "pub fn alpha() {}\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("crates/beta/Cargo.toml"),
        "[package]\nname = \"beta\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("crates/beta/src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(tmp.path().join("crates/beta/build.rs"), "fn main() {}\n").unwrap();
    std::fs::write(
        tmp.path().join("crates/beta/src/forgotten.rs"),
        "pub fn unused() {}\n",
    )
    .unwrap();

    let json = graph_json(tmp.path());

    let orphans: Vec<String> = serde_json::from_value(json["orphans"].clone()).unwrap();
    assert_eq!(
        orphans,
        vec!["crates/beta/src/forgotten.rs".to_string()],
        "member roots are structural; only the forgotten module is an orphan"
    );

    let entries = structural_entries_map(&json);
    assert_eq!(
        entries.get("crates/alpha/src/lib.rs").map(String::as_str),
        Some("cargo library target"),
        "{entries:?}"
    );
    assert_eq!(
        entries.get("crates/beta/src/main.rs").map(String::as_str),
        Some("cargo binary target"),
        "{entries:?}"
    );
    // Mirrors the kgr dogfood case: a member-crate build script scanned from
    // the workspace's crates/ parent directory.
    assert_eq!(
        entries.get("crates/beta/build.rs").map(String::as_str),
        Some("cargo build script"),
        "{entries:?}"
    );
}

/// Conventional-looking paths with no Cargo.toml anywhere are NOT Cargo
/// targets — they must keep showing up as orphan candidates so the
/// classification never over-suppresses.
#[test]
fn rust_conventional_paths_without_manifest_stay_orphans() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::create_dir_all(tmp.path().join("examples")).unwrap();

    std::fs::write(tmp.path().join("build.rs"), "fn main() {}\n").unwrap();
    std::fs::write(tmp.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(tmp.path().join("examples/demo.rs"), "fn main() {}\n").unwrap();

    let json = graph_json(tmp.path());

    let orphans: Vec<String> = serde_json::from_value(json["orphans"].clone()).unwrap();
    for path in ["build.rs", "src/main.rs", "examples/demo.rs"] {
        assert!(
            orphans.contains(&path.to_string()),
            "{path} without a manifest should stay an orphan: {orphans:?}"
        );
    }
    assert_eq!(
        json["structural_entries"].as_array().unwrap().len(),
        0,
        "no manifest means no Cargo target classification"
    );
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

/// Mixed C/C++ fixture where vendored headers dominate orphan and heaviest
/// analysis, alongside first-party files with the same names:
///
///   a.c, b.c            -> vendor/util.h, util.h
///   vendor/util.h        vendored, 2 dependents (heaviest noise)
///   vendor/foo.h         vendored orphan
///   third_party/bar.hpp  vendored orphan
///   util.h               first-party, 2 dependents, name-twin of vendor/util.h
///   src/foo.h            first-party orphan, name-twin of vendor/foo.h
fn write_vendor_fixture(root: &Path) {
    std::fs::create_dir_all(root.join("vendor")).unwrap();
    std::fs::create_dir_all(root.join("third_party")).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let includes = "#include \"vendor/util.h\"\n#include \"util.h\"\n";
    std::fs::write(root.join("a.c"), includes).unwrap();
    std::fs::write(root.join("b.c"), includes).unwrap();
    std::fs::write(root.join("vendor/util.h"), "#define VENDOR_UTIL 1\n").unwrap();
    std::fs::write(root.join("vendor/foo.h"), "#define VENDOR_FOO 1\n").unwrap();
    std::fs::write(root.join("third_party/bar.hpp"), "#define BAR 1\n").unwrap();
    std::fs::write(root.join("util.h"), "#define UTIL 1\n").unwrap();
    std::fs::write(root.join("src/foo.h"), "#define FOO 1\n").unwrap();
}

/// Default `query --orphans` stays a bare JSON array including vendored
/// paths; `--first-party` switches to an object that filters them out and
/// reports the applied vendor globs, keeping the same-named first-party
/// file.
#[test]
fn query_orphans_first_party_excludes_vendored_paths_only() {
    let tmp = tempfile::tempdir().unwrap();
    write_vendor_fixture(tmp.path());

    // Backwards-compatible default: bare array, vendored orphans included.
    let unfiltered = query_orphans_json(tmp.path());
    for path in ["vendor/foo.h", "third_party/bar.hpp", "src/foo.h"] {
        assert!(
            unfiltered.contains(&path.to_string()),
            "{path} should be an orphan by default: {unfiltered:?}"
        );
    }

    let output = kgr()
        .args([
            "query",
            "--orphans",
            "--first-party",
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

    let orphans: Vec<String> = serde_json::from_value(json["orphans"].clone()).unwrap();
    assert_eq!(
        orphans,
        vec!["src/foo.h".to_string()],
        "vendored orphans filtered; the name-twin first-party header stays"
    );
    assert_eq!(json["first_party_filter"]["excluded_orphans"], 2);
    let globs: Vec<String> =
        serde_json::from_value(json["first_party_filter"]["vendor_globs"].clone()).unwrap();
    assert_eq!(
        globs,
        vec!["**/vendor/**", "**/third_party/**", "**/external/**"],
        "applied filtering must be explicit in JSON output"
    );
}

/// `query --heaviest --first-party` drops vendored headers from the ranking
/// but keeps the first-party header with the same file name and dependent
/// count; the default output shape (bare array with vendored entries) is
/// unchanged.
#[test]
fn query_heaviest_first_party_keeps_first_party_name_twin() {
    let tmp = tempfile::tempdir().unwrap();
    write_vendor_fixture(tmp.path());

    // Default: bare array; the vendored header ranks alongside util.h.
    let output = kgr()
        .args(["query", "--heaviest", "-f", "json", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let unfiltered: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let paths: Vec<&str> = unfiltered
        .as_array()
        .expect("default --heaviest output stays a bare array")
        .iter()
        .map(|item| item["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"vendor/util.h"), "{paths:?}");
    assert!(paths.contains(&"util.h"), "{paths:?}");

    let output = kgr()
        .args([
            "query",
            "--heaviest",
            "--first-party",
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

    let entries: Vec<(String, u64)> = json["heaviest"]
        .as_array()
        .expect("filtered --heaviest output is an object with a heaviest array")
        .iter()
        .map(|item| {
            (
                item["path"].as_str().unwrap().to_string(),
                item["dependents"].as_u64().unwrap(),
            )
        })
        .collect();
    assert!(
        entries.contains(&("util.h".to_string(), 2)),
        "first-party name-twin keeps its rank: {entries:?}"
    );
    assert!(
        entries
            .iter()
            .all(|(path, _)| !path.starts_with("vendor/") && !path.starts_with("third_party/")),
        "vendored paths must be excluded: {entries:?}"
    );
    // vendor/util.h, vendor/foo.h, third_party/bar.hpp
    assert_eq!(json["first_party_filter"]["excluded_files"], 3);
    assert!(json["first_party_filter"]["vendor_globs"].is_array());
}

/// `kgr check --first-party` filters the orphan summary in both text and
/// JSON modes and reports the filtering explicitly; the default JSON shape
/// carries no filter key at all.
#[test]
fn check_first_party_filters_orphan_summary() {
    let tmp = tempfile::tempdir().unwrap();
    write_vendor_fixture(tmp.path());

    // Backwards-compatible default: vendored orphans reported, no filter key.
    let output = kgr()
        .args(["check", "-f", "json", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let orphans: Vec<String> = serde_json::from_value(json["orphans"].clone()).unwrap();
    assert!(orphans.contains(&"vendor/foo.h".to_string()), "{orphans:?}");
    assert!(
        json.get("first_party_filter").is_none(),
        "default JSON output must not grow a filter key"
    );

    let output = kgr()
        .args(["check", "--first-party", "-f", "json", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let orphans: Vec<String> = serde_json::from_value(json["orphans"].clone()).unwrap();
    assert_eq!(orphans, vec!["src/foo.h".to_string()]);
    assert_eq!(json["first_party_filter"]["excluded_orphans"], 2);

    // Text mode: vendored orphans gone from the warning, note names the count.
    kgr()
        .args(["check", "--first-party", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("src/foo.h"))
        .stderr(predicate::str::contains("vendor/foo.h").not())
        .stderr(predicate::str::contains(
            "note: first-party filter excluded 2 vendored file(s) from orphan analysis",
        ));
}

/// `kgr graph --first-party --format json` filters the orphan summary and
/// makes the applied filtering explicit; the default graph JSON is
/// byte-compatible (no filter key, vendored orphans present).
#[test]
fn graph_json_first_party_reports_applied_filter() {
    let tmp = tempfile::tempdir().unwrap();
    write_vendor_fixture(tmp.path());

    let json = graph_json(tmp.path());
    let orphans: Vec<String> = serde_json::from_value(json["orphans"].clone()).unwrap();
    assert!(orphans.contains(&"vendor/foo.h".to_string()), "{orphans:?}");
    assert!(
        json.get("first_party_filter").is_none(),
        "default graph JSON must not grow a filter key"
    );

    let output = kgr()
        .args([
            "graph",
            "--first-party",
            "--format",
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
    let orphans: Vec<String> = serde_json::from_value(json["orphans"].clone()).unwrap();
    assert_eq!(orphans, vec!["src/foo.h".to_string()]);
    assert_eq!(json["first_party_filter"]["excluded_orphans"], 2);
    // The vendored files stay in the graph itself — only the summary filters.
    let files: Vec<&str> = json["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(files.contains(&"vendor/util.h"), "{files:?}");
}

/// Config `first_party = true` enables filtering without the CLI flag, and
/// config `vendor_globs` replaces the default list entirely: only
/// third_party/ is vendored here, so vendor/foo.h stays an orphan.
#[test]
fn config_first_party_and_custom_vendor_globs_drive_query() {
    let tmp = tempfile::tempdir().unwrap();
    write_vendor_fixture(tmp.path());
    std::fs::write(
        tmp.path().join(".kgr.toml"),
        "first_party = true\nvendor_globs = [\"third_party/**\"]\n",
    )
    .unwrap();

    let output = kgr()
        .args(["query", "--orphans", "-f", "json", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

    let orphans: Vec<String> = serde_json::from_value(json["orphans"].clone()).unwrap();
    assert_eq!(
        orphans,
        vec!["src/foo.h".to_string(), "vendor/foo.h".to_string()],
        "custom vendor_globs replace the defaults: only third_party/ filters"
    );
    assert_eq!(json["first_party_filter"]["excluded_orphans"], 1);
    let globs: Vec<String> =
        serde_json::from_value(json["first_party_filter"]["vendor_globs"].clone()).unwrap();
    assert_eq!(globs, vec!["third_party/**"]);
}

// ── Skipped-unsupported reporting ─────────────────────────────────────────────

/// Mixed repo: supported sources (.py, .ts), unsupported source-looking
/// files (.zigzag x4 so the sample bound shows, .xyz, an extensionless text
/// file), non-source assets that must never be reported (json/png/binary
/// blob), and an excluded vendor/ dir whose contents must be invisible.
fn write_mixed_unsupported_fixture(root: &Path) {
    std::fs::write(root.join("main.py"), "import helper\n").unwrap();
    std::fs::write(root.join("helper.py"), "x = 1\n").unwrap();
    std::fs::write(root.join("app.ts"), "import './lib';\n").unwrap();
    std::fs::write(root.join("lib.ts"), "export const x = 1;\n").unwrap();

    for name in ["alpha", "beta", "delta", "gamma"] {
        std::fs::write(root.join(format!("{name}.zigzag")), "unsupported source\n").unwrap();
    }
    std::fs::write(root.join("data.xyz"), "records\n").unwrap();
    std::fs::write(root.join("notes"), "plain text without a shebang\n").unwrap();

    std::fs::write(root.join("config.json"), "{}\n").unwrap();
    std::fs::write(root.join("logo.png"), [0x89u8, b'P', 0x00]).unwrap();
    std::fs::write(root.join("blob"), [0u8, 159, 146, 150]).unwrap();

    std::fs::create_dir(root.join("vendor")).unwrap();
    std::fs::write(root.join("vendor/skip.zigzag"), "excluded\n").unwrap();
    std::fs::write(root.join(".kgr.toml"), "exclude = [\"vendor/**\"]\n").unwrap();
}

#[test]
fn graph_json_reports_skipped_unsupported_groups_with_bounded_sample() {
    let tmp = tempfile::tempdir().unwrap();
    write_mixed_unsupported_fixture(tmp.path());

    let json = graph_json(tmp.path());

    let groups = json["skipped_unsupported"].as_array().unwrap();
    assert_eq!(groups.len(), 3, "{groups:?}");

    // Largest group first; sample sorted and capped at 3 of the 4 files.
    assert_eq!(groups[0]["group"], "zigzag");
    assert_eq!(groups[0]["count"], 4);
    let sample: Vec<&str> = groups[0]["sample"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(sample, ["alpha.zigzag", "beta.zigzag", "delta.zigzag"]);

    // Ties order alphabetically: "(no extension)" before "xyz".
    assert_eq!(groups[1]["group"], "(no extension)");
    assert_eq!(groups[1]["count"], 1);
    assert_eq!(groups[1]["sample"][0], "notes");
    assert_eq!(groups[2]["group"], "xyz");
    assert_eq!(groups[2]["count"], 1);

    // Non-source assets, binary blobs, and excluded paths never appear.
    let raw = serde_json::to_string(&json["skipped_unsupported"]).unwrap();
    assert!(!raw.contains("config.json"), "{raw}");
    assert!(!raw.contains("logo.png"), "{raw}");
    assert!(!raw.contains("blob"), "{raw}");
    assert!(!raw.contains("vendor"), "{raw}");
}

#[test]
fn check_json_includes_skipped_unsupported_summary() {
    let tmp = tempfile::tempdir().unwrap();
    write_mixed_unsupported_fixture(tmp.path());

    let output = kgr()
        .args(["check", "--format", "json", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(json["skipped_unsupported"][0]["group"], "zigzag");
    assert_eq!(json["skipped_unsupported"][0]["count"], 4);
    let sample = json["skipped_unsupported"][0]["sample"].as_array().unwrap();
    assert_eq!(sample.len(), 3);
}

#[test]
fn orient_json_includes_skipped_unsupported_summary() {
    let tmp = tempfile::tempdir().unwrap();
    write_mixed_unsupported_fixture(tmp.path());

    let output = kgr()
        .args(["orient", "--format", "json", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

    let groups = json["skipped_unsupported"].as_array().unwrap();
    assert_eq!(groups[0]["group"], "zigzag");
    assert_eq!(groups[0]["count"], 4);

    // Orient text mode surfaces the same summary as a single bounded line.
    kgr()
        .args(["orient", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Skipped unsupported: 6 file(s) (zigzag: 4, (no extension): 1, xyz: 1)",
        ));
}

/// Supported languages excluded by --lang are filtered, not "unsupported":
/// the skipped summary must not mention them, and genuinely unsupported
/// files are still reported under an active filter.
#[test]
fn lang_filter_does_not_misreport_supported_languages_as_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    write_mixed_unsupported_fixture(tmp.path());

    let output = kgr()
        .args(["graph", "--format", "json", "--no-progress", "--lang", "py"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

    let files: Vec<&str> = json["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(files.contains(&"main.py"), "{files:?}");
    assert!(!files.contains(&"app.ts"), "{files:?}");

    let groups = json["skipped_unsupported"].as_array().unwrap();
    assert!(groups.iter().any(|g| g["group"] == "zigzag"), "{groups:?}");
    assert!(
        !groups.iter().any(|g| g["group"] == "ts"),
        "filtered TypeScript files must not be reported as skipped: {groups:?}"
    );
    let raw = serde_json::to_string(&json["skipped_unsupported"]).unwrap();
    assert!(!raw.contains("app.ts"), "{raw}");
    assert!(!raw.contains("lib.ts"), "{raw}");
}

/// Fully-supported repos keep their JSON shape: the key appears ONLY when
/// at least one unsupported file was skipped.
#[test]
fn graph_json_omits_skipped_unsupported_for_fully_supported_repo() {
    let json = graph_json(&fixtures_dir().join("python/simple"));
    assert!(json.get("skipped_unsupported").is_none());
}

/// Human-format runs get a bounded stderr note (mirroring the parse-failure
/// summary): total count, per-extension samples, omitted-count marker — and
/// nothing from excluded directories.
#[test]
fn skipped_unsupported_stderr_note_is_bounded_and_respects_excludes() {
    let tmp = tempfile::tempdir().unwrap();
    write_mixed_unsupported_fixture(tmp.path());

    kgr()
        .args(["graph", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("skipped 6 unsupported file(s)"))
        .stderr(predicate::str::contains("alpha.zigzag"))
        .stderr(predicate::str::contains("1 more"))
        .stderr(predicate::str::contains("gamma.zigzag").not())
        .stderr(predicate::str::contains("vendor").not());
}
