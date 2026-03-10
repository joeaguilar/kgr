use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::types::{FileNode, ImportKind, Lang};

pub struct Resolver {
    known_files: HashSet<PathBuf>,
    #[allow(dead_code)]
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
        let from_dir = from.parent().unwrap_or(Path::new(""));

        // For mod declarations: check {name}.rs and {name}/mod.rs
        // raw is just the module name (e.g. "utils")
        if !raw.contains("::") {
            let as_file = from_dir.join(format!("{}.rs", raw));
            if self.known_files.contains(&as_file) {
                return Some(as_file);
            }
            let as_mod = from_dir.join(raw).join("mod.rs");
            if self.known_files.contains(&as_mod) {
                return Some(as_mod);
            }
        }

        // For use crate::path::to::module
        if let Some(stripped) = raw.strip_prefix("crate::") {
            let parts = stripped.replace("::", "/");
            let as_file = PathBuf::from(format!("src/{}.rs", parts));
            if self.known_files.contains(&as_file) {
                return Some(as_file);
            }
            let as_mod = PathBuf::from(format!("src/{}/mod.rs", parts));
            if self.known_files.contains(&as_mod) {
                return Some(as_mod);
            }
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
