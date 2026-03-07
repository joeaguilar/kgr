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

#[test]
fn version_flag() {
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("kgr"));
}

#[test]
fn python_simple_json() {
    let fixture = fixtures_dir().join("python/simple");
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
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
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
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
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["check", "--no-progress"])
        .arg(&fixture)
        .assert()
        .failure()
        .stderr(predicate::str::contains("cycle"));
}

#[test]
fn javascript_simple_json() {
    let fixture = fixtures_dir().join("javascript/simple");
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
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
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["graph", "--format", "tree", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("main.ts"));
}

#[test]
fn dot_output_format() {
    let fixture = fixtures_dir().join("typescript/simple");
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["graph", "--format", "dot", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("digraph kgr"));
}

#[test]
fn init_creates_config() {
    let tmp = tempfile::tempdir().unwrap();
    // Create a dummy .py file so init detects python
    std::fs::write(tmp.path().join("test.py"), "import os\n").unwrap();

    assert_cmd::cargo::cargo_bin_cmd!("kgr")
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

    assert_cmd::cargo::cargo_bin_cmd!("kgr")
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

    assert_cmd::cargo::cargo_bin_cmd!("kgr")
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

    assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["check", "--no-progress"])
        .arg(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("All checks passed."));
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
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
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
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["check", "--no-progress", "--update-baseline"])
        .arg(tmp.path())
        .assert()
        .success();

    // Now check — should pass because all violations are in baseline
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
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
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
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
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
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

    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
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

    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
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

    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
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

// ── agent-info subcommand ─────────────────────────────────────────────────────

#[test]
fn agent_info_text() {
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .arg("agent-info")
        .assert()
        .success()
        .stdout(predicate::str::contains("SUBCOMMANDS"))
        .stdout(predicate::str::contains("kgr check"))
        .stdout(predicate::str::contains("RECOMMENDED AGENT WORKFLOW"));
}

#[test]
fn agent_info_json() {
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["agent-info", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert!(json["guide"].as_str().unwrap().contains("SUBCOMMANDS"));
}
