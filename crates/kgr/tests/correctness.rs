//! Black-box correctness oracle (itr #27).
//!
//! Every test asserts what a *correct* tool reports for a small hand-written
//! fixture with a known ground truth, never what kgr happens to print today.
//! Test names are `<check id>_<what is true>`; the ids come from the original
//! probe battery (PY-10, RS-05, GO-06, ...).
//!
//! Checks that fail today carry an ignore attribute whose reason names the itr
//! issue tracking the root cause. `just known-bugs` runs exactly those
//! (`cargo test --test correctness -- --ignored`) and is expected to fail until
//! the bugs are fixed; when one starts passing, drop its ignore attribute.
//!
//! Fixtures: directory cases live in `tests/fixtures/oracle/<case>/` and are
//! copied into a tempdir per test (commands may write caches/baselines, and the
//! walker must not see the repo's own `.gitignore`). Byte-sensitive inputs
//! (CRLF, UTF-8 multibyte, tabs, very long files) are generated at test time so
//! git line-ending normalisation can never alter them.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Output;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A fixture materialised in a private tempdir.
struct Fx {
    _tmp: TempDir,
    root: PathBuf,
}

impl Fx {
    fn arg(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let p = self.path(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, bytes).unwrap();
    }

    fn remove(&self, rel: &str) {
        std::fs::remove_file(self.path(rel)).unwrap();
    }
}

fn oracle_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures/oracle")
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), to).unwrap();
        }
    }
}

/// Copy `tests/fixtures/oracle/<name>` into a fresh tempdir.
fn case(name: &str) -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(name);
    copy_dir(&oracle_dir().join(name), &root);
    Fx { _tmp: tmp, root }
}

/// An empty tempdir to be filled with generated, byte-exact files.
fn generated() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("gen");
    std::fs::create_dir_all(&root).unwrap();
    Fx { _tmp: tmp, root }
}

/// `kgr` insulated from host `KGR_*` config, with the parse cache disabled.
fn kgr() -> assert_cmd::Command {
    let mut cmd = kgr_without_host_kgr_env();
    cmd.env("KGR_NO_CACHE", "1");
    cmd
}

fn kgr_without_host_kgr_env() -> assert_cmd::Command {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("kgr");
    for key in std::env::vars_os()
        .map(|(key, _)| key)
        .filter(|key| key.to_string_lossy().starts_with("KGR_"))
    {
        cmd.env_remove(key);
    }
    cmd.env("NO_COLOR", "1");
    cmd
}

fn run(args: &[&str]) -> Output {
    kgr().args(args).output().unwrap()
}

fn run_in(dir: &Path, args: &[&str]) -> Output {
    kgr().current_dir(dir).args(args).output().unwrap()
}

fn code(out: &Output) -> Option<i32> {
    out.status.code()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json_of(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {}\nstderr: {}",
            text(&out.stdout),
            text(&out.stderr)
        )
    })
}

/// Forward-slash normalisation so assertions hold on Windows CI.
fn norm(s: &str) -> String {
    s.replace('\\', "/")
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap_or_else(|| panic!("expected array, got {v}"))
        .iter()
        .map(|x| norm(x.as_str().unwrap()))
        .collect()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn svec(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

// ---- graph JSON accessors -------------------------------------------------

fn graph(fx: &Fx) -> Value {
    json_of(&run(&["graph", "-f", "json", "--no-progress", &fx.arg()]))
}

fn file_entry<'a>(d: &'a Value, path: &str) -> Option<&'a Value> {
    d["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| norm(f["path"].as_str().unwrap()) == path)
}

fn edges(d: &Value) -> Vec<(String, String)> {
    let mut e: Vec<(String, String)> = d["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                norm(e["from"].as_str().unwrap()),
                norm(e["to"].as_str().unwrap()),
            )
        })
        .collect();
    e.sort();
    e
}

fn has_edge(d: &Value, a: &str, b: &str) -> bool {
    edges(d).contains(&(a.to_string(), b.to_string()))
}

/// `(kind, resolved)` of the first import in `file` whose raw text is `raw`.
fn imp(d: &Value, file: &str, raw: &str) -> Option<(String, Option<String>)> {
    file_entry(d, file)?["imports"]
        .as_array()?
        .iter()
        .find(|i| i["raw"] == raw)
        .map(|i| {
            (
                i["kind"].as_str().unwrap().to_string(),
                i["resolved"].as_str().map(norm),
            )
        })
}

fn imp_local(path: &str) -> (String, Option<String>) {
    ("local".to_string(), Some(path.to_string()))
}

fn symbols(d: &Value, file: &str) -> Vec<Value> {
    file_entry(d, file)
        .map(|f| f["symbols"].as_array().cloned().unwrap_or_default())
        .unwrap_or_default()
}

fn sym(d: &Value, file: &str, name: &str) -> Vec<Value> {
    symbols(d, file)
        .into_iter()
        .filter(|s| s["name"] == name)
        .collect()
}

fn sym_names(d: &Value, file: &str) -> Vec<String> {
    symbols(d, file)
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_string())
        .collect()
}

fn span_lines(s: &Value) -> (u64, u64) {
    (
        s["span"]["start_line"].as_u64().unwrap(),
        s["span"]["end_line"].as_u64().unwrap(),
    )
}

fn calls(d: &Value, file: &str) -> Vec<String> {
    file_entry(d, file)
        .map(|f| {
            f["calls"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["callee_raw"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn cycles_sorted(d: &Value) -> Vec<Vec<String>> {
    d["cycles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| sorted(strs(c)))
        .collect()
}

fn external_deps(d: &Value, file: &str) -> Vec<String> {
    d["external_deps"].get(file).map(strs).unwrap_or_default()
}

fn contains_all(have: &[String], want: &[&str]) -> bool {
    want.iter().all(|w| have.iter().any(|h| h == w))
}

// ---- symbol-level commands ------------------------------------------------

fn refs_json(fx: &Fx, name: &str, defined_in: Option<&str>) -> Value {
    let root = fx.arg();
    let mut args = vec!["refs", name, &root, "-f", "json", "--no-progress"];
    if let Some(p) = defined_in {
        args.extend(["--defined-in", p]);
    }
    json_of(&run(&args))
}

/// Sorted `file:line` of every reference (duplicates preserved).
fn ref_sites(fx: &Fx, name: &str, defined_in: Option<&str>) -> Vec<String> {
    let d = refs_json(fx, name, defined_in);
    sorted(
        d["references"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| format!("{}:{}", norm(r["file"].as_str().unwrap()), r["line"]))
            .collect(),
    )
}

fn dead_json(fx: &Fx, name: &str, defined_in: Option<&str>) -> Value {
    let root = fx.arg();
    let mut args = vec!["dead", name, &root, "-f", "json", "--no-progress"];
    if let Some(p) = defined_in {
        args.extend(["--defined-in", p]);
    }
    json_of(&run(&args))
}

fn impact_json(fx: &Fx, name: &str) -> Value {
    json_of(&run(&[
        "impact",
        name,
        &fx.arg(),
        "-f",
        "json",
        "--no-progress",
    ]))
}

/// `(depth, calls_symbol)` for the impact entry of `file`.
fn impact_entry(d: &Value, file: &str) -> Option<(u64, bool)> {
    d["impact"]
        .as_array()?
        .iter()
        .find(|i| norm(i["file"].as_str().unwrap()) == file)
        .map(|i| {
            (
                i["depth"].as_u64().unwrap(),
                i["calls_symbol"].as_bool().unwrap(),
            )
        })
}

fn skeleton(path: &Path) -> String {
    let out = run(&["skeleton", &path.to_string_lossy(), "--no-progress"]);
    text(&out.stdout)
}

fn query(fx: &Fx, args: &[&str]) -> Value {
    let root = fx.arg();
    let mut a = vec!["query"];
    a.extend_from_slice(args);
    a.extend(["--no-progress", "-f", "json"]);
    a.push(&root);
    json_of(&run(&a))
}

fn check_cmd(fx: &Fx, extra: &[&str]) -> Output {
    let root = fx.arg();
    let mut a = vec!["check", root.as_str(), "--no-progress"];
    a.extend_from_slice(extra);
    run(&a)
}

/// Fixture for show/slice: CRLF + tabs + multibyte + a 15k-line file, as raw bytes.
fn show_fixture() -> Fx {
    let fx = generated();
    fx.write(
        "crlf.py",
        "def alpha():\r\n    return \"\u{e9}t\u{e9} \u{1f680}\"\r\n\r\n\r\ndef beta(x):\r\n\tif x:\r\n\t\treturn 1\r\n\treturn 2\r\n"
            .as_bytes(),
    );
    fx.write(
        "uni.py",
        "# \u{65e5}\u{672c}\u{8a9e} comment\ndef gamma():\n    s = \"\u{2014}\"\n    return s\n"
            .as_bytes(),
    );
    let mut long = String::new();
    for i in 0..5000 {
        long += "def f";
        long += &i.to_string();
        long += "(x):
    return x + ";
        long += &i.to_string();
        long += "

";
    }
    long.push_str("def last():\n    return 'end'\n");
    fx.write("long.py", long.as_bytes());
    fx
}

fn show_json(fx: &Fx, name: &str) -> Value {
    json_of(&run(&[
        "show",
        name,
        &fx.arg(),
        "-f",
        "json",
        "--no-progress",
    ]))
}

/// The `py` fixture; check/baseline/query tests mutate their private copy.
fn py_workspace() -> Fx {
    case("py")
}

const JM: &str = "src/main/java/com/acme";

// ---------------------------------------------------------------------------
// Guard: every ignore attribute must name the itr issue that tracks it.
// ---------------------------------------------------------------------------

#[test]
fn ignored_tests_all_cite_an_itr_issue() {
    let src = include_str!("correctness.rs");
    let any_ignore = ["#[ignore", ""].concat();
    let with_reason = ["#[ignore", " = \""].concat();
    let bare = ["#[ignore", "]"].concat();
    let mut seen = 0;
    for (n, line) in src.lines().enumerate() {
        let t = line.trim_start();
        if !t.starts_with(&any_ignore) {
            continue;
        }
        assert!(
            !t.starts_with(&bare),
            "line {}: bare ignore without a reason",
            n + 1
        );
        let rest = t
            .strip_prefix(&with_reason)
            .unwrap_or_else(|| panic!("line {}: malformed ignore attribute: {t}", n + 1));
        let after_itr = rest
            .strip_prefix("itr-")
            .unwrap_or_else(|| panic!("line {}: reason must start with itr-<N>: {t}", n + 1));
        let digits: String = after_itr
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        assert!(
            !digits.is_empty() && after_itr[digits.len()..].starts_with(": "),
            "line {}: reason must look like `itr-<N>: <why>`: {t}",
            n + 1
        );
        seen += 1;
    }
    assert!(
        seen > 0,
        "guard found no ignore attributes; its parser is broken"
    );
}

// ---------------------------------------------------------------------------
// Python
// ---------------------------------------------------------------------------

#[test]
fn py01_from_package_import_links_package_init() {
    let d = graph(&case("py"));
    assert!(has_edge(&d, "app.py", "pkg/__init__.py"));
}

#[test]
fn py02_relative_import_in_init_links_core() {
    let d = graph(&case("py"));
    assert!(has_edge(&d, "pkg/__init__.py", "pkg/core.py"));
}

#[test]
fn py03_aliased_from_import_links_util() {
    let d = graph(&case("py"));
    assert!(has_edge(&d, "app.py", "pkg/util.py"));
}

#[test]
fn py04_two_file_cycle_is_detected() {
    let d = graph(&case("py"));
    assert_eq!(cycles_sorted(&d), vec![svec(&["cyc_a.py", "cyc_b.py"])]);
}

#[test]
fn py05_only_orphan_py_is_an_orphan() {
    let d = graph(&case("py"));
    assert_eq!(strs(&d["orphans"]), svec(&["orphan.py"]));
}

#[test]
fn py06_requests_is_an_external_dep_of_app() {
    let d = graph(&case("py"));
    assert!(external_deps(&d, "app.py").contains(&"requests".to_string()));
}

#[test]
fn py07_nested_class_and_decorated_methods_are_all_captured() {
    let d = graph(&case("py"));
    assert_eq!(
        sorted(sym_names(&d, "pkg/core.py")),
        svec(&["Engine", "Inner", "build", "fact", "helper", "inner_m", "run", "step"])
    );
}

#[test]
fn py08_class_span_is_lines_4_to_18() {
    let d = graph(&case("py"));
    assert_eq!(span_lines(&sym(&d, "pkg/core.py", "Engine")[0]), (4, 18));
}

#[test]
fn py09_decorated_method_span_includes_decorator() {
    let d = graph(&case("py"));
    assert_eq!(span_lines(&sym(&d, "pkg/core.py", "run")[0]), (13, 15));
}

// B19 was refuted: classifying stdlib as external rather than system is a
// defensible design choice. What must hold is that stdlib is never "local".
#[test]
fn py10_stdlib_os_is_not_a_local_import() {
    let d = graph(&case("py"));
    let (kind, resolved) = imp(&d, "app.py", "os").unwrap();
    assert_ne!(kind, "local");
    assert_eq!(resolved, None);
}

#[test]
fn py11_method_call_is_attributed_as_reference() {
    assert_eq!(ref_sites(&case("py"), "run", None), svec(&["app.py:9"]));
}

#[test]
fn py12_calls_in_comments_and_strings_are_not_refs() {
    assert!(ref_sites(&case("py"), "never_called", None).is_empty());
}

#[test]
fn py13_call_text_inside_a_string_is_not_a_ref() {
    assert!(ref_sites(&case("py"), "only_in_string", None).is_empty());
}

#[test]
fn py14_recursion_is_recorded() {
    assert_eq!(
        ref_sites(&case("py"), "fact", None),
        svec(&["pkg/core.py:26"])
    );
}

#[test]
fn py15_unused_function_is_dead() {
    assert_eq!(dead_json(&case("py"), "never_called", None)["dead"], true);
}

#[test]
fn py16_unknown_symbol_is_not_found_and_dead_is_null() {
    let d = dead_json(&case("py"), "no_such_symbol", None);
    assert_eq!(d["found"], false);
    assert!(d["dead"].is_null());
}

#[test]
#[ignore = "itr-6: calls through `import ... as h` aliases are not matched, so dead says true"]
fn py17_dead_aliased_import_is_live() {
    let d = dead_json(&case("py"), "helper", Some("pkg/util.py"));
    assert_eq!(d["dead"], false);
}

#[test]
#[ignore = "itr-7: dead returns true when the only callers are live same-file code (self_only_references)"]
fn py18_method_called_by_live_same_file_method_is_not_dead() {
    assert_eq!(dead_json(&case("py"), "step", None)["dead"], false);
}

#[test]
#[ignore = "itr-6: impact does not see the aliased call `h()` (calls_symbol false)"]
fn py19_impact_marks_aliased_caller_as_calling_symbol() {
    let d = impact_json(&case("py"), "helper");
    assert_eq!(impact_entry(&d, "app.py").map(|e| e.1), Some(true));
}

#[test]
fn py20_impact_engine_reaches_app_at_depth_2_with_call() {
    let d = impact_json(&case("py"), "Engine");
    assert_eq!(impact_entry(&d, "app.py"), Some((2, true)));
}

#[test]
#[ignore = "itr-23: graph JSON always serializes symbols and calls; --symbols is a no-op shape switch"]
fn py21_graph_json_omits_symbols_unless_requested() {
    let d = graph(&case("py"));
    assert!(d["files"][0].get("symbols").is_none());
}

#[test]
fn py22_graph_json_root_has_no_extended_length_prefix() {
    let d = graph(&case("py"));
    assert!(!d["root"].as_str().unwrap().starts_with(r"\\?"));
}

// ---- same-name conflation (py2) ----

// B06 was refuted: refusing to attribute an ambiguous name until the caller
// picks a definition is defensible, provided the output says how to do that.
#[test]
fn py2_01_ambiguous_refs_lists_definitions_and_asks_for_defined_in() {
    let d = refs_json(&case("py2"), "helper", None);
    assert_eq!(d["ambiguous"], true);
    assert_eq!(d["definitions"].as_array().unwrap().len(), 2);
    assert!(d["scope_hint"].as_str().unwrap().contains("--defined-in"));
}

#[test]
#[ignore = "itr-8: --defined-in scope credits every same-named call in the importer closure"]
fn py2_02_refs_defined_in_b_lists_only_b_helper_call() {
    assert_eq!(
        ref_sites(&case("py2"), "helper", Some("b.py")),
        svec(&["c.py:7"])
    );
}

#[test]
#[ignore = "itr-8: --defined-in a.py still includes b.helper() at c.py:7"]
fn py2_03_refs_defined_in_a_excludes_b_helper_call() {
    let sites = ref_sites(&case("py2"), "helper", Some("a.py"));
    assert!(!sites.contains(&"c.py:7".to_string()));
}

#[test]
#[ignore = "itr-8: --defined-in a.py still includes the call of a shadowing local lambda (c.py:13)"]
fn py2_04_refs_defined_in_a_excludes_shadowed_lambda_call() {
    let sites = ref_sites(&case("py2"), "helper", Some("a.py"));
    assert!(!sites.contains(&"c.py:13".to_string()));
}

#[test]
fn py2_05_defined_in_is_echoed_with_forward_slashes() {
    let d = refs_json(&case("py2"), "helper", Some("./a.py"));
    assert_eq!(d["defined_in"], "a.py");
}

// ---- src layout / relative / fn-as-value (py3) ----

#[test]
fn py3_01_from_dotdot_import_is_recorded() {
    let d = graph(&case("py3"));
    let f = file_entry(&d, "src/proj/sub/leaf.py").unwrap();
    let on_line_1 = f["imports"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["span"]["start_line"] == 1);
    assert!(on_line_1);
}

#[test]
fn py3_02_relative_dotdot_base_resolves_to_base_py() {
    let d = graph(&case("py3"));
    assert!(has_edge(&d, "src/proj/sub/leaf.py", "src/proj/base.py"));
}

#[test]
#[ignore = "itr-14: src-layout absolute import `import proj.base` is classified external"]
fn py3_03_src_layout_absolute_import_is_local() {
    let d = graph(&case("py3"));
    assert_eq!(
        imp(&d, "src/proj/sub/leaf.py", "proj.base").map(|i| i.0),
        Some("local".to_string())
    );
}

#[test]
#[ignore = "itr-14: src-layout `from proj.sub import leaf` does not resolve, leaving cli.py an orphan"]
fn py3_04_src_layout_importer_is_not_an_orphan() {
    let d = graph(&case("py3"));
    assert!(!strs(&d["orphans"]).contains(&"src/proj/cli.py".to_string()));
}

#[test]
#[ignore = "itr-6: a function passed as a value (map(callback, ..)) is not counted as a reference"]
fn py3_05_function_passed_as_value_is_not_dead() {
    assert_eq!(dead_json(&case("py3"), "callback", None)["dead"], false);
}

// ---------------------------------------------------------------------------
// TypeScript / JavaScript
// ---------------------------------------------------------------------------

#[test]
fn ts01_tsconfig_wildcard_path_alias_resolves() {
    let d = graph(&case("ts"));
    assert_eq!(
        imp(&d, "src/main.ts", "@lib/math"),
        Some(imp_local("src/lib/math.ts"))
    );
}

#[test]
fn ts02_tsconfig_exact_path_alias_resolves_to_index() {
    let d = graph(&case("ts"));
    assert_eq!(
        imp(&d, "src/main.ts", "@util"),
        Some(imp_local("src/util/index.ts"))
    );
}

#[test]
fn ts03_barrel_export_star_links_strings() {
    let d = graph(&case("ts"));
    assert!(has_edge(&d, "src/util/index.ts", "src/util/strings.ts"));
}

#[test]
fn ts04_js_require_links_ts_module() {
    let d = graph(&case("ts"));
    assert!(has_edge(&d, "src/legacy.js", "src/util/strings.ts"));
}

#[test]
fn ts05_dynamic_import_creates_an_edge() {
    let d = graph(&case("ts"));
    assert!(has_edge(&d, "src/lazy.ts", "src/lib/math.ts"));
}

#[test]
fn ts06_a_b_cycle_is_detected() {
    let d = graph(&case("ts"));
    assert_eq!(cycles_sorted(&d), vec![svec(&["src/a.ts", "src/b.ts"])]);
}

#[test]
fn ts07_express_and_node_fs_are_external_deps() {
    let d = graph(&case("ts"));
    assert!(contains_all(
        &external_deps(&d, "src/main.ts"),
        &["express", "node:fs"]
    ));
}

#[test]
fn ts08_arrow_const_class_and_methods_are_captured() {
    let d = graph(&case("ts"));
    assert!(contains_all(
        &sym_names(&d, "src/lib/math.ts"),
        &["mul", "Calc", "compute", "create", "internal"]
    ));
}

#[test]
#[ignore = "itr-19: TS overload signatures are not indexed, so the symbol extent starts at line 3"]
fn ts09_overloaded_function_extent_starts_at_first_overload() {
    let d = graph(&case("ts"));
    let min = sym(&d, "src/lib/math.ts", "add")
        .iter()
        .map(|s| span_lines(s).0)
        .min();
    assert_eq!(min, Some(1));
}

#[test]
#[ignore = "itr-6: aliased call `plus(1, 2)` (import { add as plus }) is not matched to add"]
fn ts10_refs_defined_in_math_includes_aliased_call() {
    let sites = ref_sites(&case("ts"), "add", Some("src/lib/math.ts"));
    assert!(sites.contains(&"src/main.ts:9".to_string()));
}

#[test]
fn ts11_refs_defined_in_strings_has_no_refs_for_other_add() {
    assert!(ref_sites(&case("ts"), "add", Some("src/util/strings.ts")).is_empty());
}

#[test]
fn ts12_commented_call_is_ignored_and_member_call_counted() {
    assert_eq!(
        ref_sites(&case("ts"), "mul", None),
        svec(&["src/lazy.ts:3"])
    );
}

#[test]
fn ts13_call_text_inside_a_string_is_not_a_ref() {
    assert!(ref_sites(&case("ts"), "internal", None).is_empty());
}

#[test]
fn ts14_unreferenced_internal_function_is_dead() {
    assert_eq!(dead_json(&case("ts"), "internal", None)["dead"], true);
}

#[test]
#[ignore = "itr-18: skeleton leaks arrow-const bodies (`=> a * b`)"]
fn ts15_skeleton_elides_arrow_const_body() {
    let fx = case("ts");
    let sk = skeleton(&fx.path("src/lib/math.ts"));
    assert!(!sk.contains("=> a * b"), "{sk}");
}

// ---- ts2: JSONC tsconfig, directory imports, aliased-only use ----

#[test]
fn ts2_01_directory_import_resolves_to_index() {
    let d = graph(&case("ts2"));
    assert!(has_edge(&d, "src/feat/a.ts", "src/util/index.ts"));
}

#[test]
fn ts2_02_nodenext_js_specifier_resolves_to_ts_source() {
    let d = graph(&case("ts2"));
    assert!(has_edge(&d, "src/feat/a.ts", "src/feat/b.ts"));
}

#[test]
#[ignore = "itr-15: tsconfig.json is parsed as strict JSON, so JSONC comments and trailing commas drop the paths"]
fn ts2_03_jsonc_tsconfig_paths_are_honoured() {
    let d = graph(&case("ts2"));
    assert_eq!(
        imp(&d, "src/feat/a.ts", "~/util"),
        Some(imp_local("src/util/index.ts"))
    );
}

#[test]
fn ts2_04_calls_inside_template_literals_are_captured() {
    let d = graph(&case("ts2"));
    assert!(contains_all(&calls(&d, "src/feat/a.ts"), &["tool", "bee"]));
}

#[test]
#[ignore = "itr-6: function used only via `import { x as y }` is reported dead"]
fn ts2_05_function_used_only_through_alias_is_not_dead() {
    assert_eq!(dead_json(&case("ts2"), "onlyAliased", None)["dead"], false);
}

// ---------------------------------------------------------------------------
// Rust
// ---------------------------------------------------------------------------

#[test]
fn rs01_pub_mod_links_mod_rs() {
    let d = graph(&case("rs"));
    assert!(has_edge(&d, "src/lib.rs", "src/net/mod.rs"));
}

#[test]
fn rs02_use_crate_path_links_client_rs() {
    let d = graph(&case("rs"));
    assert!(has_edge(&d, "src/util.rs", "src/net/client.rs"));
}

#[test]
fn rs03_use_super_super_closes_a_cycle() {
    let d = graph(&case("rs"));
    assert_eq!(
        cycles_sorted(&d),
        vec![svec(&["src/net/client.rs", "src/util.rs"])]
    );
}

#[test]
fn rs04_cargo_targets_are_structural_not_orphans() {
    let d = graph(&case("rs"));
    let paths = sorted(
        d["structural_entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| norm(s["path"].as_str().unwrap()))
            .collect(),
    );
    assert_eq!(
        paths,
        svec(&["examples/ex.rs", "src/main.rs", "tests/it.rs"])
    );
    assert!(strs(&d["orphans"]).is_empty());
}

#[test]
#[ignore = "itr-12: `use demo::...` (own package name) is classified external, so no main.rs -> lib.rs edge"]
fn rs05_use_of_own_crate_name_links_main_to_lib() {
    let d = graph(&case("rs"));
    assert!(has_edge(&d, "src/main.rs", "src/lib.rs"));
}

#[test]
#[ignore = "itr-12: the package's own crate name is reported as an external package"]
fn rs06_own_crate_is_not_an_external_package() {
    let d = graph(&case("rs"));
    assert!(!external_deps(&d, "src/main.rs")
        .iter()
        .any(|x| x.starts_with("demo")));
}

#[test]
#[ignore = "itr-26: external_deps holds item paths (serde::Serialize) instead of package names"]
fn rs07_external_deps_lists_package_names_not_item_paths() {
    let d = graph(&case("rs"));
    assert_eq!(external_deps(&d, "src/lib.rs"), svec(&["serde"]));
}

// B19 was refuted: std may legitimately be reported as external. What must
// hold is that it is never mistaken for a first-party (local) import.
#[test]
fn rs08_std_import_is_not_a_local_import() {
    let d = graph(&case("rs"));
    let (kind, resolved) = imp(&d, "src/main.rs", "std::collections::HashMap").unwrap();
    assert_ne!(kind, "local");
    assert_eq!(resolved, None);
}

#[test]
#[ignore = "itr-10: type names, generics and attributes are recorded as calls"]
fn rs09_generic_param_and_attributes_are_not_calls() {
    let d = graph(&case("rs"));
    let c = calls(&d, "src/lib.rs");
    assert!(!c.contains(&"T".to_string()) && !c.contains(&"derive".to_string()));
}

#[test]
#[ignore = "itr-19: trait default method is kind=function while required trait methods are method"]
fn rs10_trait_default_method_kind_is_method() {
    let d = graph(&case("rs"));
    assert_eq!(sym(&d, "src/lib.rs", "name")[0]["kind"], "method");
}

#[test]
#[ignore = "itr-6: calls inside macro token trees (println!, assert_eq!) are invisible, so total() looks dead"]
fn rs11_function_used_in_println_and_tests_is_not_dead() {
    assert_eq!(dead_json(&case("rs"), "total", None)["dead"], false);
}

#[test]
#[ignore = "itr-8: name-only scoping attributes HashMap::new to Square::new"]
fn rs12_refs_new_is_not_conflated_with_hashmap_new() {
    assert_eq!(
        ref_sites(&case("rs"), "new", None),
        svec(&["src/main.rs:5"])
    );
}

#[test]
#[ignore = "itr-10: a struct definition line is counted as a call reference to itself"]
fn rs13_struct_definition_line_is_not_its_own_reference() {
    let sites = ref_sites(&case("rs"), "Square", None);
    assert!(!sites.contains(&"src/lib.rs:13".to_string()));
}

#[test]
#[ignore = "itr-6: a function referenced as a value (`let _ = helper;`) is reported dead"]
fn rs14_function_referenced_as_value_is_not_dead() {
    assert_eq!(dead_json(&case("rs"), "helper", None)["dead"], false);
}

#[test]
#[ignore = "itr-18: skeleton appends a bogus `{ ... }` to bodiless trait fns"]
fn rs15_skeleton_does_not_brace_bodiless_trait_fn() {
    let fx = case("rs");
    let sk = skeleton(&fx.path("src/lib.rs"));
    assert!(!sk.contains("f64; { ... }"), "{sk}");
}

#[test]
#[ignore = "itr-6: fn called only inside println!() is reported dead"]
fn rsm01_function_called_only_inside_macro_is_not_dead() {
    assert_eq!(dead_json(&case("rsmac"), "in_macro", None)["dead"], false);
}

#[test]
fn rsm02_plain_call_keeps_function_live() {
    assert_eq!(dead_json(&case("rsmac"), "plain", None)["dead"], false);
}

// ---------------------------------------------------------------------------
// Go
// ---------------------------------------------------------------------------

/// The `go` fixture reduced to single-file packages and no test file.
fn go_single_file_pkgs() -> Fx {
    let fx = case("go");
    fx.remove("pkg/util/extra.go");
    fx.remove("internal/store/store_test.go");
    fx
}

#[test]
fn go01_module_path_import_links_store() {
    let d = graph(&case("go"));
    assert!(has_edge(&d, "cmd/app/main.go", "internal/store/store.go"));
}

#[test]
fn go02_aliased_module_import_is_local() {
    let d = graph(&case("go"));
    assert_eq!(
        imp(&d, "internal/store/store.go", "example.com/demo/pkg/util").map(|i| i.0),
        Some("local".to_string())
    );
}

#[test]
fn go03_third_party_import_is_external() {
    let d = graph(&case("go"));
    assert_eq!(
        external_deps(&d, "cmd/app/main.go"),
        svec(&["github.com/pkg/errors"])
    );
}

#[test]
fn go04_test_file_is_a_test_entry() {
    let d = graph(&case("go"));
    assert_eq!(
        strs(&d["test_entries"]),
        svec(&["internal/store/store_test.go"])
    );
}

#[test]
#[ignore = "itr-11: a Go package import resolves to one file only, orphaning its siblings"]
fn go05_file_in_imported_multi_file_package_is_not_an_orphan() {
    let d = graph(&case("go"));
    assert!(!strs(&d["orphans"]).contains(&"pkg/util/util.go".to_string()));
}

#[test]
#[ignore = "itr-11: a Go package import links only the alphabetically first file of the package"]
fn go06_package_import_links_every_file_of_the_package() {
    let d = graph(&case("go"));
    assert!(has_edge(&d, "internal/store/store.go", "pkg/util/util.go"));
}

#[test]
#[ignore = "itr-11: impact of Clean is empty because the package edge points at extra.go"]
fn go07_impact_of_clean_includes_its_dependent_store() {
    let d = impact_json(&case("go"), "Clean");
    assert!(impact_entry(&d, "internal/store/store.go").is_some());
}

#[test]
#[ignore = "itr-8: --defined-in drops same-package callers that need no import edge"]
fn go09_refs_store_new_includes_same_package_test_call() {
    let sites = ref_sites(&case("go"), "New", Some("internal/store/store.go"));
    assert!(sites.contains(&"internal/store/store_test.go:6".to_string()));
}

#[test]
fn go10_method_with_receiver_is_kind_method() {
    let d = graph(&case("go"));
    assert_eq!(
        sym(&d, "internal/store/store.go", "Get")[0]["kind"],
        "method"
    );
}

#[test]
#[ignore = "itr-8: --defined-in credits store.New() and errors.New() to util.New"]
fn go08_refs_util_new_excludes_store_new_and_errors_new() {
    let fx = go_single_file_pkgs();
    assert!(ref_sites(&fx, "New", Some("pkg/util/util.go")).is_empty());
}

// ---------------------------------------------------------------------------
// C / C++
// ---------------------------------------------------------------------------

#[test]
fn c01_relative_include_resolves() {
    let d = graph(&case("c"));
    assert!(has_edge(&d, "src/main.c", "include/geo/point.h"));
}

#[test]
fn c02_include_found_via_include_dir() {
    let d = graph(&case("c"));
    assert!(has_edge(&d, "src/point.c", "include/geo/point.h"));
}

#[test]
fn c03_angle_include_is_system() {
    let d = graph(&case("c"));
    assert_eq!(
        imp(&d, "src/main.c", "stdio.h"),
        Some(("system".to_string(), None))
    );
}

#[test]
fn c04_header_include_cycle_is_detected() {
    let d = graph(&case("c"));
    assert_eq!(
        cycles_sorted(&d),
        vec![svec(&["src/local.h", "src/main_cycle.h"])]
    );
}

#[test]
fn c05_cpp_source_links_its_header() {
    let d = graph(&case("c"));
    assert!(has_edge(&d, "src/shapes.cpp", "src/shapes.hpp"));
}

#[test]
fn c06_call_inside_printf_args_is_a_reference() {
    assert_eq!(
        ref_sites(&case("c"), "point_dist", None),
        svec(&["src/main.c:7"])
    );
}

#[test]
fn c07_static_helper_called_twice_on_one_line_is_two_refs() {
    assert_eq!(
        ref_sites(&case("c"), "sq", None),
        svec(&["src/point.c:7", "src/point.c:7"])
    );
}

#[test]
fn c08_template_fn_and_method_are_captured() {
    let d = graph(&case("c"));
    assert!(contains_all(
        &sym_names(&d, "src/shapes.cpp"),
        &["twice", "area", "Circle", "run"]
    ));
}

#[test]
#[ignore = "itr-19: a C++ namespace is reported as kind=class"]
fn c09_namespace_is_not_reported_as_a_class() {
    let d = graph(&case("c"));
    let bad = sym(&d, "src/shapes.cpp", "geo")
        .iter()
        .any(|s| s["kind"] == "class");
    assert!(!bad);
}

// ---------------------------------------------------------------------------
// Java
// ---------------------------------------------------------------------------

#[test]
fn jv01_class_import_links_repo() {
    let d = graph(&case("java"));
    assert!(has_edge(
        &d,
        &format!("{JM}/app/Main.java"),
        &format!("{JM}/core/Repo.java")
    ));
}

#[test]
#[ignore = "itr-13: `import static com.acme.core.Util.twice` does not resolve to Util.java"]
fn jv02_static_import_links_util() {
    let d = graph(&case("java"));
    assert!(has_edge(
        &d,
        &format!("{JM}/app/Main.java"),
        &format!("{JM}/core/Util.java")
    ));
}

#[test]
#[ignore = "itr-13: a statically imported class is left a false orphan"]
fn jv03_statically_imported_class_is_not_an_orphan() {
    let d = graph(&case("java"));
    assert!(!strs(&d["orphans"]).contains(&format!("{JM}/core/Util.java")));
}

#[test]
#[ignore = "itr-13: first-party static import is listed as an external package"]
fn jv04_first_party_import_is_not_an_external_package() {
    let d = graph(&case("java"));
    let ext = external_deps(&d, &format!("{JM}/app/Main.java"));
    assert!(!ext.iter().any(|x| x.starts_with("com.acme")));
}

#[test]
fn jv05_src_test_file_is_a_test_entry() {
    let d = graph(&case("java"));
    assert_eq!(
        strs(&d["test_entries"]),
        svec(&["src/test/java/com/acme/core/RepoTest.java"])
    );
}

#[test]
fn jv06_overloads_produce_two_definitions() {
    let d = graph(&case("java"));
    assert_eq!(sym(&d, &format!("{JM}/core/Repo.java"), "count").len(), 2);
}

#[test]
fn jv07_refs_defined_in_repo_finds_count_call() {
    let sites = ref_sites(
        &case("java"),
        "count",
        Some(&format!("{JM}/core/Repo.java")),
    );
    assert!(sites.contains(&format!("{JM}/app/Main.java:9")));
}

#[test]
fn jv08_same_package_test_call_is_found() {
    assert_eq!(
        ref_sites(&case("java"), "all", None),
        svec(&["src/test/java/com/acme/core/RepoTest.java:4"])
    );
}

#[test]
fn jv09_private_method_is_not_exported() {
    let d = graph(&case("java"));
    assert_eq!(
        sym(&d, &format!("{JM}/core/Repo.java"), "hidden")[0]["exported"],
        false
    );
}

// ---------------------------------------------------------------------------
// Ruby
// ---------------------------------------------------------------------------

#[test]
fn rb01_require_relative_links_cart() {
    let d = graph(&case("rb"));
    assert!(has_edge(&d, "lib/shop.rb", "lib/shop/cart.rb"));
}

#[test]
fn rb02_require_relative_parent_path_links_shop() {
    let d = graph(&case("rb"));
    assert!(has_edge(&d, "spec/cart_spec.rb", "lib/shop.rb"));
}

#[test]
fn rb03_require_dot_slash_links_shop() {
    let d = graph(&case("rb"));
    assert!(has_edge(&d, "bin_tool.rb", "lib/shop.rb"));
}

#[test]
fn rb04_class_method_call_is_a_reference() {
    assert_eq!(
        ref_sites(&case("rb"), "version", None),
        svec(&["bin_tool.rb:2"])
    );
}

#[test]
#[ignore = "itr-19: Ruby instance methods are indexed as kind=function instead of method"]
fn rb05_instance_method_kind_is_method() {
    let d = graph(&case("rb"));
    assert_eq!(sym(&d, "lib/shop/cart.rb", "add")[0]["kind"], "method");
}

#[test]
#[ignore = "itr-19: Ruby `private` visibility is ignored, methods stay exported=true"]
fn rb06_method_under_private_is_not_exported() {
    let d = graph(&case("rb"));
    assert_eq!(sym(&d, "lib/shop/cart.rb", "secret")[0]["exported"], false);
}

// ---------------------------------------------------------------------------
// show / slice / skeleton: CRLF, unicode, tabs, long files
// ---------------------------------------------------------------------------

#[test]
fn sh01_crlf_file_function_range_is_5_to_8() {
    let d = show_json(&show_fixture(), "beta");
    assert_eq!(
        (d[0]["start_line"].as_u64(), d[0]["end_line"].as_u64()),
        (Some(5), Some(8))
    );
}

#[test]
fn sh02_no_linenos_preserves_crlf_bytes_exactly() {
    let fx = show_fixture();
    let out = run(&["show", "alpha", &fx.arg(), "--no-linenos", "--no-progress"]);
    let want = "def alpha():\r\n    return \"\u{e9}t\u{e9} \u{1f680}\"\r\n".as_bytes();
    assert_eq!(out.stdout, want);
}

#[test]
fn sh03_tabs_are_kept_in_json_body() {
    let d = show_json(&show_fixture(), "beta");
    assert!(d[0]["body"].as_str().unwrap().contains("\t\treturn 1"));
}

#[test]
fn sh04_unicode_body_is_intact() {
    let d = show_json(&show_fixture(), "gamma");
    assert!(d[0]["body"].as_str().unwrap().contains('\u{2014}'));
    assert_eq!(d[0]["start_line"], 2);
}

#[test]
fn sh05_last_function_of_15k_line_file_is_found() {
    let d = show_json(&show_fixture(), "last");
    assert_eq!(
        (d[0]["start_line"].as_u64(), d[0]["end_line"].as_u64()),
        (Some(15001), Some(15002))
    );
}

#[test]
fn sh06_show_unknown_symbol_exits_1() {
    let fx = show_fixture();
    let out = run(&["show", "nosuch", &fx.arg(), "--no-progress"]);
    assert_eq!(code(&out), Some(1));
}

#[test]
#[ignore = "itr-26: show -k mismatch answers 'Did you mean: beta?' with the very name it was given"]
fn sh07_show_kind_mismatch_does_not_suggest_the_same_name() {
    let fx = show_fixture();
    let out = run(&["show", "beta", &fx.arg(), "-k", "class", "--no-progress"]);
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(!all.contains("Did you mean: beta?"), "{all}");
}

#[test]
fn sl01_slice_json_window_has_lines_without_crlf() {
    let fx = show_fixture();
    let out = run_in(&fx.root, &["slice", "crlf.py:5-6", "-f", "json"]);
    let d = json_of(&out);
    assert_eq!(d["start_line"], 5);
    assert_eq!(d["end_line"], 6);
    assert_eq!(d["lines"], serde_json::json!(["def beta(x):", "\tif x:"]));
}

#[test]
fn sl02_slice_end_past_eof_clamps_and_exits_0() {
    let fx = show_fixture();
    assert_eq!(
        code(&run_in(&fx.root, &["slice", "crlf.py:7-999"])),
        Some(0)
    );
}

#[test]
fn sl03_slice_missing_file_exits_2() {
    let fx = show_fixture();
    assert_eq!(code(&run_in(&fx.root, &["slice", "nope.py:1"])), Some(2));
}

#[test]
fn sl04_slice_start_after_end_exits_2() {
    let fx = show_fixture();
    assert_eq!(code(&run_in(&fx.root, &["slice", "crlf.py:6-2"])), Some(2));
}

#[cfg(windows)]
#[test]
fn sl05_slice_windows_drive_colon_path_is_parsed() {
    let fx = show_fixture();
    let arg = format!("{}:2-3", fx.path("uni.py").display());
    let out = run(&["slice", &arg, "--no-linenos"]);
    assert_eq!(
        text(&out.stdout).lines().count(),
        2,
        "{}",
        text(&out.stderr)
    );
}

#[test]
#[ignore = "itr-26: slice with a line far past EOF still prints the last line"]
fn sl06_slice_line_past_eof_prints_no_source_lines() {
    let fx = show_fixture();
    let out = run_in(&fx.root, &["slice", "crlf.py:100"]);
    assert_eq!(text(&out.stdout).matches("return").count(), 0);
}

#[test]
fn sl07_slice_caps_output_at_500_lines_by_default() {
    let fx = show_fixture();
    let out = run_in(&fx.root, &["slice", "long.py:1-2000"]);
    assert_eq!(text(&out.stdout).lines().count(), 500);
}

#[test]
#[ignore = "itr-18: python skeleton prints the decorator line instead of the def signature"]
fn sk01_py_skeleton_decorated_method_shows_def_line() {
    let fx = case("py");
    assert!(skeleton(&fx.path("pkg/core.py")).contains("def run(self)"));
}

#[test]
#[ignore = "itr-18: python skeleton renders decorators as `@staticmethod { ... }`"]
fn sk02_py_skeleton_has_no_c_style_braces_on_decorators() {
    let fx = case("py");
    assert!(!skeleton(&fx.path("pkg/core.py")).contains("@staticmethod {"));
}

// ---------------------------------------------------------------------------
// check / baseline / rules / query
// ---------------------------------------------------------------------------

#[test]
fn ck01_check_with_a_cycle_exits_1() {
    assert_eq!(code(&check_cmd(&py_workspace(), &[])), Some(1));
}

#[test]
fn ck02_check_exit_zero_exits_0() {
    assert_eq!(code(&check_cmd(&py_workspace(), &["--exit-zero"])), Some(0));
}

#[test]
fn ck03_check_bad_path_exits_2() {
    let fx = py_workspace();
    let bad = fx.path("nope");
    let out = run(&["check", &bad.to_string_lossy(), "--no-progress"]);
    assert_eq!(code(&out), Some(2));
}

#[test]
fn ck04_check_bad_format_exits_2() {
    assert_eq!(code(&check_cmd(&py_workspace(), &["-f", "yaml"])), Some(2));
}

#[test]
fn ck05_update_baseline_exits_0() {
    assert_eq!(
        code(&check_cmd(&py_workspace(), &["--update-baseline"])),
        Some(0)
    );
}

#[test]
fn ck06_baseline_suppresses_known_cycle() {
    let fx = py_workspace();
    check_cmd(&fx, &["--update-baseline"]);
    let d = json_of(&check_cmd(&fx, &["-f", "json"]));
    assert_eq!(d["ok"], true);
    assert_eq!(d["suppressed"], 1);
}

#[test]
fn ck07_baselined_run_exits_0() {
    let fx = py_workspace();
    check_cmd(&fx, &["--update-baseline"]);
    assert_eq!(code(&check_cmd(&fx, &[])), Some(0));
}

#[test]
fn ck08_new_cycle_beyond_baseline_exits_1() {
    let fx = py_workspace();
    check_cmd(&fx, &["--update-baseline"]);
    fx.write("cyc_c.py", b"import cyc_a\n");
    fx.write("cyc_a.py", b"import cyc_b\nimport cyc_c\n");
    assert_eq!(code(&check_cmd(&fx, &[])), Some(1));
}

#[test]
fn ck09_rules_report_two_errors_and_one_warning() {
    let fx = py_workspace();
    fx.write(
        ".kgr.toml",
        b"[[rules]]\nname = \"app-no-pkg\"\nfrom = \"app.py\"\nto = \"pkg/**\"\nseverity = \"error\"\n\n[[rules]]\nname = \"warn-pkg\"\nfrom = \"pkg/**\"\nto = \"pkg/core.py\"\nseverity = \"warn\"\n",
    );
    let d = json_of(&check_cmd(&fx, &["-f", "json", "--exit-zero"]));
    let mut got: Vec<(String, String)> = d["rule_violations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v["rule"].as_str().unwrap().to_string(),
                v["severity"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    got.sort();
    let want = vec![
        ("app-no-pkg".to_string(), "error".to_string()),
        ("app-no-pkg".to_string(), "error".to_string()),
        ("warn-pkg".to_string(), "warn".to_string()),
    ];
    assert_eq!(got, want);
}

#[test]
fn ck10_broken_kgr_toml_exits_2() {
    let fx = py_workspace();
    fx.write(".kgr.toml", b"garbage = [\n");
    assert_eq!(code(&check_cmd(&fx, &[])), Some(2));
}

#[test]
#[ignore = "itr-20: a misspelled [[rule]] table is silently ignored, disabling enforcement"]
fn ck11_misspelled_rule_table_is_reported() {
    let fx = py_workspace();
    fx.write(
        ".kgr.toml",
        b"[[rule]]\nname = \"typo\"\nfrom = \"app.py\"\nto = \"pkg/**\"\nseverity = \"error\"\n",
    );
    let out = check_cmd(&fx, &["--exit-zero"]);
    let err = text(&out.stderr).to_lowercase();
    assert!(
        code(&out) != Some(0) || err.contains("unknown") || err.contains("rule"),
        "silently ignored; stderr: {err}"
    );
}

#[test]
fn ck12_malformed_baseline_exits_2() {
    let fx = py_workspace();
    fx.write(".kgr-baseline.json", b"{bad");
    assert_eq!(code(&check_cmd(&fx, &[])), Some(2));
}

#[test]
fn q01_who_imports_core_is_the_package_init() {
    let d = query(&py_workspace(), &["--who-imports", "pkg/core.py"]);
    assert_eq!(strs(&d), svec(&["pkg/__init__.py"]));
}

// Backslash separators are only path separators on Windows.
#[cfg(windows)]
#[test]
fn q02_who_imports_accepts_backslash_path() {
    let d = query(&py_workspace(), &["--who-imports", r"pkg\core.py"]);
    assert_eq!(strs(&d), svec(&["pkg/__init__.py"]));
}

#[test]
fn q03_deps_of_test_is_the_transitive_closure() {
    let d = query(&py_workspace(), &["--deps-of", "tests/test_app.py"]);
    assert_eq!(
        sorted(strs(&d)),
        svec(&["app.py", "pkg/__init__.py", "pkg/core.py", "pkg/util.py"])
    );
}

#[test]
fn q04_path_between_returns_the_import_chain() {
    let d = query(
        &py_workspace(),
        &["--path-between", "tests/test_app.py", "pkg/core.py"],
    );
    assert_eq!(
        strs(&d),
        svec(&[
            "tests/test_app.py",
            "app.py",
            "pkg/__init__.py",
            "pkg/core.py"
        ])
    );
}

#[test]
fn q05_path_between_without_a_path_is_null() {
    let d = query(
        &py_workspace(),
        &["--path-between", "pkg/core.py", "app.py"],
    );
    assert!(d.is_null());
}

#[test]
fn q06_orphans_are_exactly_orphan_py() {
    let d = query(&py_workspace(), &["--orphans"]);
    assert_eq!(strs(&d), svec(&["orphan.py"]));
}

#[test]
fn q07_largest_cycle_is_cyc_a_and_cyc_b() {
    let d = query(&py_workspace(), &["--largest-cycle"]);
    assert_eq!(sorted(strs(&d)), svec(&["cyc_a.py", "cyc_b.py"]));
}

#[test]
fn q08_query_unknown_target_exits_2() {
    let fx = py_workspace();
    let out = run(&[
        "query",
        "--who-imports",
        "nope.py",
        &fx.arg(),
        "--no-progress",
    ]);
    assert_eq!(code(&out), Some(2));
}

#[test]
#[ignore = "itr-25: error JSON `root` carries the verbatim \\\\?\\ extended-length prefix on Windows"]
fn q09_error_json_root_has_no_extended_length_prefix() {
    let fx = py_workspace();
    let out = run(&[
        "query",
        "--who-imports",
        "nope.py",
        &fx.arg(),
        "--no-progress",
        "-f",
        "json",
    ]);
    let d = json_of(&out);
    assert!(!d["root"].as_str().unwrap().starts_with(r"\\?"));
}

// B23 was refuted: `query --heaviest` (key `path`) and `orient` (key `file`)
// are separate schemas. What must hold is that each entry names a file and
// carries a numeric dependents count.
#[test]
fn q10_heaviest_entries_name_a_file_and_count_dependents() {
    let fx = py_workspace();
    let q = query(&fx, &["--heaviest"]);
    let first = &q[0];
    assert!(first["path"].is_string() || first["file"].is_string());
    assert!(first["dependents"].is_u64());

    let orient = json_of(&run(&["orient", &fx.arg(), "-f", "json", "--no-progress"]));
    let first = &orient["heaviest"][0];
    assert!(first["path"].is_string() || first["file"].is_string());
    assert!(first["dependents"].is_u64());
}

// ---------------------------------------------------------------------------
// cache: resolution must track file creation/deletion
// ---------------------------------------------------------------------------

#[test]
fn ca01_import_flips_external_local_external_as_target_comes_and_goes() {
    let fx = generated();
    fx.write("a.py", b"import b\n");
    let kind = |fx: &Fx| {
        // Cache deliberately enabled: this is the stale-resolution check.
        let out = kgr_without_host_kgr_env()
            .args(["graph", "-f", "json", "--no-progress", &fx.arg()])
            .output()
            .unwrap();
        imp(&json_of(&out), "a.py", "b").unwrap().0
    };
    let before = kind(&fx);
    fx.write("b.py", b"def x(): pass\n");
    let mid = kind(&fx);
    fx.remove("b.py");
    let after = kind(&fx);
    assert_eq!(
        (before.as_str(), mid.as_str(), after.as_str()),
        ("external", "local", "external")
    );
}
