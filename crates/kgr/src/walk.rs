use std::path::{Path, PathBuf};
use std::time::SystemTime;

use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use kgr_core::detect::detect_lang;
use kgr_core::types::Lang;

pub struct DiscoveredFile {
    pub path: PathBuf,
    pub lang: Lang,
    pub mtime: Option<SystemTime>,
    pub size: u64,
}

pub fn discover(
    root: &Path,
    langs: &Option<Vec<String>>,
    exclude: &[String],
    max_file_size: Option<u64>,
) -> Vec<DiscoveredFile> {
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
        let lang = detect_lang(&path);

        if lang == Lang::Unknown {
            continue;
        }

        // Filter by requested languages.
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
    files
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
    let lang = detect_lang(file);
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
