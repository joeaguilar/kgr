use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::types::{FileNode, ImportKind, Lang};

/// One `compilerOptions.paths` alias: the key pattern (exact like `jquery`
/// or single-wildcard like `@app/*`) plus ALL of its targets, already
/// anchored to `compilerOptions.baseUrl` (root-relative).
#[derive(Debug, Clone)]
struct TsPathAlias {
    pattern: String,
    targets: Vec<String>,
}

pub struct Resolver {
    known_files: HashSet<PathBuf>,
    #[expect(dead_code, reason = "root stored for future relative resolution")]
    root: PathBuf,
    tsconfig_paths: Vec<TsPathAlias>,
    go_module: Option<String>,
}

impl Resolver {
    pub fn new(root: PathBuf, files: &[FileNode]) -> Self {
        let known_files: HashSet<PathBuf> = files.iter().map(|f| f.path.clone()).collect();
        let tsconfig_paths = load_tsconfig_paths(&root);
        let go_module = load_go_module(&root);
        Self {
            known_files,
            root,
            tsconfig_paths,
            go_module,
        }
    }

    pub fn resolve_all(&self, files: &mut [FileNode]) {
        for file in files.iter_mut() {
            let lang = file.lang;
            let file_path = file.path.clone();
            for import in file.imports.iter_mut() {
                import.resolved = self.resolve(&import.raw, &file_path, lang, import.kind);
                // A resolved import is upgraded to Local — EXCEPT when the
                // parser classified it as System (C/C++/ObjC angle
                // includes): a project header that happens to share a system
                // header's name must not erase the System classification.
                if import.resolved.is_some() && import.kind != ImportKind::System {
                    import.kind = ImportKind::Local;
                }
            }
        }
    }

    fn resolve(&self, raw: &str, from: &Path, lang: Lang, kind: ImportKind) -> Option<PathBuf> {
        match lang {
            Lang::Python => self.resolve_python(raw, from),
            Lang::TypeScript => self.resolve_js_ts(raw, from, &["ts", "tsx", "js", "jsx"]),
            Lang::JavaScript => self.resolve_js_ts(raw, from, &["js", "jsx", "mjs", "cjs", "ts"]),
            Lang::Java => self.resolve_java(raw),
            Lang::C | Lang::Cpp => self.resolve_c(raw, from, kind),
            Lang::Rust => self.resolve_rust(raw, from),
            Lang::Go => self.resolve_go(raw, from),
            Lang::Ruby => self.resolve_ruby(raw, from),
            Lang::Php => self.resolve_php(raw, from),
            Lang::Lua => self.resolve_lua(raw, from),
            Lang::Bash => self.resolve_bash(raw, from),
            Lang::Zig => self.resolve_zig(raw, from),
            Lang::CSharp
            | Lang::ObjectiveC
            | Lang::Swift
            | Lang::Scala
            | Lang::Elixir
            | Lang::Haskell
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

        self.resolve_js_ts_target(&target, extensions)
    }

    /// Resolve a normalized js/ts target path against known files: exact
    /// match, then extension candidates, then directory index files.
    ///
    /// Extension handling must APPEND to the full specifier (`user.service`
    /// -> `user.service.ts`) rather than `Path::with_extension`, which
    /// truncates at the last dot and would try `user.ts` — breaking the
    /// standard Angular/NestJS dotted names (`*.service`, `*.module`,
    /// `*.types`). Extension SUBSTITUTION is kept only for NodeNext-style
    /// specifiers whose existing extension is a js-family one
    /// (`./foo.js` -> `foo.ts`).
    fn resolve_js_ts_target(&self, target: &Path, extensions: &[&str]) -> Option<PathBuf> {
        if self.known_files.contains(target) {
            return Some(target.to_path_buf());
        }

        for ext in extensions {
            let mut appended = target.as_os_str().to_os_string();
            appended.push(".");
            appended.push(ext);
            let appended = PathBuf::from(appended);
            if self.known_files.contains(&appended) {
                return Some(appended);
            }
        }

        let is_js_family = target
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e, "js" | "jsx" | "mjs" | "cjs"));
        if is_js_family {
            for ext in extensions {
                let with_ext = target.with_extension(ext);
                if self.known_files.contains(&with_ext) {
                    return Some(with_ext);
                }
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

    /// TypeScript `paths` matching semantics: an exact (no-`*`) key matches
    /// only the full specifier and wins outright; otherwise the wildcard
    /// pattern with the longest matched prefix wins, and the text matched by
    /// `*` is substituted into each of that pattern's targets in order.
    fn resolve_tsconfig_path(&self, raw: &str, extensions: &[&str]) -> Option<PathBuf> {
        if let Some(alias) = self
            .tsconfig_paths
            .iter()
            .find(|a| !a.pattern.contains('*') && a.pattern == raw)
        {
            return self.try_alias_targets(alias, "", extensions);
        }

        let mut best: Option<(&TsPathAlias, &str)> = None;
        let mut best_prefix_len = 0usize;
        for alias in &self.tsconfig_paths {
            let Some(star) = alias.pattern.find('*') else {
                continue;
            };
            let prefix = &alias.pattern[..star];
            let suffix = &alias.pattern[star + 1..];
            if raw.len() >= prefix.len() + suffix.len()
                && raw.starts_with(prefix)
                && raw.ends_with(suffix)
                && (best.is_none() || prefix.len() > best_prefix_len)
            {
                best = Some((alias, &raw[prefix.len()..raw.len() - suffix.len()]));
                best_prefix_len = prefix.len();
            }
        }
        let (alias, remainder) = best?;
        self.try_alias_targets(alias, remainder, extensions)
    }

    /// Try every target of the chosen alias in declaration order, replacing
    /// the target's `*` (if any) with the matched remainder; the first
    /// candidate that resolves to a known file wins.
    fn try_alias_targets(
        &self,
        alias: &TsPathAlias,
        remainder: &str,
        extensions: &[&str],
    ) -> Option<PathBuf> {
        for target in &alias.targets {
            let candidate = match target.find('*') {
                Some(star) => format!("{}{}{}", &target[..star], remainder, &target[star + 1..]),
                None => target.clone(),
            };
            let candidate = normalize_path(Path::new(&candidate));
            if let Some(resolved) = self.resolve_js_ts_target(&candidate, extensions) {
                return Some(resolved);
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

    /// C/C++ `#include` resolution, honoring the parser's classification.
    ///
    /// Quoted includes (`#include "x.h"`, parsed as Local) follow the
    /// preprocessor's quote search: the includer's directory first, then the
    /// project root and conventional include dirs. Angle includes
    /// (`#include <x.h>`, parsed as System) NEVER search the includer's
    /// directory — they only consult the conventional public include roots
    /// (`include/`, `inc/`, the typical `-I` dirs), so a project header that
    /// merely shadows a system header's name (`src/string.h` next to a file
    /// saying `#include <string.h>`) produces no phantom edge.
    fn resolve_c(&self, raw: &str, from: &Path, kind: ImportKind) -> Option<PathBuf> {
        if kind == ImportKind::System {
            for prefix in &["include", "inc"] {
                let with_prefix = PathBuf::from(prefix).join(raw);
                if self.known_files.contains(&with_prefix) {
                    return Some(with_prefix);
                }
            }
            return None;
        }

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
        // nests under `bar/`. The module dir must win: rustc only accepts
        // `bar/foo.rs` for `mod foo;` in `bar.rs`, so when both `bar/foo.rs`
        // and a sibling `foo.rs` exist, the child is the real target. The
        // sibling is kept only as a lenient fallback (e.g. `#[path]` quirks).
        // For mod.rs/lib.rs/main.rs the two candidates are the same dir.
        if !raw.contains("::") {
            return self
                .try_module(&module_dir(from), &[raw])
                .or_else(|| self.try_module(&from_dir, &[raw]));
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

        if rest.split("::").any(|s| s == "*") {
            return None;
        }

        let segments: Vec<&str> = rest.split("::").filter(|s| !s.is_empty()).collect();
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

    /// Go import resolution. Relative imports (`./x`, `../x`) resolve against
    /// the importing file's dir (legacy/GOPATH style). Module-path imports
    /// (`github.com/user/repo/internal/x`) resolve via the go.mod `module`
    /// directive: the remainder after the module path maps to a package
    /// directory under the scan root, and the import resolves to a `.go`
    /// file among that directory's DIRECT children only — nested dirs are
    /// different Go packages. Stdlib and third-party imports never match a
    /// known file and stay External/unresolved.
    fn resolve_go(&self, raw: &str, from: &Path) -> Option<PathBuf> {
        if raw.starts_with("./") || raw.starts_with("../") {
            let from_dir = from.parent().unwrap_or(Path::new(""));
            let target = from_dir.join(raw);
            let target = normalize_path(&target);

            // Go: try the directory as a package and choose a stable member file.
            return self
                .known_files
                .iter()
                .filter(|known| {
                    known.starts_with(&target)
                        && known.extension().and_then(|e| e.to_str()) == Some("go")
                })
                .min()
                .cloned();
        }

        // Module-path import: strip the module path to get the package dir.
        // An import equal to the module path itself is the root package.
        let module = self.go_module.as_deref()?;
        let pkg_dir = if raw == module {
            PathBuf::new()
        } else {
            PathBuf::from(raw.strip_prefix(module)?.strip_prefix('/')?)
        };

        // Deterministic choice: lexicographically first .go file directly
        // inside the package directory.
        self.known_files
            .iter()
            .filter(|known| {
                known.parent().unwrap_or(Path::new("")) == pkg_dir.as_path()
                    && known.extension().and_then(|e| e.to_str()) == Some("go")
            })
            .min()
            .cloned()
    }

    /// Ruby `require_relative 'helper'` / `require './lib/utils'`. The parser
    /// emits the bare name for require_relative, so both forms resolve
    /// relative to the importing file's dir, appending `.rb` when missing.
    /// Non-relative `require 'json'` only flips to Local on a confirmed
    /// sibling hit; gems simply never match a known file.
    fn resolve_ruby(&self, raw: &str, from: &Path) -> Option<PathBuf> {
        if raw.starts_with('/') {
            return None;
        }
        let from_dir = from.parent().unwrap_or(Path::new(""));
        let has_rb_ext = Path::new(raw)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("rb"));
        let with_ext = if has_rb_ext {
            raw.to_string()
        } else {
            format!("{raw}.rb")
        };
        let target = normalize_path(&from_dir.join(with_ext));
        self.known_files.contains(&target).then_some(target)
    }

    /// PHP `include`/`require` with a relative path resolves against the
    /// including file's dir, then the project root (include_path-style).
    /// Namespace `use Foo\Bar;` imports are not file paths.
    fn resolve_php(&self, raw: &str, from: &Path) -> Option<PathBuf> {
        if raw.contains('\\') || raw.starts_with('/') {
            return None;
        }
        let from_dir = from.parent().unwrap_or(Path::new(""));
        let target = normalize_path(&from_dir.join(raw));
        if self.known_files.contains(&target) {
            return Some(target);
        }
        let from_root = normalize_path(Path::new(raw));
        self.known_files.contains(&from_root).then_some(from_root)
    }

    /// Lua `require`. Path-style specifiers (`./util`) resolve relative to
    /// the requiring file; module-style specifiers map dots to `/`
    /// (`src.utils` -> `src/utils`) and are tried from the project root
    /// (conventional package.path), then the requiring file's dir. Each
    /// candidate is tried as `{path}.lua` and `{path}/init.lua`.
    fn resolve_lua(&self, raw: &str, from: &Path) -> Option<PathBuf> {
        let from_dir = from.parent().unwrap_or(Path::new(""));

        if raw.starts_with("./") || raw.starts_with("../") {
            let target = normalize_path(&from_dir.join(raw));
            return self.try_lua_candidates(&target);
        }
        if raw.starts_with('/') {
            return None;
        }

        let module_path = raw.replace('.', "/");
        if let Some(hit) = self.try_lua_candidates(Path::new(&module_path)) {
            return Some(hit);
        }
        self.try_lua_candidates(&normalize_path(&from_dir.join(&module_path)))
    }

    fn try_lua_candidates(&self, target: &Path) -> Option<PathBuf> {
        let mut as_file = target.as_os_str().to_os_string();
        as_file.push(".lua");
        let as_file = PathBuf::from(as_file);
        if self.known_files.contains(&as_file) {
            return Some(as_file);
        }
        let as_init = target.join("init.lua");
        if self.known_files.contains(&as_init) {
            return Some(as_init);
        }
        // Literal path that already carries the extension: require("./util.lua")
        if target.extension().and_then(|e| e.to_str()) == Some("lua")
            && self.known_files.contains(target)
        {
            return Some(target.to_path_buf());
        }
        None
    }

    /// Bash `source ./lib.sh` / `. lib.sh`: relative paths resolve against
    /// the sourcing script's dir. Absolute paths (`/etc/profile`) stay
    /// unresolved.
    fn resolve_bash(&self, raw: &str, from: &Path) -> Option<PathBuf> {
        if raw.starts_with('/') {
            return None;
        }
        let from_dir = from.parent().unwrap_or(Path::new(""));
        let target = normalize_path(&from_dir.join(raw));
        self.known_files.contains(&target).then_some(target)
    }

    /// Zig `@import("utils.zig")` / `@import("sub/mod.zig")`: file imports
    /// (path ends in `.zig` or contains `/`) resolve relative to the
    /// importing file's dir — idiomatic Zig omits the ./ prefix. Bare
    /// package imports (`@import("std")`, build.zig.zon package names) are
    /// never file paths and stay unresolved.
    fn resolve_zig(&self, raw: &str, from: &Path) -> Option<PathBuf> {
        if raw.starts_with('/') {
            return None;
        }
        let has_zig_ext = Path::new(raw)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("zig"));
        if !has_zig_ext && !raw.contains('/') {
            return None;
        }
        let from_dir = from.parent().unwrap_or(Path::new(""));
        let target = normalize_path(&from_dir.join(raw));
        self.known_files.contains(&target).then_some(target)
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

/// Read the module path from `<root>/go.mod` (the `module <path>` directive),
/// if present. Handles trailing comments and the rare quoted form.
fn load_go_module(root: &Path) -> Option<String> {
    let content = std::fs::read_to_string(root.join("go.mod")).ok()?;
    for line in content.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("module") else {
            continue;
        };
        // Require a whitespace boundary so e.g. `modules` doesn't match.
        if !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let module = rest
            .split("//")
            .next()
            .unwrap_or(rest)
            .trim()
            .trim_matches('"');
        if !module.is_empty() {
            tracing::info!("Loaded go.mod module path: {}", module);
            return Some(module.to_string());
        }
    }
    None
}

/// Load tsconfig.json paths aliases if present. All targets of each alias
/// are kept (not just the first), and each target is anchored to
/// `compilerOptions.baseUrl` when set — baseUrl is relative to the tsconfig
/// location, which is the project root per the loading behavior here.
fn load_tsconfig_paths(root: &Path) -> Vec<TsPathAlias> {
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
        let base_url = compiler_options
            .get("baseUrl")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if let Some(path_map) = compiler_options.get("paths") {
            if let Some(obj) = path_map.as_object() {
                for (key, value) in obj {
                    if let Some(targets) = value.as_array() {
                        let targets: Vec<String> = targets
                            .iter()
                            .filter_map(|t| t.as_str())
                            .map(|t| anchor_tsconfig_target(base_url, t))
                            .collect();
                        if !targets.is_empty() {
                            paths.push(TsPathAlias {
                                pattern: key.clone(),
                                targets,
                            });
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

/// Anchor a `paths` target under `baseUrl` (both may carry `./` segments,
/// which the later `normalize_path` of the substituted candidate cleans up).
fn anchor_tsconfig_target(base_url: &str, target: &str) -> String {
    if base_url.is_empty() {
        return target.to_string();
    }
    Path::new(base_url)
        .join(target)
        .to_string_lossy()
        .into_owned()
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

    fn resolve_go(files: &[&str], raw: &str, from: &str) -> Option<PathBuf> {
        resolver(files).resolve_go(raw, Path::new(from))
    }

    const TS_EXTS: &[&str] = &["ts", "tsx", "js", "jsx"];

    fn resolve_ts(files: &[&str], raw: &str, from: &str) -> Option<PathBuf> {
        resolver(files).resolve_js_ts(raw, Path::new(from), TS_EXTS)
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
    fn mod_declaration_prefers_child_module_dir_over_sibling() {
        // `mod config;` in src/commands.rs with BOTH src/config.rs and
        // src/commands/config.rs present — rustc only accepts the child.
        let got = resolve_rust(
            &["src/commands.rs", "src/config.rs", "src/commands/config.rs"],
            "config",
            "src/commands.rs",
        );
        assert_eq!(got, Some(PathBuf::from("src/commands/config.rs")));
    }

    #[test]
    fn mod_declaration_prefers_child_mod_rs_over_sibling() {
        // Same collision, with the child shaped as config/mod.rs.
        let got = resolve_rust(
            &[
                "src/commands.rs",
                "src/config.rs",
                "src/commands/config/mod.rs",
            ],
            "config",
            "src/commands.rs",
        );
        assert_eq!(got, Some(PathBuf::from("src/commands/config/mod.rs")));
    }

    #[test]
    fn mod_declaration_falls_back_to_sibling_without_child() {
        // No collision: only the lenient sibling exists, so it still resolves.
        let got = resolve_rust(
            &["src/commands.rs", "src/config.rs"],
            "config",
            "src/commands.rs",
        );
        assert_eq!(got, Some(PathBuf::from("src/config.rs")));
    }

    #[test]
    fn mod_declaration_from_mod_rs_resolves_own_dir_amid_collision() {
        // From mod.rs the module dir IS the file's own dir; a same-named
        // module one level up must not interfere.
        let got = resolve_rust(
            &[
                "src/commands/mod.rs",
                "src/commands/config.rs",
                "src/config.rs",
            ],
            "config",
            "src/commands/mod.rs",
        );
        assert_eq!(got, Some(PathBuf::from("src/commands/config.rs")));
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
    fn rust_glob_imports_intentionally_resolve_to_none() {
        let super_glob = resolve_rust(
            &["src/a/mod.rs", "src/a/b.rs", "src/a/c.rs"],
            "super::*",
            "src/a/b.rs",
        );
        assert_eq!(super_glob, None);

        let crate_glob = resolve_rust(
            &["src/lib.rs", "src/foo.rs", "src/foo/bar.rs"],
            "crate::foo::*",
            "src/lib.rs",
        );
        assert_eq!(crate_glob, None);
    }

    #[test]
    fn go_package_resolution_picks_lexicographically_first_member() {
        let got = resolve_go(
            &[
                "pkg/service/zeta.go",
                "pkg/service/alpha.go",
                "pkg/service/internal/helper.go",
            ],
            "./service",
            "pkg/main.go",
        );
        assert_eq!(got, Some(PathBuf::from("pkg/service/alpha.go")));
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

    #[test]
    fn dotted_specifier_appends_extension_instead_of_truncating() {
        // The headline bug: `./user.service` must resolve to user.service.ts,
        // never truncate at the last dot and draw an edge to user.ts.
        let got = resolve_ts(
            &["src/app.ts", "src/user.service.ts", "src/user.ts"],
            "./user.service",
            "src/app.ts",
        );
        assert_eq!(got, Some(PathBuf::from("src/user.service.ts")));
    }

    #[test]
    fn dotted_specifier_without_match_does_not_fall_back_to_truncated_name() {
        // user.service.ts does not exist; resolution must NOT truncate to
        // user.ts. "service" is not a js-family extension, so no substitution.
        let got = resolve_ts(
            &["src/app.ts", "src/user.ts"],
            "./user.service",
            "src/app.ts",
        );
        assert_eq!(got, None);
    }

    #[test]
    fn nodenext_js_specifier_substitutes_to_ts() {
        let got = resolve_ts(&["src/app.ts", "src/foo.ts"], "./foo.js", "src/app.ts");
        assert_eq!(got, Some(PathBuf::from("src/foo.ts")));
    }

    #[test]
    fn nodenext_dotted_js_specifier_resolves_to_dotted_ts() {
        // NodeNext + Angular style combined: `./user.service.js` substitutes
        // only the trailing js-family extension.
        let got = resolve_ts(
            &["src/app.ts", "src/user.service.ts"],
            "./user.service.js",
            "src/app.ts",
        );
        assert_eq!(got, Some(PathBuf::from("src/user.service.ts")));
    }

    /// Build a `TsPathAlias` for field-injection tests.
    fn alias(pattern: &str, targets: &[&str]) -> TsPathAlias {
        TsPathAlias {
            pattern: pattern.to_string(),
            targets: targets.iter().map(|t| (*t).to_string()).collect(),
        }
    }

    #[test]
    fn tsconfig_path_resolves_dotted_specifier() {
        let mut r = resolver(&["src/lib/user.service.ts", "src/lib/user.ts"]);
        r.tsconfig_paths = vec![alias("@lib/*", &["src/lib/*"])];
        let got = r.resolve_tsconfig_path("@lib/user.service", TS_EXTS);
        assert_eq!(got, Some(PathBuf::from("src/lib/user.service.ts")));
    }

    #[test]
    fn tsconfig_path_keeps_nodenext_substitution() {
        let mut r = resolver(&["src/lib/foo.ts"]);
        r.tsconfig_paths = vec![alias("@lib/*", &["src/lib/*"])];
        let got = r.resolve_tsconfig_path("@lib/foo.js", TS_EXTS);
        assert_eq!(got, Some(PathBuf::from("src/lib/foo.ts")));
    }

    #[test]
    fn tsconfig_exact_alias_matches_only_exact_specifier() {
        // A non-wildcard `jquery` alias must NOT prefix-match `jquery-ui`:
        // the external package stays unresolved (no phantom local edge).
        let mut r = resolver(&["vendor/jquery.ts"]);
        r.tsconfig_paths = vec![alias("jquery", &["vendor/jquery.ts"])];
        assert_eq!(
            r.resolve_tsconfig_path("jquery", TS_EXTS),
            Some(PathBuf::from("vendor/jquery.ts"))
        );
        assert_eq!(r.resolve_tsconfig_path("jquery-ui", TS_EXTS), None);
        assert_eq!(r.resolve_tsconfig_path("jquery/dist", TS_EXTS), None);
    }

    #[test]
    fn tsconfig_longest_wildcard_prefix_wins() {
        // Both candidate files exist, so first-match (BTreeMap/declared
        // order) would wrongly pick `@app/*`; TypeScript picks the pattern
        // with the longest matched prefix — `@app/core/*`.
        let mut r = resolver(&["src/app/core/util.ts", "src/core/util.ts"]);
        r.tsconfig_paths = vec![
            alias("@app/*", &["src/app/*"]),
            alias("@app/core/*", &["src/core/*"]),
        ];
        let got = r.resolve_tsconfig_path("@app/core/util", TS_EXTS);
        assert_eq!(got, Some(PathBuf::from("src/core/util.ts")));
    }

    #[test]
    fn tsconfig_alias_tries_all_targets_in_order() {
        // The first target misses; the second must still be tried.
        let mut r = resolver(&["src/shared/util.ts"]);
        r.tsconfig_paths = vec![alias("@shared/*", &["src/missing/*", "src/shared/*"])];
        let got = r.resolve_tsconfig_path("@shared/util", TS_EXTS);
        assert_eq!(got, Some(PathBuf::from("src/shared/util.ts")));
    }

    /// Unique on-disk root containing a synthetic tsconfig.json (same
    /// precedent as `go_mod_root` below).
    fn tsconfig_root(tag: &str, content: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kgr-resolve-ts-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tsconfig.json"), content).unwrap();
        dir
    }

    #[test]
    fn tsconfig_targets_anchor_to_base_url() {
        // With baseUrl "src", target "lib/*" must resolve under src/lib/.
        let root = tsconfig_root(
            "baseurl",
            r#"{"compilerOptions": {"baseUrl": "src", "paths": {"@lib/*": ["lib/*"]}}}"#,
        );
        let nodes: Vec<FileNode> = vec![node("src/lib/util.ts")];
        let r = Resolver::new(root, &nodes);
        let got = r.resolve_tsconfig_path("@lib/util", TS_EXTS);
        assert_eq!(got, Some(PathBuf::from("src/lib/util.ts")));
    }

    #[test]
    fn tsconfig_dotted_base_url_is_normalized_away() {
        // baseUrl "./" keeps the existing root-relative target behavior.
        let root = tsconfig_root(
            "baseurl-dot",
            r#"{"compilerOptions": {"baseUrl": "./", "paths": {"@lib/*": ["src/lib/*"]}}}"#,
        );
        let nodes: Vec<FileNode> = vec![node("src/lib/util.ts")];
        let r = Resolver::new(root, &nodes);
        let got = r.resolve_tsconfig_path("@lib/util", TS_EXTS);
        assert_eq!(got, Some(PathBuf::from("src/lib/util.ts")));
    }

    #[test]
    fn tsconfig_loader_keeps_all_targets() {
        // The loader must not drop targets after the first.
        let root = tsconfig_root(
            "multi-target",
            r#"{"compilerOptions": {"paths": {"@x/*": ["a/*", "b/*"]}}}"#,
        );
        let loaded = load_tsconfig_paths(&root);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].pattern, "@x/*");
        assert_eq!(loaded[0].targets, ["a/*", "b/*"]);
    }

    #[test]
    fn directory_import_still_resolves_to_index() {
        let got = resolve_ts(
            &["src/app.ts", "src/utils/index.ts"],
            "./utils",
            "src/app.ts",
        );
        assert_eq!(got, Some(PathBuf::from("src/utils/index.ts")));
    }

    // ── Ruby ─────────────────────────────────────────────────────────────

    fn resolve_ruby(files: &[&str], raw: &str, from: &str) -> Option<PathBuf> {
        resolver(files).resolve_ruby(raw, Path::new(from))
    }

    #[test]
    fn ruby_require_relative_resolves_sibling() {
        // The parser emits the bare name for `require_relative 'helper'`.
        let got = resolve_ruby(&["app/main.rb", "app/helper.rb"], "helper", "app/main.rb");
        assert_eq!(got, Some(PathBuf::from("app/helper.rb")));
    }

    #[test]
    fn ruby_relative_path_with_parent_dir_resolves() {
        let got = resolve_ruby(
            &["app/main.rb", "lib/util.rb"],
            "../lib/util",
            "app/main.rb",
        );
        assert_eq!(got, Some(PathBuf::from("lib/util.rb")));
    }

    #[test]
    fn ruby_existing_rb_extension_is_not_doubled() {
        let got = resolve_ruby(
            &["app/main.rb", "app/helper.rb"],
            "helper.rb",
            "app/main.rb",
        );
        assert_eq!(got, Some(PathBuf::from("app/helper.rb")));
    }

    #[test]
    fn ruby_external_gem_stays_unresolved() {
        let got = resolve_ruby(&["app/main.rb"], "json", "app/main.rb");
        assert_eq!(got, None);
    }

    // ── PHP ──────────────────────────────────────────────────────────────

    fn resolve_php(files: &[&str], raw: &str, from: &str) -> Option<PathBuf> {
        resolver(files).resolve_php(raw, Path::new(from))
    }

    #[test]
    fn php_relative_include_resolves_sibling() {
        let got = resolve_php(
            &["web/index.php", "web/inc.php"],
            "./inc.php",
            "web/index.php",
        );
        assert_eq!(got, Some(PathBuf::from("web/inc.php")));
    }

    #[test]
    fn php_parent_relative_include_resolves() {
        let got = resolve_php(
            &["web/index.php", "lib/db.php"],
            "../lib/db.php",
            "web/index.php",
        );
        assert_eq!(got, Some(PathBuf::from("lib/db.php")));
    }

    #[test]
    fn php_bare_path_falls_back_to_project_root() {
        let got = resolve_php(
            &["web/index.php", "vendor/autoload.php"],
            "vendor/autoload.php",
            "web/index.php",
        );
        assert_eq!(got, Some(PathBuf::from("vendor/autoload.php")));
    }

    #[test]
    fn php_namespace_use_is_never_a_file_path() {
        let got = resolve_php(
            &["web/index.php", "App/Models/User.php"],
            "App\\Models\\User",
            "web/index.php",
        );
        assert_eq!(got, None);
    }

    #[test]
    fn php_missing_target_stays_unresolved() {
        let got = resolve_php(&["web/index.php"], "./inc.php", "web/index.php");
        assert_eq!(got, None);
    }

    // ── Lua ──────────────────────────────────────────────────────────────

    fn resolve_lua(files: &[&str], raw: &str, from: &str) -> Option<PathBuf> {
        resolver(files).resolve_lua(raw, Path::new(from))
    }

    #[test]
    fn lua_dotted_module_resolves_to_file_from_root() {
        let got = resolve_lua(&["main.lua", "src/utils.lua"], "src.utils", "main.lua");
        assert_eq!(got, Some(PathBuf::from("src/utils.lua")));
    }

    #[test]
    fn lua_dotted_module_resolves_to_init_lua() {
        let got = resolve_lua(&["main.lua", "src/utils/init.lua"], "src.utils", "main.lua");
        assert_eq!(got, Some(PathBuf::from("src/utils/init.lua")));
    }

    #[test]
    fn lua_relative_path_require_resolves_against_requiring_file() {
        let got = resolve_lua(&["src/init.lua", "src/util.lua"], "./util", "src/init.lua");
        assert_eq!(got, Some(PathBuf::from("src/util.lua")));
    }

    #[test]
    fn lua_module_falls_back_to_requiring_files_dir() {
        // Root-based lookup misses; the script-dir-extended package.path hits.
        let got = resolve_lua(
            &["src/app/main.lua", "src/app/helpers/text.lua"],
            "helpers.text",
            "src/app/main.lua",
        );
        assert_eq!(got, Some(PathBuf::from("src/app/helpers/text.lua")));
    }

    #[test]
    fn lua_external_module_stays_unresolved() {
        let got = resolve_lua(&["main.lua"], "socket.http", "main.lua");
        assert_eq!(got, None);
    }

    // ── Bash ─────────────────────────────────────────────────────────────

    fn resolve_bash(files: &[&str], raw: &str, from: &str) -> Option<PathBuf> {
        resolver(files).resolve_bash(raw, Path::new(from))
    }

    #[test]
    fn bash_source_dot_slash_resolves_sibling() {
        let got = resolve_bash(
            &["scripts/main.sh", "scripts/lib.sh"],
            "./lib.sh",
            "scripts/main.sh",
        );
        assert_eq!(got, Some(PathBuf::from("scripts/lib.sh")));
    }

    #[test]
    fn bash_bare_relative_path_resolves_against_script_dir() {
        let got = resolve_bash(
            &["scripts/main.sh", "scripts/lib.sh"],
            "lib.sh",
            "scripts/main.sh",
        );
        assert_eq!(got, Some(PathBuf::from("scripts/lib.sh")));
    }

    #[test]
    fn bash_parent_relative_path_resolves() {
        let got = resolve_bash(
            &["scripts/deploy/run.sh", "scripts/common/env.sh"],
            "../common/env.sh",
            "scripts/deploy/run.sh",
        );
        assert_eq!(got, Some(PathBuf::from("scripts/common/env.sh")));
    }

    #[test]
    fn bash_absolute_path_stays_unresolved() {
        let got = resolve_bash(&["scripts/main.sh"], "/etc/profile", "scripts/main.sh");
        assert_eq!(got, None);
    }

    // ── Zig ──────────────────────────────────────────────────────────────

    fn resolve_zig(files: &[&str], raw: &str, from: &str) -> Option<PathBuf> {
        resolver(files).resolve_zig(raw, Path::new(from))
    }

    #[test]
    fn zig_sibling_import_without_prefix_resolves() {
        // Idiomatic Zig omits the ./ prefix on sibling file imports.
        let got = resolve_zig(
            &["src/main.zig", "src/utils.zig"],
            "utils.zig",
            "src/main.zig",
        );
        assert_eq!(got, Some(PathBuf::from("src/utils.zig")));
    }

    #[test]
    fn zig_subdir_import_resolves_relative_to_importing_file() {
        let got = resolve_zig(
            &["src/main.zig", "src/sub/mod.zig"],
            "sub/mod.zig",
            "src/main.zig",
        );
        assert_eq!(got, Some(PathBuf::from("src/sub/mod.zig")));
    }

    #[test]
    fn zig_dot_slash_and_parent_relative_imports_resolve() {
        let dot_slash = resolve_zig(
            &["src/main.zig", "src/utils.zig"],
            "./utils.zig",
            "src/main.zig",
        );
        assert_eq!(dot_slash, Some(PathBuf::from("src/utils.zig")));

        let parent = resolve_zig(
            &["src/app/main.zig", "src/lib.zig"],
            "../lib.zig",
            "src/app/main.zig",
        );
        assert_eq!(parent, Some(PathBuf::from("src/lib.zig")));
    }

    #[test]
    fn zig_std_and_bare_package_names_stay_unresolved() {
        let std_import = resolve_zig(&["src/main.zig"], "std", "src/main.zig");
        assert_eq!(std_import, None);

        // A bare package name never resolves to a file, even when a file
        // with a matching stem exists.
        let pkg = resolve_zig(&["src/main.zig", "src/zap.zig"], "zap", "src/main.zig");
        assert_eq!(pkg, None);
    }

    #[test]
    fn zig_missing_target_stays_unresolved() {
        let got = resolve_zig(&["src/main.zig"], "utils.zig", "src/main.zig");
        assert_eq!(got, None);
    }

    #[test]
    fn zig_full_resolve_two_file_scenario_without_prefix() {
        // End-to-end through resolve_all: the idiomatic unprefixed sibling
        // import produces a resolved Local import.
        let r = resolver(&["src/main.zig", "src/utils.zig"]);
        let mut files = vec![FileNode {
            path: PathBuf::from("src/main.zig"),
            lang: Lang::Zig,
            imports: vec![Import {
                raw: "utils.zig".to_string(),
                kind: ImportKind::Local,
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
            Some(PathBuf::from("src/utils.zig"))
        );
    }

    // ── Go modules ───────────────────────────────────────────────────────

    /// Resolver with an injected go.mod module path (same precedent as the
    /// tsconfig_paths field-injection tests above).
    fn go_resolver(files: &[&str], module: &str) -> Resolver {
        let mut r = resolver(files);
        r.go_module = Some(module.to_string());
        r
    }

    /// Unique on-disk root containing a synthetic go.mod.
    fn go_mod_root(tag: &str, module: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kgr-resolve-go-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("go.mod"), format!("module {module}\n\ngo 1.22\n")).unwrap();
        dir
    }

    #[test]
    fn go_module_import_resolves_to_package_dir() {
        let r = go_resolver(
            &["main.go", "internal/x/beta.go", "internal/x/alpha.go"],
            "github.com/user/repo",
        );
        // Deterministic: lexicographically first .go file in the package dir.
        let got = r.resolve_go("github.com/user/repo/internal/x", Path::new("main.go"));
        assert_eq!(got, Some(PathBuf::from("internal/x/alpha.go")));
    }

    #[test]
    fn go_module_import_does_not_descend_into_nested_packages() {
        // pkg/a has no direct .go children — only the nested pkg/a/sub
        // package. Nested dirs are different Go packages; no match.
        let r = go_resolver(&["main.go", "pkg/a/sub/deep.go"], "github.com/user/repo");
        let got = r.resolve_go("github.com/user/repo/pkg/a", Path::new("main.go"));
        assert_eq!(got, None);
    }

    #[test]
    fn go_module_root_import_resolves_to_root_package() {
        let r = go_resolver(
            &["main.go", "app.go", "internal/x/x.go"],
            "github.com/user/repo",
        );
        let got = r.resolve_go("github.com/user/repo", Path::new("internal/x/x.go"));
        assert_eq!(got, Some(PathBuf::from("app.go")));
    }

    #[test]
    fn go_module_prefix_requires_path_boundary() {
        // github.com/user/repofork shares a string prefix with the module
        // path but is a different module entirely.
        let r = go_resolver(&["main.go", "pkg/x.go"], "github.com/user/repo");
        let got = r.resolve_go("github.com/user/repofork/pkg", Path::new("main.go"));
        assert_eq!(got, None);
    }

    #[test]
    fn go_stdlib_and_third_party_stay_unresolved() {
        let r = go_resolver(&["main.go", "fmt/fmt.go"], "github.com/user/repo");
        assert_eq!(r.resolve_go("fmt", Path::new("main.go")), None);
        assert_eq!(
            r.resolve_go("github.com/other/lib/pkg", Path::new("main.go")),
            None
        );
    }

    #[test]
    fn go_module_import_without_gomod_stays_unresolved() {
        // No go.mod (go_module is None): module-path imports never resolve.
        let r = resolver(&["main.go", "internal/x/x.go"]);
        let got = r.resolve_go("github.com/user/repo/internal/x", Path::new("main.go"));
        assert_eq!(got, None);
    }

    #[test]
    fn go_relative_import_still_resolves_without_gomod() {
        let got = resolve_go(
            &["pkg/main.go", "pkg/util/util.go"],
            "./util",
            "pkg/main.go",
        );
        assert_eq!(got, Some(PathBuf::from("pkg/util/util.go")));
    }

    #[test]
    fn go_mod_module_path_is_read_from_disk() {
        let root = go_mod_root("read", "github.com/user/repo");
        let nodes: Vec<FileNode> = ["cmd/app/main.go", "internal/x/x.go"]
            .iter()
            .map(|p| node(p))
            .collect();
        let r = Resolver::new(root.clone(), &nodes);
        std::fs::remove_dir_all(&root).ok();

        let got = r.resolve_go(
            "github.com/user/repo/internal/x",
            Path::new("cmd/app/main.go"),
        );
        assert_eq!(got, Some(PathBuf::from("internal/x/x.go")));
    }

    #[test]
    fn go_synthetic_repo_full_resolve_flips_module_import_to_local() {
        // Acceptance scenario: a synthetic repo with a go.mod module path and
        // two packages; the module-path import resolves and flips to Local,
        // while the stdlib import stays External and unresolved.
        let root = go_mod_root("e2e", "example.com/proj");
        let nodes: Vec<FileNode> = ["cmd/app/main.go", "internal/util/util.go"]
            .iter()
            .map(|p| node(p))
            .collect();
        let r = Resolver::new(root.clone(), &nodes);
        std::fs::remove_dir_all(&root).ok();

        let mut files = vec![FileNode {
            path: PathBuf::from("cmd/app/main.go"),
            lang: Lang::Go,
            imports: vec![
                Import {
                    raw: "example.com/proj/internal/util".to_string(),
                    kind: ImportKind::External,
                    resolved: None,
                    span: None,
                },
                Import {
                    raw: "fmt".to_string(),
                    kind: ImportKind::External,
                    resolved: None,
                    span: None,
                },
            ],
            symbols: Vec::new(),
            calls: Vec::new(),
        }];
        r.resolve_all(&mut files);

        assert_eq!(files[0].imports[0].kind, ImportKind::Local);
        assert_eq!(
            files[0].imports[0].resolved,
            Some(PathBuf::from("internal/util/util.go"))
        );
        assert_eq!(files[0].imports[1].kind, ImportKind::External);
        assert_eq!(files[0].imports[1].resolved, None);
    }

    // ── Dispatch wiring for the four new arms ────────────────────────────

    #[test]
    fn resolve_dispatches_new_language_arms() {
        let r = resolver(&[
            "app/main.rb",
            "app/helper.rb",
            "web/index.php",
            "web/inc.php",
            "main.lua",
            "src/utils.lua",
            "scripts/main.sh",
            "scripts/lib.sh",
        ]);
        assert_eq!(
            r.resolve(
                "helper",
                Path::new("app/main.rb"),
                Lang::Ruby,
                ImportKind::External
            ),
            Some(PathBuf::from("app/helper.rb"))
        );
        assert_eq!(
            r.resolve(
                "./inc.php",
                Path::new("web/index.php"),
                Lang::Php,
                ImportKind::Local
            ),
            Some(PathBuf::from("web/inc.php"))
        );
        assert_eq!(
            r.resolve(
                "src.utils",
                Path::new("main.lua"),
                Lang::Lua,
                ImportKind::External
            ),
            Some(PathBuf::from("src/utils.lua"))
        );
        assert_eq!(
            r.resolve(
                "./lib.sh",
                Path::new("scripts/main.sh"),
                Lang::Bash,
                ImportKind::Local
            ),
            Some(PathBuf::from("scripts/lib.sh"))
        );
    }

    // ── C / C++ includes ─────────────────────────────────────────────────

    fn resolve_c_kind(files: &[&str], raw: &str, from: &str, kind: ImportKind) -> Option<PathBuf> {
        resolver(files).resolve_c(raw, Path::new(from), kind)
    }

    #[test]
    fn c_angle_include_ignores_shadowing_sibling_header() {
        // The headline bug: `#include <string.h>` in src/main.c with an
        // unrelated src/string.h present. Angle includes never search the
        // includer's directory — no phantom edge.
        let got = resolve_c_kind(
            &["src/main.c", "src/string.h"],
            "string.h",
            "src/main.c",
            ImportKind::System,
        );
        assert_eq!(got, None);
    }

    #[test]
    fn c_quoted_include_resolves_relative_to_includer() {
        // Same file layout, quote form: `#include "string.h"` IS the
        // sibling header.
        let got = resolve_c_kind(
            &["src/main.c", "src/string.h"],
            "string.h",
            "src/main.c",
            ImportKind::Local,
        );
        assert_eq!(got, Some(PathBuf::from("src/string.h")));
    }

    #[test]
    fn c_angle_include_skips_root_and_src_candidates() {
        // Shadowing headers at the project root or under src/ must not
        // match the angle form either — only public include roots may.
        let root_shadow = resolve_c_kind(
            &["main.c", "assert.h"],
            "assert.h",
            "main.c",
            ImportKind::System,
        );
        assert_eq!(root_shadow, None);

        let src_shadow = resolve_c_kind(
            &["app/main.c", "src/util.h"],
            "util.h",
            "app/main.c",
            ImportKind::System,
        );
        assert_eq!(src_shadow, None);
    }

    #[test]
    fn c_angle_include_still_resolves_under_public_include_dir() {
        // Angle form of a project's own header via the conventional
        // -Iinclude root keeps resolving.
        let got = resolve_c_kind(
            &["src/main.c", "include/mylib/api.h"],
            "mylib/api.h",
            "src/main.c",
            ImportKind::System,
        );
        assert_eq!(got, Some(PathBuf::from("include/mylib/api.h")));
    }

    #[test]
    fn c_full_resolve_angle_stays_system_quote_stays_local() {
        // End-to-end acceptance through resolve_all: `<string.h>` with a
        // shadowing same-dir string.h stays System with resolved=null,
        // while `"local.h"` still resolves relative to the includer.
        let r = resolver(&["src/main.c", "src/string.h", "src/local.h"]);
        let mut files = vec![FileNode {
            path: PathBuf::from("src/main.c"),
            lang: Lang::C,
            imports: vec![
                Import {
                    raw: "string.h".to_string(),
                    kind: ImportKind::System,
                    resolved: None,
                    span: None,
                },
                Import {
                    raw: "local.h".to_string(),
                    kind: ImportKind::Local,
                    resolved: None,
                    span: None,
                },
            ],
            symbols: Vec::new(),
            calls: Vec::new(),
        }];
        r.resolve_all(&mut files);

        assert_eq!(files[0].imports[0].kind, ImportKind::System);
        assert_eq!(files[0].imports[0].resolved, None);
        assert_eq!(files[0].imports[1].kind, ImportKind::Local);
        assert_eq!(
            files[0].imports[1].resolved,
            Some(PathBuf::from("src/local.h"))
        );
    }

    #[test]
    fn cpp_full_resolve_resolved_angle_include_keeps_system_kind() {
        // C++ routes through the same arm. An angle include that genuinely
        // hits the public include root resolves, but the System
        // classification is never rewritten to Local.
        let r = resolver(&["src/main.cpp", "include/mylib/api.hpp"]);
        let mut files = vec![FileNode {
            path: PathBuf::from("src/main.cpp"),
            lang: Lang::Cpp,
            imports: vec![Import {
                raw: "mylib/api.hpp".to_string(),
                kind: ImportKind::System,
                resolved: None,
                span: None,
            }],
            symbols: Vec::new(),
            calls: Vec::new(),
        }];
        r.resolve_all(&mut files);

        assert_eq!(files[0].imports[0].kind, ImportKind::System);
        assert_eq!(
            files[0].imports[0].resolved,
            Some(PathBuf::from("include/mylib/api.hpp"))
        );
    }
}
