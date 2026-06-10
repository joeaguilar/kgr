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

fn kgr() -> assert_cmd::Command {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("kgr");
    strip_host_kgr_env(&mut cmd);
    cmd.env("KGR_NO_CACHE", "1");
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

// ═══════════════════════════════════════════════════════════════════════════════
// kgr symbols — "What's defined here?"
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn symbols_python_json_returns_all_definitions() {
    let fixture = fixtures_dir().join("python/calls");
    let output = kgr()
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
    let output = kgr()
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
    let output = kgr()
        .args(["symbols", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let entries = json.as_array().unwrap();
    assert!(
        !entries.is_empty(),
        "symbols json should have at least one file entry"
    );

    // Each entry should have file + symbols
    for entry in entries {
        assert!(entry["file"].is_string(), "entry missing 'file' field");
        let symbols = entry["symbols"]
            .as_array()
            .expect("entry missing 'symbols' array");
        assert!(
            !symbols.is_empty(),
            "entry {} should have at least one symbol",
            entry["file"]
        );
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
    let output = kgr()
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
    kgr()
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
    let output = kgr()
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
    let output = kgr()
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
    let output = kgr()
        .args(["refs", "query", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let refs = json["references"].as_array().unwrap();
    assert!(
        !refs.is_empty(),
        "query is called in service.py — references must not be empty"
    );

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
    let output = kgr()
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
    let output = kgr()
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
    kgr()
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
    kgr()
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
    kgr()
        .args(["dead", "normalize", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("Not dead"));
}

#[test]
fn dead_json_shape() {
    let fixture = fixtures_dir().join("python/calls");
    let output = kgr()
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
    assert_eq!(json["found"], true);
    assert_eq!(json["dead"], true);
    assert_eq!(json["status"], "no_references");
    let definitions = json["definitions"].as_array().unwrap();
    assert_eq!(definitions.len(), 1, "should include where it's defined");
    assert!(
        definitions[0]["file"]
            .as_str()
            .unwrap()
            .contains("utils.py"),
        "should show the defining file"
    );
    assert!(json["references"].as_array().unwrap().is_empty());
    assert!(json["self_file_references"].as_array().unwrap().is_empty());
    assert!(json["cross_file_references"].as_array().unwrap().is_empty());
}

#[test]
fn dead_json_alive_includes_references() {
    let fixture = fixtures_dir().join("python/calls");
    let output = kgr()
        .args(["dead", "query", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(json["symbol"], "query");
    assert_eq!(json["found"], true);
    assert_eq!(json["dead"], false);
    assert_eq!(json["status"], "live_cross_file_references");
    assert!(
        !json["references"].as_array().unwrap().is_empty(),
        "alive symbol should list its references"
    );
    assert!(json["self_file_references"].as_array().unwrap().is_empty());
    let cross_refs = json["cross_file_references"].as_array().unwrap();
    assert!(
        cross_refs
            .iter()
            .any(|r| r["file"].as_str().unwrap().contains("service.py")),
        "cross-file references should include the service.py caller"
    );
}

#[test]
fn dead_json_self_recursive_symbol_is_self_only_not_live() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("worker.py"),
        "def tick(n):\n    if n <= 0:\n        return 0\n    return tick(n - 1)\n",
    )
    .unwrap();

    let output = kgr()
        .args(["dead", "tick", "--format", "json", "--no-progress"])
        .arg(dir.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(json["symbol"], "tick");
    assert_eq!(json["found"], true);
    assert_eq!(json["dead"], true);
    assert_eq!(json["status"], "self_only_references");
    assert!(json["caveat"]
        .as_str()
        .unwrap()
        .contains("self-file references"));

    let refs = json["references"].as_array().unwrap();
    let self_refs = json["self_file_references"].as_array().unwrap();
    let cross_refs = json["cross_file_references"].as_array().unwrap();
    assert!(
        !self_refs.is_empty(),
        "recursive call should be classified as a self-file reference"
    );
    assert_eq!(
        refs.len(),
        self_refs.len(),
        "all references should be self-file references"
    );
    assert!(
        cross_refs.is_empty(),
        "self-recursive symbol should not have cross-file references"
    );
}

#[test]
fn dead_text_names_self_only_references() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("worker.py"),
        "def tick(n):\n    if n <= 0:\n        return 0\n    return tick(n - 1)\n",
    )
    .unwrap();

    kgr()
        .args(["dead", "tick", "--no-progress"])
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("only self-file reference"))
        .stdout(predicate::str::contains("Self-file references"))
        .stdout(predicate::str::contains("Not dead").not());
}

#[test]
fn dead_private_python_function() {
    let fixture = fixtures_dir().join("python/calls");
    // _internal_reset is private (leading _) and never called
    kgr()
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
    kgr()
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
    kgr()
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
    kgr()
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
    let output = kgr()
        .args(["graph", "--format", "json", "--symbols", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let files = json["files"].as_array().unwrap();
    assert!(!files.is_empty(), "graph should have file nodes");

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

// ═══════════════════════════════════════════════════════════════════════════════
// kgr skeleton — "Show me just the signatures"
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn skeleton_text_output() {
    let fixture = fixtures_dir().join("python/calls");
    kgr()
        .args(["skeleton", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("main"))
        .stdout(predicate::str::contains("fetch_users"));
}

#[test]
fn skeleton_json_output() {
    let fixture = fixtures_dir().join("python/calls");
    let output = kgr()
        .args(["skeleton", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let entries = json.as_array().expect("top-level should be array");
    assert!(
        !entries.is_empty(),
        "skeleton json should have at least one file entry"
    );

    // Every entry should have file + skeleton
    for entry in entries {
        assert!(entry["file"].is_string(), "entry missing 'file'");
        let skeleton = entry["skeleton"]
            .as_array()
            .expect("entry missing 'skeleton' array");
        assert!(
            !skeleton.is_empty(),
            "entry {} should have at least one skeleton item",
            entry["file"]
        );
        for item in skeleton {
            assert!(item["name"].is_string(), "skeleton item missing 'name'");
            assert!(item["kind"].is_string(), "skeleton item missing 'kind'");
            assert!(item["line"].is_number(), "skeleton item missing 'line'");
            assert!(
                item["signature"].is_string(),
                "skeleton item missing 'signature'"
            );
        }
    }
}

#[test]
fn skeleton_table_output() {
    let fixture = fixtures_dir().join("python/calls");
    kgr()
        .args(["skeleton", "--format", "table", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("FILE"))
        .stdout(predicate::str::contains("SIGNATURE"))
        .stdout(predicate::str::contains("main"));
}

// ═══════════════════════════════════════════════════════════════════════════════
// kgr orient — "One-shot codebase overview"
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn orient_text_output() {
    let fixture = fixtures_dir().join("python/simple");
    kgr()
        .args(["orient", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("files"));
}

#[test]
fn orient_json_output() {
    let fixture = fixtures_dir().join("python/simple");
    kgr()
        .args(["orient", "--format", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("languages"));
}

// ═══════════════════════════════════════════════════════════════════════════════
// kgr impact — "What's the blast radius of changing this symbol?"
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn impact_text_output() {
    let fixture = fixtures_dir().join("typescript/calls");
    kgr()
        .args(["impact", "query", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("query"))
        .stdout(predicate::str::contains("Defined in"))
        .stdout(predicate::str::contains("Impact"));
}

#[test]
fn impact_json_output() {
    let fixture = fixtures_dir().join("typescript/calls");
    let output = kgr()
        .args(["impact", "query", "-f", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["symbol"], "query");
    assert_eq!(json["found"], true);
    let definitions = json["definitions"].as_array().unwrap();
    assert_eq!(definitions.len(), 1, "should have one definition");
    assert!(
        definitions[0]["file"].as_str().unwrap().contains("db.ts"),
        "should be defined in db.ts"
    );
    assert_eq!(definitions[0]["kind"], "function");
    assert!(json["impact"].is_array(), "should have impact array");

    // db.ts -> service.ts -> app.ts, so impact should have 2 entries
    let impact = json["impact"].as_array().unwrap();
    assert_eq!(impact.len(), 2, "should have 2 affected files");
}

#[test]
fn impact_json_shows_calls_symbol() {
    let fixture = fixtures_dir().join("typescript/calls");
    let output = kgr()
        .args(["impact", "query", "-f", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let impact = json["impact"].as_array().unwrap();

    // service.ts calls query directly (depth 1), so calls_symbol should be true
    let service_entry = impact
        .iter()
        .find(|e| e["file"].as_str().unwrap().contains("service.ts"));
    assert!(
        service_entry.is_some(),
        "service.ts should be in the impact list"
    );
    assert_eq!(
        service_entry.unwrap()["calls_symbol"],
        true,
        "service.ts should call query"
    );
    assert_eq!(
        service_entry.unwrap()["depth"],
        1,
        "service.ts should be at depth 1"
    );

    // app.ts is at depth 2 and does not call query directly
    let app_entry = impact
        .iter()
        .find(|e| e["file"].as_str().unwrap().contains("app.ts"));
    assert!(app_entry.is_some(), "app.ts should be in the impact list");
    assert_eq!(
        app_entry.unwrap()["calls_symbol"],
        false,
        "app.ts should not call query directly"
    );
    assert_eq!(
        app_entry.unwrap()["depth"],
        2,
        "app.ts should be at depth 2"
    );
}

#[test]
fn impact_not_found_text() {
    let fixture = fixtures_dir().join("python/calls");
    kgr()
        .args(["impact", "nonexistent_symbol", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "Symbol 'nonexistent_symbol' not found",
        ));
}

#[test]
fn impact_not_found_json() {
    let fixture = fixtures_dir().join("python/calls");
    let output = kgr()
        .args([
            "impact",
            "nonexistent_symbol",
            "-f",
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
    assert_eq!(json["symbol"], "nonexistent_symbol");
    assert_eq!(json["found"], false);
    assert!(json["error"].is_string(), "should have error field");
    assert!(json["definitions"].as_array().unwrap().is_empty());
    assert!(json["impact"].as_array().unwrap().is_empty());
}

// ═══════════════════════════════════════════════════════════════════════════════
// kgr hotspots — "Where are the complex files?"
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn hotspots_table_output() {
    let fixture = fixtures_dir().join("python/simple");
    kgr()
        .args(["hotspots", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("FUNCTIONS"));
}

#[test]
fn hotspots_json_output() {
    let fixture = fixtures_dir().join("python/simple");
    kgr()
        .args(["hotspots", "-f", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("score"));
}

#[test]
fn hotspots_text_output() {
    let fixture = fixtures_dir().join("python/calls");
    kgr()
        .args(["hotspots", "-f", "text", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .stdout(predicate::str::contains("functions"))
        .stdout(predicate::str::contains("score"));
}

#[test]
fn hotspots_json_shape() {
    let fixture = fixtures_dir().join("python/calls");
    let output = kgr()
        .args(["hotspots", "-f", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let entries = json.as_array().expect("top-level should be an array");
    assert!(
        !entries.is_empty(),
        "hotspots json should have at least one entry"
    );

    for entry in entries {
        assert!(entry["file"].is_string(), "entry missing 'file'");
        assert!(entry["functions"].is_number(), "entry missing 'functions'");
        assert!(
            entry["avg_length"].is_number(),
            "entry missing 'avg_length'"
        );
        assert!(
            entry["max_length"].is_number(),
            "entry missing 'max_length'"
        );
        assert!(entry["score"].is_number(), "entry missing 'score'");
    }
}

#[test]
fn hotspots_sorted_descending() {
    let fixture = fixtures_dir().join("python/calls");
    let output = kgr()
        .args(["hotspots", "-f", "json", "--no-progress"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let entries = json.as_array().unwrap();

    let scores: Vec<u64> = entries
        .iter()
        .map(|e| e["score"].as_u64().unwrap())
        .collect();
    assert!(
        scores.len() >= 2,
        "fixture has multiple files with functions — need at least 2 entries to check ordering"
    );

    for w in scores.windows(2) {
        assert!(w[0] >= w[1], "scores should be in descending order");
    }
}
