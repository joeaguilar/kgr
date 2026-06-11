use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use kgr_core::detect::{detect_lang, detect_lang_from_shebang};
use kgr_core::types::Lang;

/// Cap on sample paths reported per skipped-unsupported group, mirroring
/// `PARSE_FAILURE_SAMPLE_LIMIT` in pipeline.rs.
const SKIPPED_SAMPLE_LIMIT: usize = 3;

/// Group label for skipped files without a file extension.
const NO_EXTENSION_GROUP: &str = "(no extension)";

/// Bytes sniffed from extensionless files to decide text vs binary.
const TEXT_SNIFF_BYTES: usize = 1024;

pub struct DiscoveredFile {
    pub path: PathBuf,
    pub lang: Lang,
    pub mtime: Option<SystemTime>,
    pub size: u64,
}

/// One group of walked-but-unsupported files sharing a file extension.
/// `sample` holds the first `SKIPPED_SAMPLE_LIMIT` paths in sorted order;
/// `count` is the full group size.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SkippedGroup {
    pub group: String,
    pub count: usize,
    pub sample: Vec<PathBuf>,
}

/// Result of a directory walk: analyzable files plus a bounded summary of
/// source-looking files kgr has no parser for. Reporting the latter keeps
/// broad codebase analysis honest — agents can grep-fallback instead of
/// assuming the graph covered everything.
pub struct Discovery {
    pub files: Vec<DiscoveredFile>,
    pub skipped_unsupported: Vec<SkippedGroup>,
}

pub fn discover(
    root: &Path,
    langs: &Option<Vec<String>>,
    exclude: &[String],
    max_file_size: Option<u64>,
) -> Discovery {
    let compiled_excludes = build_glob_set(exclude);
    for diagnostic in &compiled_excludes.diagnostics {
        diagnostic.emit();
    }
    let exclude_set = compiled_excludes.set;
    let root_buf = root.to_path_buf();

    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .filter_entry(move |e| {
            // Always descend into the root itself.
            if e.depth() == 0 {
                return true;
            }
            // Prune any path (file or directory) matching an exclude glob.
            let rel = e.path().strip_prefix(&root_buf).unwrap_or(e.path());
            !exclude_set.is_match(rel)
        })
        .build();

    let mut files = Vec::new();
    let mut skipped_paths = Vec::new();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }

        let meta = entry.metadata().ok();
        let mtime = meta.as_ref().and_then(|m| m.modified().ok());
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);

        // Skip files exceeding the size cap.
        if let Some(max) = max_file_size {
            if size > max {
                continue;
            }
        }

        let path = entry.into_path();
        let lang = detect_lang_for_file(&path);

        if lang == Lang::Unknown {
            // Track walked-but-unanalyzable files so a broad scan cannot
            // silently look complete. Only source-looking files count —
            // see `is_reportable_unsupported`.
            if is_reportable_unsupported(&path) {
                skipped_paths.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
            }
            continue;
        }

        // Filter by requested languages. Files of OTHER SUPPORTED languages
        // are intentionally filtered out here, not reported as skipped: the
        // skipped summary covers only files kgr could never analyze.
        if !lang_matches(lang, langs) {
            continue;
        }

        // Make path relative to root.
        let rel_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();

        files.push(DiscoveredFile {
            path: rel_path,
            lang,
            mtime,
            size,
        });
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    let skipped_unsupported = group_skipped_unsupported(skipped_paths);
    emit_skipped_unsupported_summary(&skipped_unsupported);

    Discovery {
        files,
        skipped_unsupported,
    }
}

/// Pragmatic filter for which `Lang::Unknown` files are worth reporting as
/// skipped. The goal is a useful signal — source files in languages kgr
/// cannot parse (Kotlin, Perl, Vue, ...) — not a dump of every asset:
/// - Files whose extension is a well-known non-source format (data/config,
///   docs, lockfiles, images, fonts, media, archives, binaries, certs) are
///   never reported.
/// - Extensionless files (which only reach here after shebang detection
///   already failed) are reported only when they are not well-known repo
///   metadata (LICENSE, CHANGELOG, ...) and look like non-empty text.
fn is_reportable_unsupported(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => !is_non_source_extension(ext),
        None => extensionless_looks_like_source(path),
    }
}

fn is_non_source_extension(ext: &str) -> bool {
    // Grouped for readability: data/config, lockfiles, docs, images, fonts,
    // media, archives, compiled/binary artifacts, certs/keys, scratch.
    const NON_SOURCE_EXTENSIONS: &[&str] = &[
        // data / config
        "cfg",
        "conf",
        "csv",
        "ini",
        "json",
        "json5",
        "jsonc",
        "plist",
        "properties",
        "toml",
        "tsv",
        "xml",
        "yaml",
        "yml",
        // lockfiles & dependency manifest pins
        "lock",
        "mod",
        "sum",
        // docs
        "adoc",
        "markdown",
        "md",
        "org",
        "pdf",
        "rst",
        "txt",
        // images
        "bmp",
        "gif",
        "icns",
        "ico",
        "jpeg",
        "jpg",
        "png",
        "svg",
        "tiff",
        "webp",
        // fonts
        "eot",
        "otf",
        "ttf",
        "woff",
        "woff2",
        // media
        "aac",
        "avi",
        "flac",
        "mov",
        "mp3",
        "mp4",
        "ogg",
        "wav",
        "webm",
        // archives
        "7z",
        "bz2",
        "gz",
        "jar",
        "rar",
        "tar",
        "tgz",
        "war",
        "xz",
        "zip",
        "zst",
        // compiled / binary artifacts
        "a",
        "bin",
        "class",
        "dat",
        "dll",
        "dylib",
        "exe",
        "lib",
        "o",
        "obj",
        "pdb",
        "pyc",
        "pyo",
        "so",
        "wasm",
        // certificates & keys
        "cer",
        "crt",
        "der",
        "key",
        "p12",
        "pem",
        "pfx",
        // scratch / generated
        "bak",
        "log",
        "map",
        "swp",
        "tmp",
    ];

    let ext = ext.to_ascii_lowercase();
    NON_SOURCE_EXTENSIONS.contains(&ext.as_str())
}

/// Heuristic for extensionless files: skip well-known repo metadata by name
/// (prefix match, so LICENSE-MIT also matches), then require the first
/// `TEXT_SNIFF_BYTES` to be non-empty NUL-free text. Anything binary or
/// empty is silently dropped — kgr was never meant to parse it.
fn extensionless_looks_like_source(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if is_repo_metadata_name(name) {
        return false;
    }

    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut buf = [0u8; TEXT_SNIFF_BYTES];
    let Ok(read) = file.read(&mut buf) else {
        return false;
    };
    read > 0 && !buf[..read].contains(&0)
}

/// Well-known extensionless repo metadata, matched by case-insensitive
/// prefix so variants like LICENSE-MIT or CHANGELOG2 are covered too.
fn is_repo_metadata_name(name: &str) -> bool {
    const NAMES: &[&str] = &[
        "AUTHORS",
        "CHANGELOG",
        "CODEOWNERS",
        "CONTRIBUTORS",
        "COPYING",
        "COPYRIGHT",
        "LICENCE",
        "LICENSE",
        "MAINTAINERS",
        "NOTICE",
        "OWNERS",
        "PATENTS",
        "README",
        "TODO",
        "VERSION",
    ];

    NAMES.iter().any(|meta| has_ci_prefix(name, meta))
}

fn has_ci_prefix(name: &str, prefix: &str) -> bool {
    name.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// Group skipped paths by lowercased extension (or `NO_EXTENSION_GROUP`),
/// keeping the first `SKIPPED_SAMPLE_LIMIT` sorted paths per group. Groups
/// are ordered largest-first, ties alphabetical, so the most significant
/// gaps surface first.
fn group_skipped_unsupported(mut skipped: Vec<PathBuf>) -> Vec<SkippedGroup> {
    skipped.sort();

    let mut grouped: BTreeMap<String, (usize, Vec<PathBuf>)> = BTreeMap::new();
    for path in skipped {
        let group = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_else(|| NO_EXTENSION_GROUP.to_string());
        let entry = grouped.entry(group).or_default();
        entry.0 += 1;
        if entry.1.len() < SKIPPED_SAMPLE_LIMIT {
            entry.1.push(path);
        }
    }

    let mut groups: Vec<SkippedGroup> = grouped
        .into_iter()
        .map(|(group, (count, sample))| SkippedGroup {
            group,
            count,
            sample,
        })
        .collect();
    // Stable sort: BTreeMap iteration is alphabetical, so ties stay sorted.
    groups.sort_by_key(|group| std::cmp::Reverse(group.count));
    groups
}

fn emit_skipped_unsupported_summary(groups: &[SkippedGroup]) {
    if let Some(summary) = skipped_unsupported_summary(groups) {
        tracing::warn!("{summary}");
    }
}

fn skipped_unsupported_summary(groups: &[SkippedGroup]) -> Option<String> {
    if groups.is_empty() {
        return None;
    }

    let total: usize = groups.iter().map(|g| g.count).sum();
    let parts: Vec<String> = groups
        .iter()
        .map(|g| {
            let sample: Vec<String> = g.sample.iter().map(|p| p.display().to_string()).collect();
            let omitted = g.count.saturating_sub(sample.len());
            if omitted == 0 {
                format!("{} x{} (e.g. {})", g.group, g.count, sample.join(", "))
            } else {
                format!(
                    "{} x{} (e.g. {}; {omitted} more)",
                    g.group,
                    g.count,
                    sample.join(", ")
                )
            }
        })
        .collect();

    Some(format!(
        "skipped {total} unsupported file(s) with no parser; graph coverage is partial — use grep for these: {}",
        parts.join("; ")
    ))
}

/// Discover a single explicitly-named file. `root` is the directory used as
/// the scan root (typically the file's parent directory); the returned path
/// is relative to it so the rest of the pipeline (parsing, import
/// resolution) works exactly as it does for directory scans.
///
/// Explicitly named files intentionally bypass config `exclude` globs — the
/// user asked for this file by name. Returns a human-readable reason when
/// the file cannot be analyzed.
pub fn discover_single_file(
    root: &Path,
    file: &Path,
    langs: &Option<Vec<String>>,
    max_file_size: Option<u64>,
) -> Result<DiscoveredFile, String> {
    let lang = detect_lang_for_file(file);
    if lang == Lang::Unknown {
        return Err("unsupported file type (unknown language)".to_string());
    }
    if !lang_matches(lang, langs) {
        return Err(format!("language '{}' excluded by --lang filter", lang));
    }

    let meta = std::fs::metadata(file).ok();
    let mtime = meta.as_ref().and_then(|m| m.modified().ok());
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);

    if let Some(max) = max_file_size {
        if size > max {
            return Err(format!(
                "file size {} exceeds the max file size limit of {} bytes",
                size, max
            ));
        }
    }

    let rel_path = file.strip_prefix(root).unwrap_or(file).to_path_buf();

    Ok(DiscoveredFile {
        path: rel_path,
        lang,
        mtime,
        size,
    })
}

fn detect_lang_for_file(path: &Path) -> Lang {
    let lang = detect_lang(path);
    if lang != Lang::Unknown || path.extension().is_some() {
        return lang;
    }

    read_first_line(path)
        .map(|line| detect_lang_from_shebang(&line))
        .unwrap_or(Lang::Unknown)
}

fn read_first_line(path: &Path) -> io::Result<String> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line)
}

/// True when `lang` passes the optional `--lang` filter (matched by full
/// name, e.g. "python", or short name, e.g. "py").
fn lang_matches(lang: Lang, langs: &Option<Vec<String>>) -> bool {
    let Some(filter) = langs else {
        return true;
    };
    let aliases: &[&str] = match lang {
        Lang::Python => &["py"],
        Lang::TypeScript => &["ts"],
        Lang::JavaScript => &["js"],
        Lang::Java => &["java"],
        Lang::C => &["c"],
        Lang::Cpp => &["cpp"],
        Lang::Rust => &["rs"],
        Lang::Go => &["go"],
        Lang::Zig => &["zig"],
        Lang::CSharp => &["cs"],
        Lang::ObjectiveC => &["objc", "objectivec"],
        Lang::Swift => &["swift"],
        Lang::Ruby => &["rb"],
        Lang::Php => &["php"],
        Lang::Scala => &["scala"],
        Lang::Lua => &["lua"],
        Lang::Elixir => &["ex"],
        Lang::Haskell => &["hs"],
        Lang::Bash => &["sh"],
        Lang::Unknown => return false,
    };
    let lang_str = lang.to_string();
    filter
        .iter()
        .any(|l| l == &lang_str || aliases.iter().any(|alias| l == alias))
}

struct CompiledExcludes {
    set: GlobSet,
    diagnostics: Vec<ExcludeGlobDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExcludeGlobDiagnostic {
    pattern: String,
    message: String,
}

impl ExcludeGlobDiagnostic {
    fn warning(&self) -> String {
        format!(
            "warning[kgr::exclude-config]: invalid exclude glob '{}': {}",
            self.pattern, self.message
        )
    }

    fn emit(&self) {
        eprintln!("{}", self.warning());
    }
}

fn build_glob_set(patterns: &[String]) -> CompiledExcludes {
    let mut builder = GlobSetBuilder::new();
    let mut diagnostics = Vec::new();

    for pat in patterns {
        match Glob::new(pat) {
            Ok(g) => {
                builder.add(g);
            }
            Err(error) => diagnostics.push(ExcludeGlobDiagnostic {
                pattern: pat.clone(),
                message: error.to_string(),
            }),
        }
    }

    // Building a GlobSet from valid globs is infallible.
    let set = builder
        .build()
        .unwrap_or_else(|_| GlobSetBuilder::new().build().unwrap());

    CompiledExcludes { set, diagnostics }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_exclude_glob_diagnostic_names_bad_pattern() {
        let patterns = vec!["src/[oops".to_string()];

        let compiled = build_glob_set(&patterns);

        assert_eq!(compiled.diagnostics.len(), 1);
        let warning = compiled.diagnostics[0].warning();
        assert!(
            warning.starts_with("warning[kgr::exclude-config]: invalid exclude glob 'src/[oops': ")
        );
        assert!(
            warning.len()
                > "warning[kgr::exclude-config]: invalid exclude glob 'src/[oops': ".len()
        );
    }

    #[test]
    fn valid_exclude_globs_still_apply_when_another_pattern_is_invalid() {
        let patterns = vec!["vendor/**".to_string(), "src/[oops".to_string()];

        let compiled = build_glob_set(&patterns);

        assert_eq!(compiled.diagnostics.len(), 1);
        assert!(compiled.set.is_match("vendor/generated.py"));
        assert!(!compiled.set.is_match("src/main.py"));
    }

    #[test]
    fn discovers_extensionless_script_from_shebang() {
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("deploy");
        std::fs::write(&script, "#!/usr/bin/env python3\nimport os\n").unwrap();

        let discovery = discover(tmp.path(), &None, &[], None);

        assert_eq!(discovery.files.len(), 1);
        assert_eq!(discovery.files[0].path, PathBuf::from("deploy"));
        assert_eq!(discovery.files[0].lang, Lang::Python);
        assert!(discovery.skipped_unsupported.is_empty());
    }

    #[test]
    fn extensionless_files_without_first_line_shebang_stay_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("notes"), "import os\n").unwrap();
        std::fs::write(
            tmp.path().join("late-shebang"),
            "\n#!/usr/bin/env python3\nimport os\n",
        )
        .unwrap();

        let discovery = discover(tmp.path(), &None, &[], None);

        assert!(discovery.files.is_empty());
        // ...but they are text without a recognized shebang, so the skipped
        // summary reports them instead of silently dropping them.
        assert_eq!(discovery.skipped_unsupported.len(), 1);
        let group = &discovery.skipped_unsupported[0];
        assert_eq!(group.group, NO_EXTENSION_GROUP);
        assert_eq!(group.count, 2);
    }

    #[test]
    fn skipped_unsupported_groups_by_extension_with_bounded_sorted_sample() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["epsilon", "alpha", "delta", "beta", "gamma"] {
            std::fs::write(tmp.path().join(format!("{name}.kt")), "fun main() {}\n").unwrap();
        }
        std::fs::write(tmp.path().join("script.pl"), "print 1;\n").unwrap();
        std::fs::write(tmp.path().join("app.py"), "import os\n").unwrap();

        let discovery = discover(tmp.path(), &None, &[], None);

        assert_eq!(discovery.files.len(), 1);
        assert_eq!(discovery.skipped_unsupported.len(), 2);

        // Largest group first; sample is sorted and capped.
        let kt = &discovery.skipped_unsupported[0];
        assert_eq!(kt.group, "kt");
        assert_eq!(kt.count, 5);
        assert_eq!(
            kt.sample,
            vec![
                PathBuf::from("alpha.kt"),
                PathBuf::from("beta.kt"),
                PathBuf::from("delta.kt"),
            ]
        );

        let pl = &discovery.skipped_unsupported[1];
        assert_eq!(pl.group, "pl");
        assert_eq!(pl.count, 1);
        assert_eq!(pl.sample, vec![PathBuf::from("script.pl")]);
    }

    #[test]
    fn non_source_assets_are_not_reported_as_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("config.json"), "{}\n").unwrap();
        std::fs::write(tmp.path().join("README.md"), "# readme\n").unwrap();
        std::fs::write(tmp.path().join("Cargo.lock"), "[[package]]\n").unwrap();
        std::fs::write(tmp.path().join("photo.png"), [0x89u8, b'P', 0x00]).unwrap();
        std::fs::write(tmp.path().join("LICENSE"), "MIT License\n").unwrap();
        std::fs::write(tmp.path().join("LICENSE-MIT"), "MIT License\n").unwrap();
        std::fs::write(tmp.path().join("empty"), "").unwrap();
        std::fs::write(tmp.path().join("blob"), [0u8, 159, 146, 150]).unwrap();
        std::fs::write(tmp.path().join("app.py"), "import os\n").unwrap();

        let discovery = discover(tmp.path(), &None, &[], None);

        assert_eq!(discovery.files.len(), 1);
        assert!(discovery.skipped_unsupported.is_empty());
    }

    #[test]
    fn lang_filtered_supported_files_are_not_counted_as_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("app.py"), "import os\n").unwrap();
        std::fs::write(tmp.path().join("app.ts"), "export const x = 1;\n").unwrap();
        std::fs::write(tmp.path().join("tool.kt"), "fun main() {}\n").unwrap();

        let discovery = discover(tmp.path(), &Some(vec!["py".to_string()]), &[], None);

        assert_eq!(discovery.files.len(), 1);
        assert_eq!(discovery.files[0].path, PathBuf::from("app.py"));
        // app.ts is a supported language excluded by --lang: filtered, not
        // skipped. Only the genuinely unsupported .kt file is reported.
        assert_eq!(discovery.skipped_unsupported.len(), 1);
        assert_eq!(discovery.skipped_unsupported[0].group, "kt");
        assert_eq!(discovery.skipped_unsupported[0].count, 1);
    }

    #[test]
    fn excluded_paths_never_reach_the_skipped_summary() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("vendor")).unwrap();
        std::fs::write(tmp.path().join("vendor/hidden.kt"), "fun main() {}\n").unwrap();
        std::fs::write(tmp.path().join("kept.kt"), "fun main() {}\n").unwrap();
        std::fs::write(tmp.path().join("app.py"), "import os\n").unwrap();

        let discovery = discover(tmp.path(), &None, &["vendor/**".to_string()], None);

        assert_eq!(discovery.skipped_unsupported.len(), 1);
        assert_eq!(discovery.skipped_unsupported[0].count, 1);
        assert_eq!(
            discovery.skipped_unsupported[0].sample,
            vec![PathBuf::from("kept.kt")]
        );
    }

    #[test]
    fn skipped_summary_message_is_bounded_and_mentions_grep_fallback() {
        let groups = vec![
            SkippedGroup {
                group: "kt".to_string(),
                count: 5,
                sample: vec![
                    PathBuf::from("a.kt"),
                    PathBuf::from("b.kt"),
                    PathBuf::from("c.kt"),
                ],
            },
            SkippedGroup {
                group: NO_EXTENSION_GROUP.to_string(),
                count: 1,
                sample: vec![PathBuf::from("Rakefile")],
            },
        ];

        let summary = skipped_unsupported_summary(&groups).unwrap();

        assert!(summary.contains("skipped 6 unsupported file(s)"));
        assert!(summary.contains("grep"));
        assert!(summary.contains("kt x5 (e.g. a.kt, b.kt, c.kt; 2 more)"));
        assert!(summary.contains("(no extension) x1 (e.g. Rakefile)"));

        assert!(skipped_unsupported_summary(&[]).is_none());
    }

    #[test]
    fn discovers_explicit_extensionless_file_from_shebang() {
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("run-node");
        std::fs::write(&script, "#!/usr/bin/env node\nrequire('./lib')\n").unwrap();

        let file = discover_single_file(tmp.path(), &script, &None, None).unwrap();

        assert_eq!(file.path, PathBuf::from("run-node"));
        assert_eq!(file.lang, Lang::JavaScript);
    }

    #[test]
    fn objective_c_filter_accepts_display_and_json_names() {
        assert!(lang_matches(
            Lang::ObjectiveC,
            &Some(vec!["objc".to_string()])
        ));
        assert!(lang_matches(
            Lang::ObjectiveC,
            &Some(vec!["objectivec".to_string()])
        ));
    }
}
