use assert_cmd::Command;
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
    Command::cargo_bin("kgr")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("kgr"));
}

#[test]
fn python_simple_json() {
    let fixture = fixtures_dir().join("python/simple");
    let output = Command::cargo_bin("kgr")
        .unwrap()
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
    let output = Command::cargo_bin("kgr")
        .unwrap()
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
    Command::cargo_bin("kgr")
        .unwrap()
        .args(["check", "--no-progress"])
        .arg(&fixture)
        .assert()
        .failure()
        .stderr(predicate::str::contains("cycle"));
}

#[test]
fn javascript_simple_json() {
    let fixture = fixtures_dir().join("javascript/simple");
    let output = Command::cargo_bin("kgr")
        .unwrap()
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
    Command::cargo_bin("kgr")
        .unwrap()
        .args(["graph", "--format", "tree", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("main.ts"));
}

#[test]
fn dot_output_format() {
    let fixture = fixtures_dir().join("typescript/simple");
    Command::cargo_bin("kgr")
        .unwrap()
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

    Command::cargo_bin("kgr")
        .unwrap()
        .args(["init"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(".kgr.toml"));

    assert!(tmp.path().join(".kgr.toml").exists());
    let content = std::fs::read_to_string(tmp.path().join(".kgr.toml")).unwrap();
    assert!(content.contains("py"));
}
