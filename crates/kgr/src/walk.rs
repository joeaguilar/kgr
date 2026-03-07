use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use kgr_core::detect::detect_lang;
use kgr_core::types::Lang;

pub struct DiscoveredFile {
    pub path: PathBuf,
    pub lang: Lang,
}

pub fn discover(root: &Path, langs: &Option<Vec<String>>) -> Vec<DiscoveredFile> {
    let walker = WalkBuilder::new(root).hidden(true).git_ignore(true).build();

    let mut files = Vec::new();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }

        let path = entry.into_path();
        let lang = detect_lang(&path);

        if lang == Lang::Unknown {
            continue;
        }

        // Filter by requested languages
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

        // Make path relative to root
        let rel_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();

        files.push(DiscoveredFile {
            path: rel_path,
            lang,
        });
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}
