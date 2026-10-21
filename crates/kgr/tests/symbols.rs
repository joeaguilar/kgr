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

#[test]
fn refs_bare_duplicate_symbol_reports_ambiguity_with_definition_ids() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("alpha.py"),
        "def normalize(value):\n    return value\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("beta.py"),
        "def normalize(value):\n    return value\n",
    )
    .unwrap();

    let output = kgr()
        .args(["refs", "normalize", "--format", "json", "--no-progress"])
        .arg(dir.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["symbol"], "normalize");
    assert_eq!(json["ambiguous"], true);
    assert!(json["scope_hint"]
        .as_str()
        .unwrap()
        .contains("--defined-in"));

    let definitions = json["definitions"].as_array().unwrap();
    assert_eq!(definitions.len(), 2);
    let ids: Vec<&str> = definitions
        .iter()
        .map(|definition| definition["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1], "definition ids must identify the file");
    assert!(json["references"].as_array().unwrap().is_empty());
}

#[test]
fn refs_defined_in_filters_duplicate_symbol_references_to_that_definition() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("alpha.py"),
        "def normalize(value):\n    return value\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("beta.py"),
        "def normalize(value):\n    return value\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("use_alpha.py"),
        "import alpha\n\n\ndef run():\n    return alpha.normalize('a')\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("use_beta.py"),
        "import beta\n\n\ndef run():\n    return beta.normalize('b')\n",
    )
    .unwrap();

    let output = kgr()
        .args([
            "refs",
            "normalize",
            "--defined-in",
            "alpha.py",
            "--format",
            "json",
            "--no-progress",
        ])
        .arg(dir.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["ambiguous"], false);

    let definitions = json["definitions"].as_array().unwrap();
    assert_eq!(definitions.len(), 1);
    assert!(definitions[0]["file"]
        .as_str()
        .unwrap()
        .ends_with("alpha.py"));
    assert!(definitions[0]["id"].is_string());

    let refs = json["references"].as_array().unwrap();
    assert!(
        refs.iter()
            .any(|r| r["file"].as_str().unwrap().ends_with("use_alpha.py")),
        "scoped refs should include the alpha caller"
    );
    assert!(
        !refs
            .iter()
            .any(|r| r["file"].as_str().unwrap().ends_with("use_beta.py")),
        "scoped refs should not include same-name calls to beta.normalize"
    );
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
fn dead_rust_enum_variants_are_alive_when_used_in_match_patterns() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("defs.rs"),
        r#"
pub enum Flow {
    Ready(String),
    Waiting { id: u32 },
    Done,
}
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("consumer.rs"),
        r#"
fn consume(flow: crate::defs::Flow) {
    match flow {
        crate::defs::Flow::Ready(value) => drop(value),
        crate::defs::Flow::Waiting { id } => drop(id),
        _ => {}
    }
}
"#,
    )
    .unwrap();

    for variant in ["Ready", "Waiting"] {
        kgr()
            .args(["dead", variant, "--no-progress"])
            .arg(dir.path())
            .assert()
            .success()
            .stdout(predicate::str::contains("Not dead"));
    }
}

#[test]
fn dead_rust_impl_and_where_bounds_keep_traits_alive() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("defs.rs"),
        r#"
pub trait Repository {}
pub trait Entity {}
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("impls.rs"),
        r#"
struct Adapter<T>(T);

impl<T> crate::defs::Repository for Adapter<T>
where
    T: crate::defs::Entity,
{
}
"#,
    )
    .unwrap();

    for symbol in ["Repository", "Entity"] {
        kgr()
            .args(["dead", symbol, "--no-progress"])
            .arg(dir.path())
            .assert()
            .success()
            .stdout(predicate::str::contains("Not dead"));
    }
}

#[test]
fn dead_rust_derive_and_attribute_paths_keep_symbols_alive() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("markers.rs"),
        r#"
pub struct Track;
pub fn audit_attr() {}
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("model.rs"),
        r#"
#[crate::markers::audit_attr]
#[derive(crate::markers::Track)]
struct Event;
"#,
    )
    .unwrap();

    for symbol in ["Track", "audit_attr"] {
        kgr()
            .args(["dead", symbol, "--no-progress"])
            .arg(dir.path())
            .assert()
            .success()
            .stdout(predicate::str::contains("Not dead"));
    }
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
    assert_eq!(json["status"], "dead");
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
    assert_eq!(json["status"], "live");
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
    assert_eq!(json["status"], "dead");
    assert!(json["caveat"].as_str().unwrap().contains("recursion"));

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

#[test]
fn dead_bare_duplicate_symbol_reports_ambiguity_instead_of_global_verdict() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("unused.py"), "def helper():\n    pass\n").unwrap();
    std::fs::write(dir.path().join("used.py"), "def helper():\n    pass\n").unwrap();
    std::fs::write(
        dir.path().join("caller.py"),
        "import used\n\n\ndef run():\n    used.helper()\n",
    )
    .unwrap();

    let output = kgr()
        .args(["dead", "helper", "--format", "json", "--no-progress"])
        .arg(dir.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["found"], true);
    assert_eq!(json["ambiguous"], true);
    assert_eq!(json["dead"], serde_json::Value::Null);
    assert_eq!(json["status"], "ambiguous");
    assert!(json["scope_hint"]
        .as_str()
        .unwrap()
        .contains("--defined-in"));

    let definitions = json["definitions"].as_array().unwrap();
    assert_eq!(definitions.len(), 2);
    assert!(definitions
        .iter()
        .all(|definition| definition["id"].is_string()));
}

#[test]
fn dead_defined_in_checks_only_the_selected_duplicate_symbol() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("unused.py"), "def helper():\n    pass\n").unwrap();
    std::fs::write(dir.path().join("used.py"), "def helper():\n    pass\n").unwrap();
    std::fs::write(
        dir.path().join("caller.py"),
        "import used\n\n\ndef run():\n    used.helper()\n",
    )
    .unwrap();

    let used_output = kgr()
        .args([
            "dead",
            "helper",
            "--defined-in",
            "used.py",
            "--format",
            "json",
            "--no-progress",
        ])
        .arg(dir.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let used_json: serde_json::Value = serde_json::from_slice(&used_output).unwrap();
    assert_eq!(used_json["ambiguous"], false);
    assert_eq!(used_json["dead"], false);
    assert!(
        used_json["cross_file_references"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["file"].as_str().unwrap().ends_with("caller.py")),
        "used.py helper should be kept alive by caller.py"
    );

    let unused_output = kgr()
        .args([
            "dead",
            "helper",
            "--defined-in",
            "unused.py",
            "--format",
            "json",
            "--no-progress",
        ])
        .arg(dir.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let unused_json: serde_json::Value = serde_json::from_slice(&unused_output).unwrap();
    assert_eq!(unused_json["ambiguous"], false);
    assert_eq!(unused_json["dead"], true);
    assert!(unused_json["references"].as_array().unwrap().is_empty());
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

// ═══════════════════════════════════════════════════════════════════════════════
// kgr dead — tri-state verdicts and non-call references (itr #6, #7)
// ═══════════════════════════════════════════════════════════════════════════════

fn dead_verdict(files: &[(&str, &str)], args: &[&str]) -> serde_json::Value {
    let dir = tempfile::tempdir().unwrap();
    for (path, content) in files {
        let full = dir.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, content).unwrap();
    }
    let output = kgr()
        .arg("dead")
        .args(args)
        .args(["--format", "json", "--no-progress"])
        .arg(dir.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).unwrap()
}

#[test]
fn dead_cpp_arrow_call_keeps_method_live() {
    let json = dead_verdict(
        &[
            ("w.cpp", "struct W { void m(); };\nvoid W::m() {}\n"),
            (
                "main.cpp",
                "#include \"w.cpp\"\nint main() { W* p = new W(); p->m(); }\n",
            ),
        ],
        &["m"],
    );
    assert_eq!(json["dead"], false, "{json:#}");
}

#[test]
fn dead_lua_colon_call_keeps_method_live() {
    let json = dead_verdict(
        &[
            (
                "obj.lua",
                "local Obj = {}\nfunction Obj:greet() return 1 end\nreturn Obj\n",
            ),
            ("main.lua", "local Obj = require(\"obj\")\nObj:greet()\n"),
        ],
        &["greet"],
    );
    assert_eq!(json["dead"], false, "{json:#}");
}

#[test]
fn dead_rust_turbofish_call_keeps_function_live() {
    let json = dead_verdict(
        &[
            ("src/lib.rs", "pub mod util;\n"),
            (
                "src/util.rs",
                "pub fn parse_as<T: Default>() -> T { T::default() }\n",
            ),
            (
                "src/main.rs",
                "fn main() { let _n = demo::util::parse_as::<u32>(); }\n",
            ),
        ],
        &["parse_as"],
    );
    assert_eq!(json["dead"], false, "{json:#}");
}

#[test]
fn dead_jsx_tag_keeps_component_live() {
    let json = dead_verdict(
        &[
            (
                "src/Button.tsx",
                "export function Button() { return <button />; }\n",
            ),
            (
                "src/App.tsx",
                "import { Button } from \"./Button\";\nexport const App = () => <Button />;\n",
            ),
        ],
        &["Button"],
    );
    assert_eq!(json["dead"], false, "{json:#}");
    assert_eq!(json["status"], "live");
}

#[test]
fn dead_python_decorator_keeps_function_live() {
    let json = dead_verdict(
        &[
            ("deco.py", "def traced(fn):\n    return fn\n"),
            (
                "app.py",
                "from deco import traced\n\n@traced\ndef run():\n    pass\n",
            ),
        ],
        &["traced"],
    );
    assert_eq!(json["dead"], false, "{json:#}");
}

#[test]
fn dead_ambiguous_cross_definition_call_is_not_dead() {
    // a.py calls b.helper(); both files define helper. The call sits in a
    // definition file but outside either definition, so it is live evidence.
    let json = dead_verdict(
        &[
            (
                "a.py",
                "import b\n\n\ndef helper():\n    pass\n\n\ndef go():\n    b.helper()\n",
            ),
            ("b.py", "def helper():\n    pass\n"),
        ],
        &["helper"],
    );
    assert_eq!(json["ambiguous"], true);
    assert!(json["dead"].is_null(), "{json:#}");
    assert_eq!(json["status"], "ambiguous");
}

#[test]
fn dead_same_file_caller_is_file_local_not_dead() {
    let json = dead_verdict(
        &[(
            "engine.py",
            "class Engine:\n    def run(self):\n        return self.step()\n\n    def step(self):\n        return 2\n",
        )],
        &["step"],
    );
    assert_eq!(json["dead"], false);
    assert_eq!(json["status"], "file_local");
}

#[test]
fn dead_name_only_in_string_literal_is_dead_with_caveat() {
    let json = dead_verdict(
        &[
            ("handlers.py", "def on_save():\n    pass\n"),
            ("routes.py", "ROUTES = {\"save\": \"on_save\"}\n"),
        ],
        &["on_save"],
    );
    assert_eq!(json["dead"], true);
    assert_eq!(json["string_mentions"].as_array().unwrap().len(), 1);
    assert!(json["caveat"].as_str().unwrap().contains("dynamic use"));
}

#[test]
fn dead_name_only_in_comment_is_dead() {
    let json = dead_verdict(
        &[
            ("util.rs", "pub fn legacy() {}\n"),
            (
                "main.rs",
                "// legacy() was removed from the hot path\nfn main() {}\n",
            ),
        ],
        &["legacy"],
    );
    assert_eq!(json["dead"], true, "{json:#}");
}

// Regression cases from the cross-model review of the dead/refs rework.

fn kgr_json(files: &[(&str, &str)], args: &[&str]) -> serde_json::Value {
    let dir = tempfile::tempdir().unwrap();
    for (path, content) in files {
        let full = dir.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, content).unwrap();
    }
    let output = kgr()
        .args(args)
        .args(["--format", "json", "--no-progress"])
        .arg(dir.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).unwrap()
}

#[test]
fn dead_call_after_definition_on_same_line_is_live() {
    let json = dead_verdict(
        &[("main.js", "function helper() {} helper();\n")],
        &["helper"],
    );
    assert_eq!(json["dead"], false, "{json:#}");
}

#[test]
fn dead_value_reference_on_definition_line_is_live() {
    let json = dead_verdict(
        &[("main.js", "function helper() {} const cb = helper; cb();\n")],
        &["helper"],
    );
    assert_eq!(json["dead"], false, "{json:#}");
}

#[test]
fn dead_qualified_call_inside_same_named_definition_is_not_recursion() {
    let json = dead_verdict(
        &[
            ("a.py", "import b\ndef helper():\n    b.helper()\n"),
            ("b.py", "def helper():\n    pass\n"),
        ],
        &["helper"],
    );
    assert!(json["dead"].is_null(), "{json:#}");
    assert_eq!(json["status"], "ambiguous");
}

#[test]
fn dead_code_after_inline_block_comment_is_live() {
    let json = dead_verdict(
        &[(
            "main.rs",
            "fn helper() {}\nfn main() {\n    /* setup */ let callback = helper;\n    callback();\n}\n",
        )],
        &["helper"],
    );
    assert_eq!(json["dead"], false, "{json:#}");
}

#[test]
fn dead_deref_assignment_is_not_a_comment() {
    let json = dead_verdict(
        &[(
            "main.rs",
            "fn helper() {}\nfn main() {\n    let mut callback: fn() = || {};\n    let slot = &mut callback;\n    * slot = helper;\n    callback();\n}\n",
        )],
        &["helper"],
    );
    assert_eq!(json["dead"], false, "{json:#}");
}

#[test]
fn dead_multiline_block_comment_prose_is_not_a_reference() {
    let json = dead_verdict(
        &[(
            "main.rs",
            "fn helper() {}\n/*\nhelper is obsolete\n*/\nfn main() {}\n",
        )],
        &["helper"],
    );
    assert_eq!(json["dead"], true, "{json:#}");
}

#[test]
fn dead_fstring_interpolation_is_code() {
    let json = dead_verdict(
        &[(
            "main.py",
            "def helper():\n    return 1\n\ncallbacks = []\nprint(f\"{callbacks.append(helper)}\")\ncallbacks[0]()\n",
        )],
        &["helper"],
    );
    assert_eq!(json["dead"], false, "{json:#}");
}

#[test]
fn dead_code_after_rust_char_literal_is_live() {
    let json = dead_verdict(
        &[(
            "main.rs",
            "fn helper() {}\nfn main() {\n    let quote = '\"'; let cb = helper;\n    cb();\n}\n",
        )],
        &["helper"],
    );
    assert_eq!(json["dead"], false, "{json:#}");
}

#[test]
fn dead_template_literal_text_is_a_string_mention() {
    let json = dead_verdict(
        &[(
            "main.js",
            "function helper() {}\nconst msg = `helper is obsolete`;\n",
        )],
        &["helper"],
    );
    assert_eq!(json["dead"], true, "{json:#}");
    assert_eq!(json["string_mentions"].as_array().unwrap().len(), 1);
}

#[test]
fn dead_qualifier_aliasing_our_module_keeps_definition_live() {
    let json = dead_verdict(
        &[
            ("a.py", "def helper():\n    return 42\n"),
            ("other.py", "def helper():\n    return 0\n"),
            (
                "main.py",
                "import a as other\ncallbacks = [other.helper]\nprint(callbacks[0]())\n",
            ),
        ],
        &["helper", "--defined-in", "a.py"],
    );
    assert_eq!(json["dead"], false, "{json:#}");
}

#[test]
fn dead_dict_key_colon_is_not_a_qualifier() {
    let json = dead_verdict(
        &[
            ("a.py", "def helper():\n    return 42\n"),
            ("other.py", "def helper():\n    return 0\n"),
            (
                "main.py",
                "from a import *\nother = \"x\"\nregistry = {other: helper}\nprint(registry[other]())\n",
            ),
        ],
        &["helper", "--defined-in", "a.py"],
    );
    assert_eq!(json["dead"], false, "{json:#}");
}

#[test]
fn refs_ignores_as_inside_strings_and_comments() {
    for noise in [
        "query = \"SELECT helper as unrelated FROM util\"",
        "# util.helper as unrelated",
    ] {
        let main = format!("import util\n{noise}\ndef unrelated():\n    pass\nunrelated()\n");
        let files = [
            ("util.py", "def helper():\n    pass\n"),
            ("main.py", main.as_str()),
        ];
        let refs = kgr_json(&files, &["refs", "helper"]);
        assert!(
            refs["references"].as_array().unwrap().is_empty(),
            "{refs:#}"
        );
        let impact = kgr_json(&files, &["impact", "helper"]);
        assert_eq!(impact["impact"][0]["calls_symbol"], false, "{impact:#}");
    }
}

#[test]
fn refs_and_impact_follow_multiline_aliased_import() {
    let files = [
        ("util.py", "def helper():\n    return 1\n"),
        ("main.py", "from util import (\n    helper as h,\n)\nh()\n"),
    ];
    let refs = kgr_json(&files, &["refs", "helper"]);
    assert_eq!(refs["references"][0]["via_alias"], "h", "{refs:#}");
    let impact = kgr_json(&files, &["impact", "helper"]);
    assert_eq!(impact["impact"][0]["calls_symbol"], true, "{impact:#}");
}

#[test]
fn refs_keeps_callee_with_comparison_in_index() {
    let files = [
        (
            "util.py",
            "class Widget:\n    def helper(self):\n        return 42\n",
        ),
        (
            "main.py",
            "import util\nitems = [util.Widget(), util.Widget()]\ni, j = 0, 1\nitems[i < j].helper()\n",
        ),
    ];
    let refs = kgr_json(&files, &["refs", "helper"]);
    assert_eq!(refs["references"].as_array().unwrap().len(), 1, "{refs:#}");
}

// Second-pass review cases: a lexing mistake must never hide a live reference.

fn assert_live(files: &[(&str, &str)], name: &str) {
    let json = dead_verdict(files, &[name]);
    assert_eq!(json["dead"], false, "{json:#}");
}

#[test]
fn dead_call_adjacent_to_definition_end_is_not_recursion() {
    assert_live(&[("main.js", "function helper() {}helper();\n")], "helper");
}

#[test]
fn dead_call_on_constructed_receiver_is_not_recursion() {
    let json = dead_verdict(
        &[(
            "main.py",
            "class B:\n    def helper(self): return 1\ndef helper():\n    return B().helper()\n",
        )],
        &["helper"],
    );
    assert!(json["dead"].is_null(), "{json:#}");
}

#[test]
fn dead_super_call_is_not_recursion() {
    let json = dead_verdict(
        &[(
            "main.py",
            "class A:\n    def helper(self): return 1\nclass B(A):\n    def helper(self): return super().helper()\n",
        )],
        &["helper"],
    );
    assert!(json["dead"].is_null(), "{json:#}");
}

#[test]
fn dead_shadowed_foreign_qualifier_keeps_definition_live() {
    // `other` is both an imported module and a parameter, so `other.helper`
    // may be a.py's own helper (it is, at runtime): it must count as live.
    let json = dead_verdict(
        &[
            (
                "a.py",
                "import sys
import other
def helper(): return 42
def pick(other):
    return other.helper
cb = pick(sys.modules[__name__])
print(cb())
",
            ),
            (
                "other.py",
                "def helper(): return 0
",
            ),
        ],
        &["helper", "--defined-in", "a.py"],
    );
    assert_eq!(json["dead"], false, "{json:#}");
}

#[test]
fn dead_jsx_text_punctuation_does_not_hide_components() {
    assert_live(
        &[(
            "main.jsx",
            "function Helper() { return <span/>; }\nconst view = <div>don't <Helper /></div>;\n",
        )],
        "Helper",
    );
    assert_live(
        &[(
            "main.tsx",
            "function Helper() { return <span/>; }\r\nconst view = <div>say \"hi <Helper /></div>;\r\n",
        )],
        "Helper",
    );
    assert_live(
        &[(
            "main.jsx",
            "function Helper() { return <span/>; }\nconst v = <p>it's <Helper/> isn't</p>;\n",
        )],
        "Helper",
    );
}

#[test]
fn dead_js_regex_with_quote_does_not_hide_code() {
    assert_live(
        &[(
            "main.js",
            "function helper() {}\nconst pattern = /\"/; const cb = helper; cb();\n",
        )],
        "helper",
    );
}

#[test]
fn dead_rust_byte_char_and_nested_comment_do_not_hide_code() {
    assert_live(
        &[(
            "main.rs",
            "fn helper() {}\nfn main() { let quote = b'\"'; let cb = helper; cb(); }\n",
        )],
        "helper",
    );
    assert_live(
        &[(
            "main.rs",
            "fn helper() {}\n/* outer /* inner */ \" */\nfn main() { let cb = helper; cb(); }\n",
        )],
        "helper",
    );
}

#[test]
fn dead_python_raw_string_escape_does_not_hide_code() {
    assert_live(
        &[(
            "main.py",
            "def helper(): pass\ns = r\"\\\"\"; cb = helper\ncb()\n",
        )],
        "helper",
    );
}

#[test]
fn dead_cpp_raw_string_does_not_open_a_comment() {
    assert_live(
        &[(
            "main.cpp",
            "void helper() {}\nint main() { const char* s = R\"tag(\" /*)tag\"; auto cb = helper; cb(); }\n",
        )],
        "helper",
    );
}

#[test]
fn dead_ruby_char_and_percent_literals_do_not_hide_code() {
    assert_live(
        &[(
            "main.rb",
            "def helper; puts :ok; end\nx = ?'; cb = method(:helper); cb.call\n",
        )],
        "helper",
    );
    assert_live(
        &[(
            "main.rb",
            "def helper; puts :ok; end\nx = %w[\"]\ncb = method(:helper)\ncb.call\n",
        )],
        "helper",
    );
}

#[test]
fn dead_bash_ansi_quote_and_heredoc_do_not_hide_code() {
    assert_live(
        &[(
            "main.sh",
            "helper() { echo ok; }\ns=$'\\''; cb=helper\n$cb\n",
        )],
        "helper",
    );
    assert_live(
        &[(
            "main.sh",
            "helper() { echo ok; }\ncat <<EOF\n\"\nEOF\ncb=helper\n$cb\n",
        )],
        "helper",
    );
}

#[test]
fn dead_php_heredoc_does_not_hide_code() {
    assert_live(
        &[(
            "main.php",
            "<?php\nclass helper {}\n$s = <<<TXT\n\"\nTXT;\n$cls = helper::class;\nnew $cls();\n",
        )],
        "helper",
    );
}

#[test]
fn dead_lua_long_string_does_not_hide_code() {
    assert_live(
        &[(
            "main.lua",
            "function helper() end\nlocal s = [[ \" ]]; local cb = helper; cb()\n",
        )],
        "helper",
    );
}

#[test]
fn dead_swift_interpolation_is_code() {
    assert_live(
        &[(
            "main.swift",
            "func helper() {}\nlet s = \"\\([helper].count)\"\n",
        )],
        "helper",
    );
}

// Third-pass review cases: tree-sitter literal ranges.

#[test]
fn dead_js_regex_with_comment_opener_does_not_hide_code() {
    assert_live(
        &[(
            "main.js",
            "function helper() {}\nconst punctuation = /[/*]/;\nconst cb = helper;\n/* Invoke the selected callback. */\ncb();\n",
        )],
        "helper",
    );
    assert_live(
        &[(
            "main.js",
            "function helper() {}\nconst url = /https?:\\/\\//; const cb = helper; cb();\n",
        )],
        "helper",
    );
}

#[test]
fn dead_paired_quotes_in_jsx_text_do_not_hide_components() {
    assert_live(
        &[(
            "main.jsx",
            "function Helper() { return <span/>; }\nconst view = <div>Click \"<Helper />\" to continue</div>;\n",
        )],
        "Helper",
    );
    assert_live(
        &[(
            "main.tsx",
            "function Helper() { return <span/>; }\nconst view = <div>Use `\n<Helper />\n` here</div>;\n",
        )],
        "Helper",
    );
}

#[test]
fn dead_cpp_wide_char_does_not_hide_code() {
    assert_live(
        &[(
            "main.cpp",
            "void helper() {}\nint main() { wchar_t q = L'\"'; auto cb = helper; puts(\"ready\"); cb(); }\n",
        )],
        "helper",
    );
}

#[test]
fn dead_ruby_percent_and_regex_literals_do_not_hide_code() {
    assert_live(
        &[(
            "main.rb",
            "def helper; puts :ok; end\nwords = %w[\"]; cb = method(:helper); puts \"ready\"; cb.call\n",
        )],
        "helper",
    );
    assert_live(
        &[(
            "main.rb",
            "def helper; puts :ok; end\nr = /\"/; cb = method(:helper); puts \"ready\"; cb.call\n",
        )],
        "helper",
    );
}

#[test]
fn dead_interpolation_forms_are_code() {
    assert_live(
        &[(
            "main.py",
            "def helper(): pass\ntext = rf\"\\{helper.__name__}\"\n",
        )],
        "helper",
    );
    assert_live(
        &[(
            "main.swift",
            "func helper() {}\nlet s = \"\"\"\nCount: \\([helper].count)\n\"\"\"\n",
        )],
        "helper",
    );
    assert_live(
        &[(
            "main.cs",
            "class C {\n    static int helper() => 1;\n    string s = $\"{new System.Func<int>(helper)()}\";\n}\n",
        )],
        "helper",
    );
    assert_live(
        &[(
            "main.sh",
            "helper() { echo ok; }\noutput=\"$(cb=helper; $cb)\"\n",
        )],
        "helper",
    );
}

#[test]
fn dead_bound_method_on_constructed_receiver_is_not_recursion() {
    let json = dead_verdict(
        &[(
            "main.py",
            "class B:\n    def helper(self): return 42\ndef helper():\n    return B().helper\n",
        )],
        &["helper"],
    );
    assert!(json["dead"].is_null(), "{json:#}");
}
