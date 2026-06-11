use std::path::{Path, PathBuf};

use figment::providers::{Env, Format, Serialized, Toml};
use figment::Figment;
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    #[default]
    Error,
    Warn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub name: String,
    /// Glob pattern for the importing file
    pub from: String,
    /// Glob pattern for the imported file
    pub to: String,
    #[serde(default)]
    pub severity: Severity,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Default `--lang` filter applied when the CLI flag is absent.
    /// The CLI flag always wins when given.
    /// Example: `languages = ["rs", "py"]`
    #[serde(default)]
    pub languages: Option<Vec<String>>,
    /// Glob patterns (relative to root) to exclude from scanning.
    /// Matched paths are skipped entirely — directories are not walked into.
    /// Example: `["vendor/**", "third_party/**", "generated/**"]`
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Skip files larger than this many kilobytes. Useful for ignoring
    /// generated or vendored megabyte-scale files that slow down parsing.
    /// Example: `max_file_size_kb = 500`
    #[serde(default)]
    pub max_file_size_kb: Option<u64>,
    /// Default `--format` applied when the CLI flag is absent.
    /// Precedence: CLI flag > this setting > each subcommand's built-in
    /// default (graph: tree, check: text, query: table, ...).
    #[serde(default)]
    pub format: Option<String>,
    /// Reserved: paired with the `--no-external` flag (currently a render
    /// no-op, tracked separately). Kept for config-file compatibility.
    #[serde(default)]
    pub no_external: bool,
    /// Suppress the progress bar by default (same as `--no-progress`).
    /// OR-semantics: either the CLI flag or this setting suppresses it.
    #[serde(default)]
    pub no_progress: bool,
    /// Enable first-party filtering by default (same as always passing
    /// `--first-party`): vendored paths are excluded from orphan and
    /// heaviest analysis. OR-semantics: either the CLI flag or this
    /// setting enables it. Vendored files stay in the graph — only the
    /// analysis summaries are filtered.
    #[serde(default)]
    pub first_party: bool,
    /// Glob patterns (relative to root) marking paths as vendored for the
    /// first-party filter. Only consulted when filtering is enabled; when
    /// unset, `DEFAULT_VENDOR_GLOBS` applies.
    /// Example: `vendor_globs = ["**/vendor/**", "deps/**"]`
    #[serde(default)]
    pub vendor_globs: Option<Vec<String>>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl Config {
    /// `max_file_size_kb` converted to bytes, or `None` if not set.
    pub fn max_file_size_bytes(&self) -> Option<u64> {
        self.max_file_size_kb.map(|kb| kb * 1024)
    }
}

/// Resolve the effective output format: CLI flag > config `format` >
/// the subcommand's built-in default.
pub fn resolve_format<'a>(
    cli: Option<&'a str>,
    config: Option<&'a str>,
    built_in: &'a str,
) -> &'a str {
    cli.or(config).unwrap_or(built_in)
}

/// Resolve the effective language filter: the CLI `--lang` flag wins when
/// given; otherwise config `languages` applies; otherwise no filter.
pub fn resolve_langs(
    cli: &Option<Vec<String>>,
    config: &Option<Vec<String>>,
) -> Option<Vec<String>> {
    cli.clone().or_else(|| config.clone())
}

/// Resolve progress-bar suppression with OR-semantics: either the CLI
/// `--no-progress` flag or config `no_progress = true` suppresses it.
pub fn resolve_no_progress(cli: bool, config: bool) -> bool {
    cli || config
}

/// Resolve first-party filtering with OR-semantics: either the CLI
/// `--first-party` flag or config `first_party = true` enables it.
pub fn resolve_first_party(cli: bool, config: bool) -> bool {
    cli || config
}

/// Default vendor globs for the first-party filter. `**/` matches zero or
/// more leading directories, so these cover both top-level (`vendor/x.h`)
/// and nested (`libs/vendor/x.h`) vendored trees.
pub const DEFAULT_VENDOR_GLOBS: &[&str] = &["**/vendor/**", "**/third_party/**", "**/external/**"];

/// The vendor globs in effect: config `vendor_globs` when set, otherwise
/// `DEFAULT_VENDOR_GLOBS`.
pub fn effective_vendor_globs(config: &Option<Vec<String>>) -> Vec<String> {
    match config {
        Some(globs) => globs.clone(),
        None => DEFAULT_VENDOR_GLOBS
            .iter()
            .map(|g| (*g).to_string())
            .collect(),
    }
}

/// Compiled first-party filter: root-relative paths matching any vendor
/// glob are treated as vendored and excluded from orphan/heaviest analysis
/// when filtering is enabled.
pub struct FirstPartyFilter {
    set: GlobSet,
    globs: Vec<String>,
}

impl FirstPartyFilter {
    /// Compile vendor globs. Invalid patterns are skipped (valid ones
    /// still apply) and reported back as warning strings for the caller
    /// to emit, mirroring the exclude-glob diagnostics in walk.rs.
    pub fn compile(patterns: Vec<String>) -> (Self, Vec<String>) {
        let mut builder = GlobSetBuilder::new();
        let mut globs = Vec::new();
        let mut warnings = Vec::new();

        for pattern in patterns {
            match Glob::new(&pattern) {
                Ok(glob) => {
                    builder.add(glob);
                    globs.push(pattern);
                }
                Err(error) => warnings.push(format!(
                    "warning[kgr::vendor-config]: invalid vendor glob '{pattern}': {error}"
                )),
            }
        }

        // Building a GlobSet from valid globs is infallible.
        let set = builder
            .build()
            .unwrap_or_else(|_| GlobSetBuilder::new().build().unwrap());

        (Self { set, globs }, warnings)
    }

    /// True when `path` (relative to the scan root) matches a vendor glob.
    pub fn is_vendored(&self, path: &Path) -> bool {
        self.set.is_match(path)
    }

    /// The vendor globs actually applied (invalid patterns excluded).
    pub fn globs(&self) -> &[String] {
        &self.globs
    }
}

pub fn load_config(root: &Path) -> Result<Config, Box<figment::Error>> {
    let config_path = root.join(".kgr.toml");

    let figment = Figment::new().merge(Serialized::defaults(Config::default()));

    let figment = if config_path.exists() {
        figment.merge(Toml::file(&config_path))
    } else {
        figment
    };

    figment
        .merge(Env::prefixed("KGR_"))
        .extract()
        .map_err(Box::new)
}

pub fn init_config(root: &Path, force: bool) -> std::io::Result<PathBuf> {
    let config_path = root.join(".kgr.toml");

    if config_path.exists() && !force {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!(
                "{} already exists; refusing to overwrite (use --force to replace it)",
                config_path.display()
            ),
        ));
    }

    let mut detected = std::collections::HashSet::new();

    for e in ignore::Walk::new(root).flatten() {
        if let Some(ext) = e.path().extension().and_then(|e| e.to_str()) {
            match ext {
                "py" | "pyi" => {
                    detected.insert("py");
                }
                "ts" | "tsx" => {
                    detected.insert("ts");
                }
                "js" | "jsx" | "mjs" | "cjs" => {
                    detected.insert("js");
                }
                "java" => {
                    detected.insert("java");
                }
                "c" | "h" => {
                    detected.insert("c");
                }
                "cpp" | "cc" | "cxx" | "hpp" => {
                    detected.insert("cpp");
                }
                "rs" => {
                    detected.insert("rs");
                }
                "go" => {
                    detected.insert("go");
                }
                _ => {}
            }
        }
    }

    let mut langs: Vec<&str> = detected.into_iter().collect();
    langs.sort();
    let langs: Vec<String> = langs.iter().map(|l| format!("\"{}\"", l)).collect();

    let content = format!(
        r#"# .kgr.toml — project configuration for kgr

# Glob patterns (relative to project root) to skip entirely.
# Matched directories are not walked into, so this is fast.
# exclude = ["vendor/**", "third_party/**", "generated/**"]
exclude = []

# Skip files larger than this many kilobytes (speeds up cold-cache scans
# of projects with large vendored or generated files).
# max_file_size_kb = 500

# Detected languages — the default --lang filter when the CLI flag is
# absent (the CLI flag always wins).
# languages = [{}]

# Default output format when --format is not given on the command line.
# Precedence: CLI flag > this setting > each subcommand's built-in default
# (graph: tree, check: text, query: table, ...).
# format = "json"

# Suppress the progress bar by default (same as always passing --no-progress).
# no_progress = true

# Exclude vendored paths from orphan and heaviest analysis by default
# (same as always passing --first-party). Vendored files stay in the
# graph; only the analysis summaries are filtered.
# first_party = true

# Paths treated as vendored when first-party filtering is enabled.
# Defaults to ["**/vendor/**", "**/third_party/**", "**/external/**"].
# vendor_globs = ["**/vendor/**", "**/third_party/**", "**/external/**"]

# Enforce architectural boundaries. Each rule checks that no import
# edge runs from a 'from' file to a 'to' file matching the globs.
# severity: "error" (default, fails kgr check) or "warn" (informational).
#
# [[rules]]
# name = "no-legacy-to-core"
# from = "src/legacy/**"
# to   = "src/core/**"
# severity = "error"
"#,
        langs.join(", ")
    );

    std::fs::write(&config_path, content)?;
    Ok(config_path)
}

#[cfg(test)]
mod tests {
    use crate::test_env::{CleanKgrEnv, KGR_ENV_LOCK};

    use super::*;

    #[test]
    fn resolve_format_cli_beats_config_and_built_in() {
        assert_eq!(resolve_format(Some("dot"), Some("json"), "tree"), "dot");
    }

    #[test]
    fn resolve_format_config_beats_built_in() {
        assert_eq!(resolve_format(None, Some("json"), "tree"), "json");
    }

    #[test]
    fn resolve_format_falls_back_to_built_in() {
        assert_eq!(resolve_format(None, None, "tree"), "tree");
    }

    #[test]
    fn resolve_langs_cli_beats_config() {
        let cli = Some(vec!["py".to_string()]);
        let config = Some(vec!["rs".to_string()]);
        assert_eq!(resolve_langs(&cli, &config), Some(vec!["py".to_string()]));
    }

    #[test]
    fn resolve_langs_config_applies_when_cli_absent() {
        let config = Some(vec!["rs".to_string()]);
        assert_eq!(resolve_langs(&None, &config), Some(vec!["rs".to_string()]));
    }

    #[test]
    fn resolve_langs_none_when_neither_set() {
        assert_eq!(resolve_langs(&None, &None), None);
    }

    #[test]
    fn resolve_no_progress_or_semantics() {
        assert!(!resolve_no_progress(false, false));
        assert!(resolve_no_progress(true, false));
        assert!(resolve_no_progress(false, true));
        assert!(resolve_no_progress(true, true));
    }

    #[test]
    fn resolve_first_party_or_semantics() {
        assert!(!resolve_first_party(false, false));
        assert!(resolve_first_party(true, false));
        assert!(resolve_first_party(false, true));
        assert!(resolve_first_party(true, true));
    }

    #[test]
    fn effective_vendor_globs_defaults_when_unset() {
        assert_eq!(
            effective_vendor_globs(&None),
            DEFAULT_VENDOR_GLOBS
                .iter()
                .map(|g| (*g).to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn effective_vendor_globs_config_replaces_defaults() {
        let config = Some(vec!["legacy/**".to_string()]);
        assert_eq!(
            effective_vendor_globs(&config),
            vec!["legacy/**".to_string()]
        );
    }

    /// Default vendor globs match top-level and nested vendor directories
    /// by path, never by file name: `src/foo.h` stays first-party even
    /// when `vendor/foo.h` shares its name.
    #[test]
    fn default_vendor_globs_match_by_path_not_name() {
        let (filter, warnings) = FirstPartyFilter::compile(effective_vendor_globs(&None));
        assert!(warnings.is_empty());

        assert!(filter.is_vendored(Path::new("vendor/foo.h")));
        assert!(filter.is_vendored(Path::new("third_party/bar.hpp")));
        assert!(filter.is_vendored(Path::new("external/baz.h")));
        assert!(filter.is_vendored(Path::new("libs/vendor/deep/foo.h")));

        assert!(!filter.is_vendored(Path::new("src/foo.h")));
        assert!(!filter.is_vendored(Path::new("foo.h")));
        // Component match only — `myvendor/` is not `vendor/`.
        assert!(!filter.is_vendored(Path::new("myvendor/foo.h")));
    }

    #[test]
    fn invalid_vendor_glob_is_skipped_with_warning() {
        let (filter, warnings) =
            FirstPartyFilter::compile(vec!["vendor/**".to_string(), "src/[oops".to_string()]);

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0]
            .starts_with("warning[kgr::vendor-config]: invalid vendor glob 'src/[oops': "));
        // The valid glob still applies and is the only one reported.
        assert!(filter.is_vendored(Path::new("vendor/foo.h")));
        assert_eq!(filter.globs(), &["vendor/**".to_string()]);
    }

    /// All load_config assertions that touch KGR_FORMAT live in this single
    /// test so the env-var manipulation cannot race a parallel test reading
    /// the same key. (Other tests in this process call `load_config` but
    /// never consume `format`.)
    #[test]
    fn load_config_layers_defaults_toml_and_env() {
        let _env_lock = KGR_ENV_LOCK.lock().unwrap();
        let _env = CleanKgrEnv::new();
        let dir = tempfile::tempdir().unwrap();

        // Built-in defaults: optional fields stay unset.
        let cfg = load_config(dir.path()).unwrap();
        assert_eq!(cfg.format, None);
        assert_eq!(cfg.languages, None);
        assert!(!cfg.no_progress);

        // Toml layer fills in the wired fields.
        std::fs::write(
            dir.path().join(".kgr.toml"),
            "languages = [\"rs\"]\nformat = \"table\"\nno_progress = true\n",
        )
        .unwrap();
        let cfg = load_config(dir.path()).unwrap();
        assert_eq!(cfg.languages, Some(vec!["rs".to_string()]));
        assert_eq!(cfg.format.as_deref(), Some("table"));
        assert!(cfg.no_progress);

        // Env layer (KGR_FORMAT) wins over the toml value.
        std::env::set_var("KGR_FORMAT", "json");
        let cfg = load_config(dir.path()).unwrap();
        assert_eq!(cfg.format.as_deref(), Some("json"));
    }

    /// `depth`, `entry`, and `no_color` were removed from Config (never
    /// consumed); old config files that still set them must load fine.
    #[test]
    fn load_config_ignores_removed_legacy_keys() {
        let _env_lock = KGR_ENV_LOCK.lock().unwrap();
        let _env = CleanKgrEnv::new();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".kgr.toml"),
            "depth = 3\nentry = \"src/main.py\"\nno_color = true\n",
        )
        .unwrap();
        let cfg = load_config(dir.path()).unwrap();
        assert_eq!(cfg.languages, None);
        assert!(cfg.exclude.is_empty());
    }

    #[test]
    fn init_config_creates_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = init_config(dir.path(), false).unwrap();
        assert_eq!(path, dir.path().join(".kgr.toml"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# .kgr.toml"));
    }

    #[test]
    fn init_config_refuses_to_overwrite_existing_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(".kgr.toml");
        let existing = "[[rules]]\nname = \"keep-me\"\nfrom = \"a/**\"\nto = \"b/**\"\n";
        std::fs::write(&config_path, existing).unwrap();

        let err = init_config(dir.path(), false).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(err.to_string().contains(".kgr.toml"));

        // The existing file must be preserved byte-for-byte.
        let content = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(content, existing);
    }

    #[test]
    fn init_config_overwrites_existing_with_force() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(".kgr.toml");
        std::fs::write(&config_path, "[[rules]]\nname = \"old\"\n").unwrap();

        let path = init_config(dir.path(), true).unwrap();
        assert_eq!(path, config_path);
        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("# .kgr.toml"));
        assert!(!content.contains("\"old\""));
    }
}
