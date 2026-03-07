use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures")
}

fn kgr_output(fixture: &str, format: &str) -> String {
    let fixture_path = fixtures_dir().join(fixture);
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
        .args(["graph", "--format", format, "--no-progress"])
        .arg(&fixture_path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).unwrap()
}

fn kgr_check_stderr(fixture: &str) -> String {
    let fixture_path = fixtures_dir().join(fixture);
    let output = assert_cmd::cargo::cargo_bin_cmd!("kgr")
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
snapshot_format!(snap_python_simple_dot, "python/simple", "dot");
snapshot_format!(snap_python_simple_mermaid, "python/simple", "mermaid");
snapshot_format!(snap_python_simple_table, "python/simple", "table");

// typescript/simple × formats
snapshot_format!(snap_ts_simple_tree, "typescript/simple", "tree");
snapshot_format!(snap_ts_simple_dot, "typescript/simple", "dot");
snapshot_format!(snap_ts_simple_mermaid, "typescript/simple", "mermaid");
snapshot_format!(snap_ts_simple_table, "typescript/simple", "table");

// javascript/simple × formats
snapshot_format!(snap_js_simple_tree, "javascript/simple", "tree");
snapshot_format!(snap_js_simple_dot, "javascript/simple", "dot");
snapshot_format!(snap_js_simple_mermaid, "javascript/simple", "mermaid");
snapshot_format!(snap_js_simple_table, "javascript/simple", "table");

// javascript/mixed × formats
snapshot_format!(snap_js_mixed_tree, "javascript/mixed", "tree");
snapshot_format!(snap_js_mixed_dot, "javascript/mixed", "dot");
snapshot_format!(snap_js_mixed_mermaid, "javascript/mixed", "mermaid");
snapshot_format!(snap_js_mixed_table, "javascript/mixed", "table");

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
