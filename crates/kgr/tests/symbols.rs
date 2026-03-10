//! Integration tests for `kgr symbols`, `kgr refs`, and `kgr dead`.
//!
//! These tests define the contract for symbol-level analysis.
//! All are #[ignore] until the underlying implementation exists.
//!
//! Fixture: tests/fixtures/python/calls/
//!   app.py      — main(), cli(); calls service.fetch_users, utils.normalize, utils.log
//!   service.py  — class UserService (get_user, list_users), fetch_users(); calls db.query
//!   db.py       — query(), connect(), _internal_reset() (never called)
//!   utils.py    — normalize(), log(), deprecated_helper() (never called)
//!
//! Fixture: tests/fixtures/typescript/calls/
//!   app.ts      — main() [exported], cli(); calls fetchUsers, normalize, log
//!   service.ts  — class UserService (getUser, listUsers), fetchUsers() [exported]; calls query
//!   db.ts       — query() [exported], connect() [exported], internalReset() (not exported, never called)
//!   utils.ts    — normalize() [exported], log() [exported], deprecatedHelper() [exported, never called]

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

// ═══════════════════════════════════════════════════════════════════════════════
// kgr symbols — "What's defined here?"
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn symbols_python_json_returns_all_definitions() {
    let fixture = fixtures_dir().join("python/calls");
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["symbols", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let entries = json
        .as_array()
        .expect("top-level should be an array of file entries");

    // Collect all symbol names across all files
    let all_symbols: Vec<&str> = entries
        .iter()
        .flat_map(|entry| {
            entry["symbols"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["name"].as_str().unwrap())
        })
        .collect();

    // Every defined function should appear
    assert!(all_symbols.contains(&"main"), "missing main");
    assert!(all_symbols.contains(&"cli"), "missing cli");
    assert!(all_symbols.contains(&"fetch_users"), "missing fetch_users");
    assert!(all_symbols.contains(&"query"), "missing query");
    assert!(all_symbols.contains(&"connect"), "missing connect");
    assert!(all_symbols.contains(&"normalize"), "missing normalize");
    assert!(all_symbols.contains(&"log"), "missing log");
    assert!(
        all_symbols.contains(&"deprecated_helper"),
        "missing deprecated_helper"
    );
    assert!(
        all_symbols.contains(&"_internal_reset"),
        "missing _internal_reset"
    );
}

#[test]
fn symbols_python_json_includes_classes() {
    let fixture = fixtures_dir().join("python/calls");
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["symbols", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let entries = json.as_array().unwrap();

    // Find UserService class
    let has_user_service = entries.iter().any(|entry| {
        entry["symbols"].as_array().unwrap().iter().any(|s| {
            s["name"].as_str() == Some("UserService") && s["kind"].as_str() == Some("class")
        })
    });

    assert!(has_user_service, "should find UserService class");
}

#[test]
fn symbols_python_json_has_correct_shape() {
    let fixture = fixtures_dir().join("python/calls");
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["symbols", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let entries = json.as_array().unwrap();

    // Each entry should have file + symbols
    for entry in entries {
        assert!(entry["file"].is_string(), "entry missing 'file' field");
        let symbols = entry["symbols"]
            .as_array()
            .expect("entry missing 'symbols' array");
        for sym in symbols {
            assert!(sym["name"].is_string(), "symbol missing 'name'");
            assert!(sym["kind"].is_string(), "symbol missing 'kind'");
            assert!(sym["line"].is_number(), "symbol missing 'line'");
            // kind must be one of the known values
            let kind = sym["kind"].as_str().unwrap();
            assert!(
                ["function", "method", "class"].contains(&kind),
                "unexpected symbol kind: {kind}"
            );
        }
    }
}

#[test]
fn symbols_typescript_json_marks_exported() {
    let fixture = fixtures_dir().join("typescript/calls");
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["symbols", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let entries = json.as_array().unwrap();

    // Find the utils.ts entry
    let utils_entry = entries
        .iter()
        .find(|e| e["file"].as_str().unwrap().contains("utils.ts"))
        .expect("should find utils.ts");

    let symbols = utils_entry["symbols"].as_array().unwrap();

    // normalize should be exported
    let normalize = symbols
        .iter()
        .find(|s| s["name"].as_str() == Some("normalize"))
        .expect("should find normalize");
    assert_eq!(normalize["exported"], true, "normalize should be exported");

    // Find db.ts — internalReset should NOT be exported
    let db_entry = entries
        .iter()
        .find(|e| e["file"].as_str().unwrap().contains("db.ts"))
        .expect("should find db.ts");

    let db_symbols = db_entry["symbols"].as_array().unwrap();
    let internal = db_symbols
        .iter()
        .find(|s| s["name"].as_str() == Some("internalReset"))
        .expect("should find internalReset");
    assert_eq!(
        internal["exported"], false,
        "internalReset should not be exported"
    );
}

#[test]
fn symbols_table_output() {
    let fixture = fixtures_dir().join("python/calls");
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["symbols", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("main"))
        .stdout(predicate::str::contains("function"))
        .stdout(predicate::str::contains("fetch_users"));
}

// ═══════════════════════════════════════════════════════════════════════════════
// kgr refs — "Where is this thing used?"
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn refs_finds_function_definition_and_calls() {
    let fixture = fixtures_dir().join("python/calls");
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["refs", "normalize", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

    // Should have the symbol name echoed back
    assert_eq!(json["symbol"], "normalize");

    // Should find the definition in utils.py
    let defs = json["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1);
    assert!(defs[0]["file"].as_str().unwrap().contains("utils.py"));
    assert_eq!(defs[0]["kind"], "function");

    // Should find the call in app.py
    let refs = json["references"].as_array().unwrap();
    assert!(
        refs.iter()
            .any(|r| r["file"].as_str().unwrap().contains("app.py") && r["kind"] == "call"),
        "should find call reference in app.py"
    );
}

#[test]
fn refs_finds_class_references() {
    let fixture = fixtures_dir().join("python/calls");
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["refs", "UserService", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

    // Definition in service.py
    let defs = json["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1);
    assert!(defs[0]["file"].as_str().unwrap().contains("service.py"));
    assert_eq!(defs[0]["kind"], "class");

    // Instantiation in service.py::fetch_users (svc = UserService())
    let refs = json["references"].as_array().unwrap();
    assert!(
        refs.iter()
            .any(|r| r["file"].as_str().unwrap().contains("service.py") && r["kind"] == "call"),
        "should find UserService() instantiation"
    );
}

#[test]
fn refs_includes_context_line() {
    let fixture = fixtures_dir().join("python/calls");
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["refs", "query", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let refs = json["references"].as_array().unwrap();

    // Every reference should include a context snippet
    for r in refs {
        assert!(
            r["context"].is_string(),
            "each reference should have a 'context' field with the source line"
        );
        let ctx = r["context"].as_str().unwrap();
        assert!(!ctx.is_empty(), "context should not be empty");
    }
}

#[test]
fn refs_no_results_exits_cleanly() {
    let fixture = fixtures_dir().join("python/calls");

    // Should succeed but with empty results (not crash)
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args([
            "refs",
            "nonexistent_symbol",
            "--format",
            "json",
            "--no-progress",
        ])
        .arg(&fixture)
        .assert()
        .success();
    let output = output.get_output().stdout.clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["symbol"], "nonexistent_symbol");
    assert!(json["definitions"].as_array().unwrap().is_empty());
    assert!(json["references"].as_array().unwrap().is_empty());
}

#[test]
fn refs_typescript_exported_function() {
    let fixture = fixtures_dir().join("typescript/calls");
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["refs", "fetchUsers", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

    // Defined in service.ts
    let defs = json["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1);
    assert!(defs[0]["file"].as_str().unwrap().contains("service.ts"));

    // Referenced in app.ts (import + call)
    let refs = json["references"].as_array().unwrap();
    let app_refs: Vec<_> = refs
        .iter()
        .filter(|r| r["file"].as_str().unwrap().contains("app.ts"))
        .collect();
    assert!(!app_refs.is_empty(), "should find references in app.ts");
}

#[test]
fn refs_table_output() {
    let fixture = fixtures_dir().join("python/calls");
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["refs", "normalize", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("normalize"))
        .stdout(predicate::str::contains("utils.py"))
        .stdout(predicate::str::contains("app.py"));
}

// ═══════════════════════════════════════════════════════════════════════════════
// kgr dead — "Is this thing safe to remove?"
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn dead_reports_unused_function() {
    let fixture = fixtures_dir().join("python/calls");
    // deprecated_helper is defined in utils.py but never called
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["dead", "deprecated_helper", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("Dead"))
        .stdout(predicate::str::contains("no references"));
}

#[test]
fn dead_confirms_used_function_is_alive() {
    let fixture = fixtures_dir().join("python/calls");
    // normalize is called in app.py — not dead
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["dead", "normalize", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("Not dead"));
}

#[test]
fn dead_json_shape() {
    let fixture = fixtures_dir().join("python/calls");
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args([
            "dead",
            "deprecated_helper",
            "--format",
            "json",
            "--no-progress",
        ])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(json["symbol"], "deprecated_helper");
    assert_eq!(json["dead"], true);
    assert!(
        json["definition"].is_object(),
        "should include where it's defined"
    );
    assert!(
        json["definition"]["file"]
            .as_str()
            .unwrap()
            .contains("utils.py"),
        "should show the defining file"
    );
    assert!(json["references"].as_array().unwrap().is_empty());
}

#[test]
fn dead_json_alive_includes_references() {
    let fixture = fixtures_dir().join("python/calls");
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["dead", "query", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(json["symbol"], "query");
    assert_eq!(json["dead"], false);
    assert!(
        !json["references"].as_array().unwrap().is_empty(),
        "alive symbol should list its references"
    );
}

#[test]
fn dead_private_python_function() {
    let fixture = fixtures_dir().join("python/calls");
    // _internal_reset is private (leading _) and never called
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["dead", "_internal_reset", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("Dead"));
}

#[test]
fn dead_typescript_unexported_function() {
    let fixture = fixtures_dir().join("typescript/calls");
    // internalReset is not exported and never called
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["dead", "internalReset", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("Dead"));
}

#[test]
fn dead_typescript_exported_unused() {
    let fixture = fixtures_dir().join("typescript/calls");
    // deprecatedHelper is exported but never called anywhere
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["dead", "deprecatedHelper", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("Dead"));
}

#[test]
fn dead_nonexistent_symbol() {
    let fixture = fixtures_dir().join("python/calls");
    // Symbol that doesn't exist at all — should report not found, not crash
    assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["dead", "no_such_thing", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("not found"));
}

// ═══════════════════════════════════════════════════════════════════════════════
// Cross-cutting: symbols in existing graph output
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn graph_json_with_symbols_flag_enriches_file_nodes() {
    let fixture = fixtures_dir().join("python/calls");
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["graph", "--format", "json", "--symbols", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let files = json["files"].as_array().unwrap();

    // With --symbols, each file node should have a symbols array
    for file in files {
        assert!(
            file["symbols"].is_array(),
            "file {} should have symbols array when --symbols is passed",
            file["path"]
        );
    }

    // db.py should have query, connect, _internal_reset
    let db_file = files
        .iter()
        .find(|f| f["path"].as_str().unwrap().contains("db.py"))
        .expect("should find db.py");
    let db_symbols = db_file["symbols"].as_array().unwrap();
    assert!(
        db_symbols.len() >= 3,
        "db.py should have at least 3 symbols"
    );
}
