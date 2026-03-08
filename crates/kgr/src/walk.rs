use std::path::{Path, PathBuf};
use std::time::SystemTime;

use globset::{GlobSet, GlobSetBuilder};
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
    let exclude_set = build_glob_set(exclude);
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
        if let Some(ref lang_filter) = langs {
            let lang_str = lang.to_string();
            let short = match lang {
                Lang::Python => "py",
                Lang::TypeScript => "ts",
                Lang::JavaScript => "js",
                Lang::Java => "java",
                Lang::C => "c",
                Lang::Cpp => "cpp",
                Lang::Rust => "rs",
                Lang::Go => "go",
                Lang::Unknown => continue,
            };
            if !lang_filter.iter().any(|l| l == &lang_str || l == short) {
                continue;
            }
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

fn build_glob_set(patterns: &[String]) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        if let Ok(g) = globset::Glob::new(pat) {
            builder.add(g);
        }
    }
    // Building a GlobSet from valid globs is infallible.
    builder
        .build()
        .unwrap_or_else(|_| GlobSetBuilder::new().build().unwrap())
}
