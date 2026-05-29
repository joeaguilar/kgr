use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::types::{FileNode, ImportKind, Lang};

pub struct Resolver {
    known_files: HashSet<PathBuf>,
    #[expect(dead_code, reason = "root stored for future relative resolution")]
    root: PathBuf,
    tsconfig_paths: Vec<(String, String)>,
}

impl Resolver {
    pub fn new(root: PathBuf, files: &[FileNode]) -> Self {
        let known_files: HashSet<PathBuf> = files.iter().map(|f| f.path.clone()).collect();
        let tsconfig_paths = load_tsconfig_paths(&root);
        Self {
            known_files,
            root,
            tsconfig_paths,
        }
    }

    pub fn resolve_all(&self, files: &mut [FileNode]) {
        for file in files.iter_mut() {
            let lang = file.lang;
            let file_path = file.path.clone();
            for import in file.imports.iter_mut() {
                import.resolved = self.resolve(&import.raw, &file_path, lang);
                if import.resolved.is_some() {
                    import.kind = ImportKind::Local;
                }
            }
        }
    }

    fn resolve(&self, raw: &str, from: &Path, lang: Lang) -> Option<PathBuf> {
        match lang {
            Lang::Python => self.resolve_python(raw, from),
            Lang::TypeScript => self.resolve_js_ts(raw, from, &["ts", "tsx", "js", "jsx"]),
            Lang::JavaScript => self.resolve_js_ts(raw, from, &["js", "jsx", "mjs", "cjs", "ts"]),
            Lang::Java => self.resolve_java(raw),
            Lang::C | Lang::Cpp => self.resolve_c(raw, from),
            Lang::Rust => self.resolve_rust(raw, from),
            Lang::Go => self.resolve_go(raw, from),
            Lang::Zig
            | Lang::CSharp
            | Lang::ObjectiveC
            | Lang::Swift
            | Lang::Ruby
            | Lang::Php
            | Lang::Scala
            | Lang::Lua
            | Lang::Elixir
            | Lang::Haskell
            | Lang::Bash
            | Lang::Unknown => None,
        }
    }

    fn resolve_python(&self, raw: &str, from: &Path) -> Option<PathBuf> {
        let from_dir = from.parent().unwrap_or(Path::new(""));

        if raw.starts_with('.') {
            let dots = raw.chars().take_while(|c| *c == '.').count();
            let remainder = &raw[dots..];

            let mut base = from_dir.to_path_buf();
            for _ in 1..dots {
                base = base.parent().unwrap_or(Path::new("")).to_path_buf();
            }

            if remainder.is_empty() {
                let init = base.join("__init__.py");
                if self.known_files.contains(&init) {
                    return Some(init);
                }
                return None;
            }

            let module_path = remainder.replace('.', "/");
            let as_file = base.join(format!("{}.py", module_path));
            if self.known_files.contains(&as_file) {
                return Some(as_file);
            }
            let as_pkg = base.join(&module_path).join("__init__.py");
            if self.known_files.contains(&as_pkg) {
                return Some(as_pkg);
            }
            return None;
        }

        // Absolute import
        let module_path = raw.replace('.', "/");
        let rel_file = PathBuf::from(format!("{}.py", module_path));
        if self.known_files.contains(&rel_file) {
            return Some(rel_file);
        }
        let rel_pkg = PathBuf::from(&module_path).join("__init__.py");
        if self.known_files.contains(&rel_pkg) {
            return Some(rel_pkg);
        }

        let rel_from = from_dir.join(format!("{}.py", module_path));
        if self.known_files.contains(&rel_from) {
            return Some(rel_from);
        }

        None
    }

    fn resolve_js_ts(&self, raw: &str, from: &Path, extensions: &[&str]) -> Option<PathBuf> {
        // Try tsconfig paths first for non-relative imports
        if !raw.starts_with('.') {
            if let Some(resolved) = self.resolve_tsconfig_path(raw, extensions) {
                return Some(resolved);
            }
            return None;
        }

        let from_dir = from.parent().unwrap_or(Path::new(""));
        let target = from_dir.join(raw);
        let target = normalize_path(&target);

        if self.known_files.contains(&target) {
            return Some(target);
        }

        for ext in extensions {
            let with_ext = target.with_extension(ext);
            if self.known_files.contains(&with_ext) {
                return Some(with_ext);
            }
        }

        for ext in extensions {
            let index = target.join(format!("index.{}", ext));
            if self.known_files.contains(&index) {
                return Some(index);
            }
        }

        None
    }

    fn resolve_tsconfig_path(&self, raw: &str, extensions: &[&str]) -> Option<PathBuf> {
        for (prefix, target_dir) in &self.tsconfig_paths {
            if let Some(remainder) = raw.strip_prefix(prefix.trim_end_matches('*')) {
                let target_base = target_dir.trim_end_matches('*');
                let target = PathBuf::from(format!("{}{}", target_base, remainder));
                let target = normalize_path(&target);

                if self.known_files.contains(&target) {
                    return Some(target);
                }
                for ext in extensions {
                    let with_ext = target.with_extension(ext);
                    if self.known_files.contains(&with_ext) {
                        return Some(with_ext);
                    }
                }
                for ext in extensions {
                    let index = target.join(format!("index.{}", ext));
                    if self.known_files.contains(&index) {
                        return Some(index);
                    }
                }
            }
        }
        None
    }

    fn resolve_java(&self, raw: &str) -> Option<PathBuf> {
        // Convert com.example.MyClass -> com/example/MyClass.java
        let file_path = PathBuf::from(format!("{}.java", raw.replace('.', "/")));
        if self.known_files.contains(&file_path) {
            return Some(file_path);
        }
        // Also try with src/main/java prefix (common Maven layout)
        let maven_path = PathBuf::from(format!("src/main/java/{}.java", raw.replace('.', "/")));
        if self.known_files.contains(&maven_path) {
            return Some(maven_path);
        }
        None
    }

    fn resolve_c(&self, raw: &str, from: &Path) -> Option<PathBuf> {
        let from_dir = from.parent().unwrap_or(Path::new(""));

        // Try relative to including file
        let target = from_dir.join(raw);
        let target = normalize_path(&target);
        if self.known_files.contains(&target) {
            return Some(target);
        }

        // Try from project root
        let from_root = PathBuf::from(raw);
        if self.known_files.contains(&from_root) {
            return Some(from_root);
        }

        // Try common include directories
        for prefix in &["include", "src", "inc"] {
            let with_prefix = PathBuf::from(prefix).join(raw);
            if self.known_files.contains(&with_prefix) {
                return Some(with_prefix);
            }
        }

        None
    }

    fn resolve_rust(&self, raw: &str, from: &Path) -> Option<PathBuf> {
        let from_dir = from.parent().unwrap_or(Path::new("")).to_path_buf();

        // `mod foo;` — a submodule of the current file's module. From
        // mod.rs/lib.rs/main.rs it's a sibling (`foo.rs`); from `bar.rs` it
        // nests under `bar/`.
        if !raw.contains("::") {
            return self
                .try_module(&from_dir, &[raw])
                .or_else(|| self.try_module(&module_dir(from), &[raw]));
        }

        // Resolve the base directory and remaining path from the leading
        // qualifier. `crate::` is anchored at the owning crate's `src/` root
        // (NOT a hardcoded `src/` off the scan root, so workspaces resolve);
        // `self::`/`super::` are relative to the current module.
        let (base, rest): (PathBuf, &str) = if let Some(r) = raw.strip_prefix("crate::") {
            (crate_src_base(from), r)
        } else if let Some(r) = raw.strip_prefix("self::") {
            (module_dir(from), r)
        } else if raw.starts_with("super::") {
            let mut dir = module_dir(from);
            let mut r = raw;
            while let Some(stripped) = r.strip_prefix("super::") {
                dir = dir.parent().map(Path::to_path_buf).unwrap_or_default();
                r = stripped;
            }
            (dir, r)
        } else {
            // Bare 2018-edition path import of a crate-local module, e.g.
            // `use cli::Foo;` at the crate root. Falls through to External
            // (resolved stays None) if no such local module exists.
            (crate_src_base(from), raw)
        };

        let segments: Vec<&str> = rest
            .split("::")
            .filter(|s| !s.is_empty() && *s != "*")
            .collect();
        self.try_module(&base, &segments)
    }

    /// Resolve a `::`-separated module path under `base` to a file. The final
    /// segments of a `use` path may name an item (fn/type/const) rather than a
    /// module, so we shorten the path one segment at a time until it maps to a
    /// known `{path}.rs` or `{path}/mod.rs`.
    fn try_module(&self, base: &Path, segments: &[&str]) -> Option<PathBuf> {
        let mut segs = segments;
        while !segs.is_empty() {
            let joined = segs.join("/");
            let as_file = base.join(format!("{joined}.rs"));
            if self.known_files.contains(&as_file) {
                return Some(as_file);
            }
            let as_mod = base.join(&joined).join("mod.rs");
            if self.known_files.contains(&as_mod) {
                return Some(as_mod);
            }
            segs = &segs[..segs.len() - 1];
        }
        None
    }

    fn resolve_go(&self, raw: &str, from: &Path) -> Option<PathBuf> {
        if !raw.starts_with("./") && !raw.starts_with("../") {
            return None;
        }

        let from_dir = from.parent().unwrap_or(Path::new(""));
        let target = from_dir.join(raw);
        let target = normalize_path(&target);

        // Go: try the directory as a package — look for any .go file in it
        for known in &self.known_files {
            if known.starts_with(&target)
                && known.extension().and_then(|e| e.to_str()) == Some("go")
            {
                return Some(known.clone());
            }
        }

        None
    }
}

/// The crate's `src` directory for a root-relative file path: the nearest
/// ancestor directory named `src`. Falls back to the repo root, so both
/// single-crate (`src/main.rs`) and workspace (`crates/foo/src/lib.rs`)
/// layouts resolve `crate::` imports correctly.
fn crate_src_base(from: &Path) -> PathBuf {
    let mut current = from.parent();
    while let Some(dir) = current {
        if dir.file_name().and_then(|n| n.to_str()) == Some("src") {
            return dir.to_path_buf();
        }
        current = dir.parent();
    }
    PathBuf::new()
}

/// Directory that holds the submodules of the module defined by `from`.
/// `mod.rs`/`lib.rs`/`main.rs` own their containing directory; `foo.rs` owns a
/// sibling `foo/` directory.
fn module_dir(from: &Path) -> PathBuf {
    let dir = from.parent().unwrap_or(Path::new("")).to_path_buf();
    match from.file_stem().and_then(|s| s.to_str()) {
        Some("mod" | "lib" | "main") | None => dir,
        Some(stem) => dir.join(stem),
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            other => {
                components.push(other);
            }
        }
    }
    components.iter().collect()
}

/// Load tsconfig.json paths aliases if present
fn load_tsconfig_paths(root: &Path) -> Vec<(String, String)> {
    let tsconfig_path = root.join("tsconfig.json");
    if !tsconfig_path.exists() {
        return Vec::new();
    }

    let content = match std::fs::read_to_string(&tsconfig_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Failed to parse tsconfig.json: {}", e);
            return Vec::new();
        }
    };

    let mut paths = Vec::new();

    if let Some(compiler_options) = json.get("compilerOptions") {
        if let Some(path_map) = compiler_options.get("paths") {
            if let Some(obj) = path_map.as_object() {
                for (key, value) in obj {
                    if let Some(targets) = value.as_array() {
                        if let Some(first) = targets.first() {
                            if let Some(target) = first.as_str() {
                                paths.push((key.clone(), target.to_string()));
                            }
                        }
                    }
                }
            }
        }
    }

    if !paths.is_empty() {
        tracing::info!("Loaded {} tsconfig.json path aliases", paths.len());
    }

    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Import;

    fn node(path: &str) -> FileNode {
        FileNode {
            path: PathBuf::from(path),
            lang: Lang::Rust,
            imports: Vec::new(),
            symbols: Vec::new(),
            calls: Vec::new(),
        }
    }

    fn resolver(files: &[&str]) -> Resolver {
        let nodes: Vec<FileNode> = files.iter().map(|p| node(p)).collect();
        Resolver::new(PathBuf::new(), &nodes)
    }

    fn resolve_rust(files: &[&str], raw: &str, from: &str) -> Option<PathBuf> {
        resolver(files).resolve_rust(raw, Path::new(from))
    }

    #[test]
    fn crate_import_resolves_in_workspace_layout() {
        // The headline bug: `crate::` must anchor at the owning crate's src/,
        // not a hardcoded `src/` off the scan root.
        let got = resolve_rust(
            &["crates/core/src/lib.rs", "crates/core/src/types.rs"],
            "crate::types",
            "crates/core/src/lib.rs",
        );
        assert_eq!(got, Some(PathBuf::from("crates/core/src/types.rs")));
    }

    #[test]
    fn crate_import_resolves_in_single_crate_layout() {
        let got = resolve_rust(
            &["src/main.rs", "src/config.rs"],
            "crate::config",
            "src/main.rs",
        );
        assert_eq!(got, Some(PathBuf::from("src/config.rs")));
    }

    #[test]
    fn bare_local_module_import_resolves() {
        // `use cli::Command;` at the crate root (the ../itr false-positive).
        let got = resolve_rust(
            &["src/main.rs", "src/cli.rs"],
            "cli::Command",
            "src/main.rs",
        );
        assert_eq!(got, Some(PathBuf::from("src/cli.rs")));
    }

    #[test]
    fn trailing_item_segment_is_shortened_to_the_module() {
        // `crate::config::Settings` — Settings is an item, config is the module.
        let got = resolve_rust(
            &["src/main.rs", "src/config.rs"],
            "crate::config::Settings",
            "src/main.rs",
        );
        assert_eq!(got, Some(PathBuf::from("src/config.rs")));
    }

    #[test]
    fn external_crate_is_not_misresolved_as_local() {
        let got = resolve_rust(&["src/main.rs"], "serde::Serialize", "src/main.rs");
        assert_eq!(got, None);
    }

    #[test]
    fn mod_declaration_resolves_sibling() {
        let got = resolve_rust(&["src/lib.rs", "src/util.rs"], "util", "src/lib.rs");
        assert_eq!(got, Some(PathBuf::from("src/util.rs")));
    }

    #[test]
    fn super_import_resolves_relative_to_parent_module() {
        let got = resolve_rust(
            &["src/a/mod.rs", "src/a/b.rs", "src/a/c.rs"],
            "super::c",
            "src/a/b.rs",
        );
        assert_eq!(got, Some(PathBuf::from("src/a/c.rs")));
    }

    #[test]
    fn full_resolve_flips_kind_to_local() {
        // End-to-end through resolve_all: a bare local-module import is
        // upgraded from External to Local once resolved.
        let r = resolver(&["src/main.rs", "src/cli.rs"]);
        let mut files = vec![FileNode {
            path: PathBuf::from("src/main.rs"),
            lang: Lang::Rust,
            imports: vec![Import {
                raw: "cli::Command".to_string(),
                kind: ImportKind::External,
                resolved: None,
                span: None,
            }],
            symbols: Vec::new(),
            calls: Vec::new(),
        }];
        r.resolve_all(&mut files);
        assert_eq!(files[0].imports[0].kind, ImportKind::Local);
        assert_eq!(
            files[0].imports[0].resolved,
            Some(PathBuf::from("src/cli.rs"))
        );
    }
}
