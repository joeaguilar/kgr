use std::path::{Path, PathBuf};

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

fn kgr_output(fixture: &str, format: &str) -> String {
    let fixture_path = fixtures_dir().join(fixture);
    kgr_output_path(&fixture_path, format)
}

// All helpers set KGR_NO_CACHE=1 so fixture scans are hermetic: every run
// re-parses sources, never reads a stale `.kgr-cache.json`, and never writes
// one into tests/fixtures/. A warm cache must not be able to mask a parser
// regression in snapshot output.

fn kgr_output_path(path: &Path, format: &str) -> String {
    let output = kgr()
        .args(["graph", "--format", format, "--no-progress"])
        .arg(path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).unwrap()
}

fn kgr_orient_json_path(path: &Path) -> String {
    let output = kgr()
        .args(["orient", "--format", "json", "--no-progress"])
        .arg(path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).unwrap()
}

fn kgr_check_stderr(fixture: &str) -> String {
    let fixture_path = fixtures_dir().join(fixture);
    let output = kgr()
        .args(["check", "--no-progress"])
        .arg(&fixture_path)
        .output()
        .unwrap();
    String::from_utf8(output.stderr).unwrap()
}

macro_rules! snapshot_format {
    ($name:ident, $fixture:expr, $format:expr) => {
        #[test]
        fn $name() {
            let stdout = kgr_output($fixture, $format);
            let fixtures = fixtures_dir();
            let fixtures_str = fixtures.to_str().unwrap();
            insta::with_settings!({
                filters => vec![(fixtures_str, "[FIXTURES]")],
            }, {
                insta::assert_snapshot!(stdout);
            });
        }
    };
}

// python/simple × formats
snapshot_format!(snap_python_simple_tree, "python/simple", "tree");
snapshot_format!(snap_python_simple_json, "python/simple", "json");
snapshot_format!(snap_python_simple_dot, "python/simple", "dot");
snapshot_format!(snap_python_simple_mermaid, "python/simple", "mermaid");
snapshot_format!(snap_python_simple_table, "python/simple", "table");

// typescript/simple × formats
snapshot_format!(snap_ts_simple_tree, "typescript/simple", "tree");
snapshot_format!(snap_ts_simple_json, "typescript/simple", "json");
snapshot_format!(snap_ts_simple_dot, "typescript/simple", "dot");
snapshot_format!(snap_ts_simple_mermaid, "typescript/simple", "mermaid");
snapshot_format!(snap_ts_simple_table, "typescript/simple", "table");

// javascript/simple × formats
snapshot_format!(snap_js_simple_tree, "javascript/simple", "tree");
snapshot_format!(snap_js_simple_json, "javascript/simple", "json");
snapshot_format!(snap_js_simple_dot, "javascript/simple", "dot");
snapshot_format!(snap_js_simple_mermaid, "javascript/simple", "mermaid");
snapshot_format!(snap_js_simple_table, "javascript/simple", "table");

// javascript/mixed × formats
snapshot_format!(snap_js_mixed_tree, "javascript/mixed", "tree");
snapshot_format!(snap_js_mixed_json, "javascript/mixed", "json");
snapshot_format!(snap_js_mixed_dot, "javascript/mixed", "dot");
snapshot_format!(snap_js_mixed_mermaid, "javascript/mixed", "mermaid");
snapshot_format!(snap_js_mixed_table, "javascript/mixed", "table");

// typescript/cycle × mermaid — pins deterministic node IDs and sorted
// cycle style-line ordering (the old HashSet iteration shuffled them).
snapshot_format!(snap_ts_cycle_mermaid, "typescript/cycle", "mermaid");

// cycle check stderr snapshots
#[test]
fn snap_python_cycle_check() {
    let stderr = kgr_check_stderr("python/cycle");
    let fixtures = fixtures_dir();
    let fixtures_str = fixtures.to_str().unwrap();
    insta::with_settings!({
        filters => vec![(fixtures_str, "[FIXTURES]")],
    }, {
        insta::assert_snapshot!(stderr);
    });
}

#[test]
fn snap_ts_cycle_check() {
    let stderr = kgr_check_stderr("typescript/cycle");
    let fixtures = fixtures_dir();
    let fixtures_str = fixtures.to_str().unwrap();
    insta::with_settings!({
        filters => vec![(fixtures_str, "[FIXTURES]")],
    }, {
        insta::assert_snapshot!(stderr);
    });
}

// Rust local-module resolution: pins the regression where a crate's own
// modules (used via `mod foo; use foo::Bar;` or `use crate::foo::Bar;`) were
// misclassified as external packages. external_deps must list ONLY real crates
// (serde, std::*), and local modules must produce graph edges. Deterministic,
// in-repo — no external dependency.
#[test]
fn snap_rust_local_modules_tree() {
    let stdout = kgr_output("rust/local_modules", "tree");
    insta::assert_snapshot!(stdout);
}

snapshot_format!(snap_rust_local_modules_json, "rust/local_modules", "json");

#[test]
fn snap_rust_local_modules_orient_json() {
    let stdout = kgr_orient_json_path(&fixtures_dir().join("rust/local_modules"));
    insta::assert_snapshot!(stdout);
}
