use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use crate::types::{FileNode, ImportKind, Lang};

const TYPESCRIPT_RESOLVE_EXTS: &[&str] = &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];
const JAVASCRIPT_RESOLVE_EXTS: &[&str] = &["js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts"];

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
    rust_declared_modules: HashSet<PathBuf>,
    /// `crate::` source base per Rust source directory, precomputed in `new`
    /// (the Cargo.toml probes hit the filesystem; see `derive_crate_src_base`).
    rust_crate_bases: HashMap<PathBuf, PathBuf>,
    tsconfig_paths: Vec<TsPathAlias>,
    go_module: Option<String>,
    /// Owned copy of the scan root, kept for on-the-fly crate-base derivation
    /// when `crate_src_base` is asked about a file outside the scanned set.
    root: PathBuf,
}

impl Resolver {
    /// Build a resolver for the files discovered under `root` (the scan
    /// root). `tsconfig.json` and `go.mod` are loaded from `root`, NOT the
    /// process CWD, so scanning a directory other than the CWD picks up that
    /// directory's path aliases and module path.
    pub fn new(root: &Path, files: &[FileNode]) -> Self {
        let known_files: HashSet<PathBuf> = files.iter().map(|f| f.path.clone()).collect();
        let rust_declared_modules = collect_rust_declared_modules(&known_files, files);
        let rust_crate_bases = compute_rust_crate_bases(root, files);
        let tsconfig_paths = load_tsconfig_paths(root);
        let go_module = load_go_module(root);
        Self {
            known_files,
            rust_declared_modules,
            rust_crate_bases,
            tsconfig_paths,
            go_module,
            root: root.to_path_buf(),
        }
    }

    pub fn resolve_all(&self, files: &mut [FileNode]) {
        for file in files.iter_mut() {
            let lang = file.lang;
            let file_path = file.path.clone();
            for import in file.imports.iter_mut() {
                import.resolved = self.resolve(&import.raw, &file_path, lang, import.kind);
                if lang == Lang::Rust && import.resolved.as_deref() == Some(file_path.as_path()) {
                    // Rust same-file reference: `use self::Item;`, a test-module
                    // `use super::*;` (rebased to `self::*` by the parser), or a
                    // crate-absolute path to the importing file's own module. It
                    // names items in this very file, so there is no dependency
                    // edge and it is never an external package — keep it Local
                    // and unresolved instead of surfacing it as External.
                    import.resolved = None;
                    import.kind = ImportKind::Local;
                    continue;
                }
                if import.resolved.is_some() {
                    // A resolved import is upgraded to Local — EXCEPT when the
                    // parser classified it as System (C/C++/ObjC angle
                    // includes): a project header that happens to share a system
                    // header's name must not erase the System classification.
                    if import.kind != ImportKind::System {
                        import.kind = ImportKind::Local;
                    }
                } else if import.kind == ImportKind::Local {
                    // Unresolved locals would otherwise create neither a graph
                    // edge nor an external_deps entry. Keep resolved=None, but
                    // surface the raw specifier through the existing External
                    // projection. Resolver-less languages that emit Local
                    // imports follow the same documented behavior.
                    import.kind = ImportKind::External;
                }
            }
        }
    }

    fn resolve(&self, raw: &str, from: &Path, lang: Lang, kind: ImportKind) -> Option<PathBuf> {
        match lang {
            Lang::Python => self.resolve_python(raw, from),
            Lang::TypeScript => self.resolve_js_ts(raw, from, TYPESCRIPT_RESOLVE_EXTS),
            Lang::JavaScript => self.resolve_js_ts(raw, from, JAVASCRIPT_RESOLVE_EXTS),
            Lang::Java => self.resolve_java(raw),
            Lang::C | Lang::Cpp => self.resolve_c(raw, from, kind),
            Lang::Rust => self.resolve_rust(raw, from, kind),
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

    fn resolve_rust(&self, raw: &str, from: &Path, kind: ImportKind) -> Option<PathBuf> {
        let from_dir = from.parent().unwrap_or(Path::new("")).to_path_buf();

        if is_rust_path_attribute(raw) {
            if kind != ImportKind::Local {
                return None;
            }
            let raw_path = raw.replace('\\', "/");
            let target = normalize_path(&from_dir.join(raw_path));
            return self.known_files.contains(&target).then_some(target);
        }

        // Keyword-only paths: `use crate as me;` and group-self members
        // (`use super::{self}` rebased to `self` by the parser) name a module
        // file directly. They must not fall through to the single-segment
        // branches below, which would look for a literal `crate.rs`.
        match raw {
            "crate" => return self.crate_root_file(from),
            // Same-file reference; resolve_all clears it without an edge.
            "self" => return Some(from.to_path_buf()),
            "super" => {
                let dir = module_dir(from)
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default();
                return self.module_owner_file(&dir);
            }
            _ => {}
        }

        // `mod foo;` — a submodule of the current file's module. From
        // mod.rs/lib.rs/main.rs it's a sibling (`foo.rs`); from `bar.rs` it
        // nests under `bar/`. The module dir must win: rustc only accepts
        // `bar/foo.rs` for `mod foo;` in `bar.rs`, so when both `bar/foo.rs`
        // and a sibling `foo.rs` exist, the child is the real target. The
        // sibling is kept only as a lenient fallback (e.g. `#[path]` quirks).
        // For mod.rs/lib.rs/main.rs the two candidates are the same dir.
        if !raw.contains("::") {
            if kind == ImportKind::Local {
                return resolve_rust_mod_declaration(&self.known_files, raw, from);
            }
            let resolved = self.try_module(&self.crate_src_base(from), &[raw])?;
            return self
                .rust_declared_modules
                .contains(&resolved)
                .then_some(resolved);
        }

        // Resolve the base directory and remaining path from the leading
        // qualifier. `crate::` is anchored at the owning crate's source base
        // (the nearest `src/` ancestor, else the nearest ancestor with a
        // Cargo.toml on disk — NOT a hardcoded `src/` off the scan root, so
        // workspaces and lib-at-root layouts resolve; see
        // `derive_crate_src_base`); `self::`/`super::` are relative to the
        // current module.
        //
        // Each qualifier also carries an ANCHOR file: the module file the
        // qualifier itself names. When the remaining segments never match a
        // module FILE, the path names an item defined in — or re-exported
        // by — that anchor module (`use crate::Config;` -> lib.rs barrel,
        // `use super::helper;` -> the parent's mod.rs), so the anchor is the
        // real file-level dependency. Bare 2018-edition paths get no anchor:
        // an unresolved bare path is an external crate, not the crate root.
        let (base, rest, require_declared_module, anchor): (PathBuf, &str, bool, Option<PathBuf>) =
            if let Some(r) = raw.strip_prefix("crate::") {
                (
                    self.crate_src_base(from),
                    r,
                    false,
                    self.crate_root_file(from),
                )
            } else if let Some(r) = raw.strip_prefix("self::") {
                // Self-anchor = the importing file; resolve_all clears it.
                (module_dir(from), r, false, Some(from.to_path_buf()))
            } else if raw.starts_with("super::") {
                let mut dir = module_dir(from);
                let mut r = raw;
                while let Some(stripped) = r.strip_prefix("super::") {
                    dir = dir.parent().map(Path::to_path_buf).unwrap_or_default();
                    r = stripped;
                }
                let anchor = self.module_owner_file(&dir);
                (dir, r, false, anchor)
            } else {
                // Bare 2018-edition path import of a crate-local module, e.g.
                // `use cli::Foo;` at the crate root. A file-name collision with
                // an external crate is not enough: the shortened module target
                // must also come from a parsed `mod` declaration.
                (self.crate_src_base(from), raw, true, None)
            };

        // A trailing glob re-exports the named module's items: the dependency
        // is the module itself (`pub use crate::models::*;` -> models). Globs
        // anywhere else are not valid Rust.
        let mut segments: Vec<&str> = rest.split("::").filter(|s| !s.is_empty()).collect();
        if segments.last() == Some(&"*") {
            segments.pop();
        }
        if segments.contains(&"*") {
            return None;
        }

        let resolved = self.try_module(&base, &segments).or(anchor)?;
        if require_declared_module && !self.rust_declared_modules.contains(&resolved) {
            return None;
        }
        Some(resolved)
    }

    /// The directory `crate::` paths resolve under for the crate owning
    /// `from` (see `derive_crate_src_base` for the heuristic). Precomputed
    /// per source directory in `new`; files outside the scanned set (possible
    /// only for direct programmatic calls) are derived on the fly.
    fn crate_src_base(&self, from: &Path) -> PathBuf {
        let dir = from.parent().unwrap_or(Path::new(""));
        if let Some(base) = self.rust_crate_bases.get(dir) {
            return base.clone();
        }
        derive_crate_src_base(&self.root, from, &mut HashMap::new())
    }

    /// The crate root file owning `from`: `lib.rs` (the module-defining root
    /// when both targets exist) or `main.rs` inside the crate's source base.
    fn crate_root_file(&self, from: &Path) -> Option<PathBuf> {
        let base = self.crate_src_base(from);
        for name in ["lib.rs", "main.rs"] {
            let candidate = base.join(name);
            if self.known_files.contains(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    /// The file that DEFINES the module whose children live in `dir`:
    /// `dir/mod.rs`, the modern sibling `<dir>.rs`, or — when `dir` is a
    /// crate source root — `lib.rs`/`main.rs`.
    fn module_owner_file(&self, dir: &Path) -> Option<PathBuf> {
        let as_mod = dir.join("mod.rs");
        if self.known_files.contains(&as_mod) {
            return Some(as_mod);
        }
        if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
            let sibling = dir
                .parent()
                .unwrap_or(Path::new(""))
                .join(format!("{name}.rs"));
            if self.known_files.contains(&sibling) {
                return Some(sibling);
            }
        }
        for name in ["lib.rs", "main.rs"] {
            let candidate = dir.join(name);
            if self.known_files.contains(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    /// Resolve a `::`-separated module path under `base` to a file. The final
    /// segments of a `use` path may name an item (fn/type/const) rather than a
    /// module, so we shorten the path one segment at a time until it maps to a
    /// known `{path}.rs` or `{path}/mod.rs`.
    fn try_module(&self, base: &Path, segments: &[&str]) -> Option<PathBuf> {
        try_module_path(&self.known_files, base, segments)
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
                    known.parent().unwrap_or(Path::new("")) == target.as_path()
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

fn collect_rust_declared_modules(
    known_files: &HashSet<PathBuf>,
    files: &[FileNode],
) -> HashSet<PathBuf> {
    let mut declared = HashSet::new();
    for file in files.iter().filter(|f| f.lang == Lang::Rust) {
        for import in &file.imports {
            if import.kind != ImportKind::Local || import.raw.contains("::") {
                continue;
            }
            // Keyword-only use paths (`use crate as me;`) are Local without
            // `::` but are NOT mod declarations.
            if matches!(import.raw.as_str(), "crate" | "self" | "super") {
                continue;
            }
            if let Some(target) = resolve_rust_mod_declaration(known_files, &import.raw, &file.path)
            {
                declared.insert(target);
            }
        }
    }
    declared
}

fn resolve_rust_mod_declaration(
    known_files: &HashSet<PathBuf>,
    raw: &str,
    from: &Path,
) -> Option<PathBuf> {
    let from_dir = from.parent().unwrap_or(Path::new("")).to_path_buf();

    if is_rust_path_attribute(raw) {
        let raw_path = raw.replace('\\', "/");
        let target = normalize_path(&from_dir.join(raw_path));
        return known_files.contains(&target).then_some(target);
    }

    try_module_path(known_files, &module_dir(from), &[raw])
        .or_else(|| try_module_path(known_files, &from_dir, &[raw]))
}

fn try_module_path(
    known_files: &HashSet<PathBuf>,
    base: &Path,
    segments: &[&str],
) -> Option<PathBuf> {
    let mut segs = segments;
    while !segs.is_empty() {
        let joined = segs.join("/");
        let as_file = base.join(format!("{joined}.rs"));
        if known_files.contains(&as_file) {
            return Some(as_file);
        }
        let as_mod = base.join(&joined).join("mod.rs");
        if known_files.contains(&as_mod) {
            return Some(as_mod);
        }
        segs = &segs[..segs.len() - 1];
    }
    None
}

/// Precompute the `crate::` source base for every directory containing a
/// parsed Rust file. The Cargo.toml probes in `derive_crate_src_base` hit
/// the filesystem, so they run once per directory here (with a shared
/// manifest-probe memo) instead of once per resolved import.
fn compute_rust_crate_bases(root: &Path, files: &[FileNode]) -> HashMap<PathBuf, PathBuf> {
    let mut bases: HashMap<PathBuf, PathBuf> = HashMap::new();
    let mut has_manifest: HashMap<PathBuf, bool> = HashMap::new();
    for file in files.iter().filter(|f| f.lang == Lang::Rust) {
        let dir = file.path.parent().unwrap_or(Path::new("")).to_path_buf();
        bases
            .entry(dir)
            .or_insert_with(|| derive_crate_src_base(root, &file.path, &mut has_manifest));
    }
    bases
}

/// Derive the crate source base — the directory `crate::` paths resolve
/// under, i.e. the one holding the crate root file — for a scan-root-relative
/// file path. Heuristic, in priority order:
///
/// 1. The nearest ancestor directory literally named `src`: the standard
///    Cargo layout (`src/main.rs`, `crates/foo/src/lib.rs`). Pure path
///    matching, no filesystem access.
/// 2. Otherwise the nearest ancestor directory whose `Cargo.toml` exists on
///    disk (probed relative to the scan root, NOT the process CWD — the same
///    discipline as tsconfig.json/go.mod loading). This anchors lib-at-root
///    crates and other `[lib]/[[bin]] path = ...` layouts where modules sit
///    beside the manifest, including workspace members without a `src/` dir.
/// 3. Otherwise the scan root itself. This is a guess — `crate::` imports
///    from such a file usually resolve to nothing — so it is logged at debug
///    level to surface during dogfooding instead of failing silently.
fn derive_crate_src_base(
    root: &Path,
    from: &Path,
    has_manifest: &mut HashMap<PathBuf, bool>,
) -> PathBuf {
    let mut current = from.parent();
    while let Some(dir) = current {
        if dir.file_name().and_then(|n| n.to_str()) == Some("src") {
            return dir.to_path_buf();
        }
        current = dir.parent();
    }

    let mut current = from.parent();
    while let Some(dir) = current {
        let found = *has_manifest
            .entry(dir.to_path_buf())
            .or_insert_with(|| root.join(dir).join("Cargo.toml").is_file());
        if found {
            tracing::debug!(
                "no `src` ancestor for '{}'; anchoring crate:: at manifest dir '{}' (Cargo.toml)",
                from.display(),
                dir.display()
            );
            return dir.to_path_buf();
        }
        current = dir.parent();
    }

    tracing::debug!(
        "no `src` ancestor or ancestor Cargo.toml for '{}'; \
         falling back to the scan root for crate:: resolution",
        from.display()
    );
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

fn is_rust_path_attribute(raw: &str) -> bool {
    raw.contains('/')
        || raw.contains('\\')
        || Path::new(raw)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    let mut escaped_root = false;
    for component in path.components() {
        match component {
            Component::ParentDir => match components.last() {
                Some(Component::Normal(_)) => {
                    components.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {
                    escaped_root = true;
                }
                Some(Component::ParentDir) | None => {
                    escaped_root = true;
                    components.push(component);
                }
                Some(Component::CurDir) => {}
            },
            Component::CurDir => {}
            other => {
                components.push(other);
            }
        }
    }
    if escaped_root {
        tracing::warn!(
            "Path '{}' traverses above the project root during import resolution",
            path.display()
        );
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

    fn node_with_imports(path: &str, imports: Vec<Import>) -> FileNode {
        FileNode {
            imports,
            ..node(path)
        }
    }

    fn rust_import(raw: &str, kind: ImportKind) -> Import {
        Import {
            raw: raw.to_string(),
            kind,
            resolved: None,
            span: None,
        }
    }

    fn python_import(raw: &str) -> Import {
        Import {
            raw: raw.to_string(),
            kind: if raw.starts_with('.') {
                ImportKind::Local
            } else {
                ImportKind::External
            },
            resolved: None,
            span: None,
        }
    }

    fn python_node_with_imports(path: &str, imports: Vec<Import>) -> FileNode {
        FileNode {
            path: PathBuf::from(path),
            lang: Lang::Python,
            imports,
            symbols: Vec::new(),
            calls: Vec::new(),
        }
    }

    fn resolver_from_nodes(nodes: &[FileNode]) -> Resolver {
        Resolver::new(Path::new(""), nodes)
    }

    fn resolver(files: &[&str]) -> Resolver {
        let nodes: Vec<FileNode> = files.iter().map(|p| node(p)).collect();
        resolver_from_nodes(&nodes)
    }

    fn resolve_rust(files: &[&str], raw: &str, from: &str) -> Option<PathBuf> {
        resolver(files).resolve_rust(raw, Path::new(from), ImportKind::External)
    }

    fn resolve_rust_mod(files: &[&str], raw: &str, from: &str) -> Option<PathBuf> {
        resolver(files).resolve_rust(raw, Path::new(from), ImportKind::Local)
    }

    fn resolve_go(files: &[&str], raw: &str, from: &str) -> Option<PathBuf> {
        resolver(files).resolve_go(raw, Path::new(from))
    }

    const TS_EXTS: &[&str] = TYPESCRIPT_RESOLVE_EXTS;

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

    /// Unique on-disk root containing Cargo.toml manifests at the given
    /// subdirectories ("" = the root itself) — same precedent as
    /// `tsconfig_root` / `go_mod_root` below.
    fn cargo_root(tag: &str, manifest_dirs: &[&str]) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kgr-resolve-cargo-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for sub in manifest_dirs {
            let manifest_dir = dir.join(sub);
            std::fs::create_dir_all(&manifest_dir).unwrap();
            std::fs::write(manifest_dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        }
        dir
    }

    #[test]
    fn crate_import_resolves_in_lib_at_root_workspace_member() {
        // No `src/` dir anywhere: the member crate keeps lib.rs beside its
        // Cargo.toml. `crate::` must anchor at the NEAREST manifest dir (the
        // member), not the workspace root and not the scan-root fallback.
        let root = cargo_root("member", &["", "member"]);
        let nodes: Vec<FileNode> = ["member/lib.rs", "member/util.rs"]
            .iter()
            .map(|p| node(p))
            .collect();
        let r = Resolver::new(&root, &nodes);
        // Bases are precomputed in `new`; resolution itself needs no disk.
        std::fs::remove_dir_all(&root).ok();

        let got = r.resolve_rust(
            "crate::util",
            Path::new("member/lib.rs"),
            ImportKind::External,
        );
        assert_eq!(got, Some(PathBuf::from("member/util.rs")));
    }

    #[test]
    fn crate_import_resolves_when_manifest_is_at_the_scan_root() {
        // Single lib-at-root crate scanned at its own root: the manifest dir
        // IS the scan root, so `crate::` anchors there intentionally.
        let root = cargo_root("rootlib", &[""]);
        let nodes: Vec<FileNode> = ["lib.rs", "util.rs"].iter().map(|p| node(p)).collect();
        let r = Resolver::new(&root, &nodes);
        std::fs::remove_dir_all(&root).ok();

        let got = r.resolve_rust("crate::util", Path::new("lib.rs"), ImportKind::External);
        assert_eq!(got, Some(PathBuf::from("util.rs")));
    }

    #[test]
    fn crate_item_anchor_holds_in_lib_at_root_layout() {
        // Anchor-file contract in a manifest-derived layout: a `crate::` item
        // path whose segments match no module file anchors to the crate root
        // lib.rs found beside the Cargo.toml.
        let root = cargo_root("anchor", &["member"]);
        let nodes: Vec<FileNode> = ["member/lib.rs", "member/util.rs"]
            .iter()
            .map(|p| node(p))
            .collect();
        let r = Resolver::new(&root, &nodes);
        std::fs::remove_dir_all(&root).ok();

        let got = r.resolve_rust(
            "crate::Config",
            Path::new("member/util.rs"),
            ImportKind::External,
        );
        assert_eq!(got, Some(PathBuf::from("member/lib.rs")));
    }

    #[test]
    fn crate_import_without_src_or_manifest_falls_back_to_scan_root() {
        // Documented last resort: no `src` ancestor and no Cargo.toml on
        // disk. `crate::` re-anchors at the scan root (logged at debug
        // level), so the member-local target is NOT found.
        let root = cargo_root("bare", &[]);
        let nodes: Vec<FileNode> = ["member/lib.rs", "member/util.rs"]
            .iter()
            .map(|p| node(p))
            .collect();
        let r = Resolver::new(&root, &nodes);
        std::fs::remove_dir_all(&root).ok();

        let got = r.resolve_rust(
            "crate::util",
            Path::new("member/lib.rs"),
            ImportKind::External,
        );
        assert_eq!(got, None);
    }

    #[test]
    fn src_ancestor_wins_without_filesystem_probes() {
        // Standard layouts derive purely from the path: a nonexistent root
        // proves no Cargo.toml probe runs (none is memoized either).
        let mut memo = HashMap::new();
        let base = derive_crate_src_base(
            Path::new("/nonexistent-kgr-test-root"),
            Path::new("crates/foo/src/bar/baz.rs"),
            &mut memo,
        );
        assert_eq!(base, PathBuf::from("crates/foo/src"));
        assert!(memo.is_empty());
    }

    #[test]
    fn crate_src_base_for_unscanned_file_is_derived_on_the_fly() {
        // `from` outside the scanned set has no precomputed entry; the
        // src-name fast path still anchors it without touching the disk.
        let got = resolve_rust(
            &["elsewhere/src/lib.rs", "elsewhere/src/util.rs"],
            "crate::util",
            "elsewhere/src/sub/nested.rs",
        );
        assert_eq!(got, Some(PathBuf::from("elsewhere/src/util.rs")));
    }

    #[test]
    fn bare_local_module_import_resolves() {
        // `use cli::Command;` at the crate root (the ../itr false-positive).
        let nodes = vec![
            node_with_imports("src/main.rs", vec![rust_import("cli", ImportKind::Local)]),
            node("src/cli.rs"),
        ];
        let got = resolver_from_nodes(&nodes).resolve_rust(
            "cli::Command",
            Path::new("src/main.rs"),
            ImportKind::External,
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
    fn bare_external_crate_import_ignores_shadowing_module_file() {
        let got = resolve_rust(
            &["src/main.rs", "src/time.rs"],
            "time::Duration",
            "src/main.rs",
        );
        assert_eq!(got, None);
    }

    #[test]
    fn bare_single_segment_use_resolves_only_when_declared() {
        let nodes = vec![
            node_with_imports("src/main.rs", vec![rust_import("cli", ImportKind::Local)]),
            node("src/cli.rs"),
        ];
        let declared = resolver_from_nodes(&nodes).resolve_rust(
            "cli",
            Path::new("src/main.rs"),
            ImportKind::External,
        );
        assert_eq!(declared, Some(PathBuf::from("src/cli.rs")));

        let undeclared = resolve_rust(&["src/main.rs", "src/time.rs"], "time", "src/main.rs");
        assert_eq!(undeclared, None);
    }

    #[test]
    fn mod_declaration_resolves_sibling() {
        let got = resolve_rust_mod(&["src/lib.rs", "src/util.rs"], "util", "src/lib.rs");
        assert_eq!(got, Some(PathBuf::from("src/util.rs")));
    }

    #[test]
    fn mod_declaration_prefers_child_module_dir_over_sibling() {
        // `mod config;` in src/commands.rs with BOTH src/config.rs and
        // src/commands/config.rs present — rustc only accepts the child.
        let got = resolve_rust_mod(
            &["src/commands.rs", "src/config.rs", "src/commands/config.rs"],
            "config",
            "src/commands.rs",
        );
        assert_eq!(got, Some(PathBuf::from("src/commands/config.rs")));
    }

    #[test]
    fn mod_declaration_prefers_child_mod_rs_over_sibling() {
        // Same collision, with the child shaped as config/mod.rs.
        let got = resolve_rust_mod(
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
        let got = resolve_rust_mod(
            &["src/commands.rs", "src/config.rs"],
            "config",
            "src/commands.rs",
        );
        assert_eq!(got, Some(PathBuf::from("src/config.rs")));
    }

    #[test]
    fn path_attribute_mod_declaration_resolves_relative_to_declaring_file() {
        let got = resolve_rust_mod(
            &[
                "src/commands.rs",
                "src/custom/config.rs",
                "src/commands/custom/config.rs",
            ],
            "custom/config.rs",
            "src/commands.rs",
        );
        assert_eq!(got, Some(PathBuf::from("src/custom/config.rs")));
    }

    #[test]
    fn mod_declaration_from_mod_rs_resolves_own_dir_amid_collision() {
        // From mod.rs the module dir IS the file's own dir; a same-named
        // module one level up must not interfere.
        let got = resolve_rust_mod(
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
    fn rust_glob_imports_resolve_to_the_named_module() {
        // `use super::*;` at file top level pulls in the parent module's
        // items: the dependency is the parent's defining file.
        let super_glob = resolve_rust(
            &["src/a/mod.rs", "src/a/b.rs", "src/a/c.rs"],
            "super::*",
            "src/a/b.rs",
        );
        assert_eq!(super_glob, Some(PathBuf::from("src/a/mod.rs")));

        // `pub use crate::foo::*;` — the re-export barrel depends on foo.
        let crate_glob = resolve_rust(
            &["src/lib.rs", "src/foo.rs", "src/foo/bar.rs"],
            "crate::foo::*",
            "src/lib.rs",
        );
        assert_eq!(crate_glob, Some(PathBuf::from("src/foo.rs")));
    }

    #[test]
    fn bare_glob_reexport_resolves_only_through_declared_modules() {
        // `pub use cli::*;` in main.rs with `mod cli;` parsed — the barrel
        // gets an out-edge. The same shape against an undeclared name (an
        // external crate glob) must stay unresolved.
        let nodes = vec![
            node_with_imports("src/main.rs", vec![rust_import("cli", ImportKind::Local)]),
            node("src/cli.rs"),
            node("src/time.rs"),
        ];
        let r = resolver_from_nodes(&nodes);
        assert_eq!(
            r.resolve_rust("cli::*", Path::new("src/main.rs"), ImportKind::Local),
            Some(PathBuf::from("src/cli.rs"))
        );
        assert_eq!(
            r.resolve_rust("time::*", Path::new("src/main.rs"), ImportKind::External),
            None
        );
    }

    #[test]
    fn crate_item_path_anchors_to_the_crate_root_file() {
        // `use crate::Config;` — Config is an item defined in (or re-exported
        // by) lib.rs, so lib.rs is the file-level dependency.
        let got = resolve_rust(&["src/lib.rs", "src/foo.rs"], "crate::Config", "src/foo.rs");
        assert_eq!(got, Some(PathBuf::from("src/lib.rs")));

        // Workspace layout anchors at the owning crate's root, not the scan root.
        let workspace = resolve_rust(
            &["crates/core/src/lib.rs", "crates/core/src/foo.rs"],
            "crate::Config",
            "crates/core/src/foo.rs",
        );
        assert_eq!(workspace, Some(PathBuf::from("crates/core/src/lib.rs")));

        // Bin-only crate: the root is main.rs.
        let bin = resolve_rust(
            &["src/main.rs", "src/foo.rs"],
            "crate::Config",
            "src/foo.rs",
        );
        assert_eq!(bin, Some(PathBuf::from("src/main.rs")));
    }

    #[test]
    fn super_item_path_anchors_to_the_parent_module_file() {
        // `use super::helper;` where helper is an item, not a submodule file:
        // the dependency is the parent module's defining file.
        let mod_rs = resolve_rust(
            &["src/a/mod.rs", "src/a/b.rs"],
            "super::helper",
            "src/a/b.rs",
        );
        assert_eq!(mod_rs, Some(PathBuf::from("src/a/mod.rs")));

        // Modern layout: the parent module is the sibling a.rs.
        let sibling = resolve_rust(&["src/a.rs", "src/a/b.rs"], "super::helper", "src/a/b.rs");
        assert_eq!(sibling, Some(PathBuf::from("src/a.rs")));

        // One super from a top-level module lands on the crate root.
        let root = resolve_rust(&["src/lib.rs", "src/b.rs"], "super::helper", "src/b.rs");
        assert_eq!(root, Some(PathBuf::from("src/lib.rs")));
    }

    #[test]
    fn self_item_path_resolves_to_the_importing_file_itself() {
        // resolve_all clears these (no self-loop edge); resolution-level they
        // identify the importing file.
        let got = resolve_rust(&["src/lib.rs", "src/foo.rs"], "self::Item", "src/foo.rs");
        assert_eq!(got, Some(PathBuf::from("src/foo.rs")));

        let glob = resolve_rust(&["src/lib.rs", "src/foo.rs"], "self::*", "src/foo.rs");
        assert_eq!(glob, Some(PathBuf::from("src/foo.rs")));
    }

    #[test]
    fn keyword_only_paths_resolve_to_module_files() {
        // `use crate as me;`
        let krate = resolve_rust(&["src/lib.rs", "src/foo.rs"], "crate", "src/foo.rs");
        assert_eq!(krate, Some(PathBuf::from("src/lib.rs")));

        // `use super::{self};` at file top level.
        let sup = resolve_rust(&["src/a/mod.rs", "src/a/b.rs"], "super", "src/a/b.rs");
        assert_eq!(sup, Some(PathBuf::from("src/a/mod.rs")));

        // Keyword raws never feed the declared-modules set, so a stray
        // `crate.rs` file is not treated as a declared module target.
        let nodes = vec![
            node_with_imports("src/main.rs", vec![rust_import("crate", ImportKind::Local)]),
            node("src/crate.rs"),
        ];
        let r = resolver_from_nodes(&nodes);
        assert!(!r
            .rust_declared_modules
            .contains(&PathBuf::from("src/crate.rs")));
    }

    #[test]
    fn full_resolve_keeps_rust_self_reference_local_and_unresolved() {
        // A test-module `use super::*;` arrives rebased as `self::*`. It must
        // not create an edge, a self-loop cycle, or an External entry.
        let mut files = vec![
            FileNode {
                path: PathBuf::from("src/foo.rs"),
                lang: Lang::Rust,
                imports: vec![
                    rust_import("self::*", ImportKind::Local),
                    rust_import("self::helper", ImportKind::Local),
                ],
                symbols: Vec::new(),
                calls: Vec::new(),
            },
            node("src/lib.rs"),
        ];
        let r = resolver_from_nodes(&files);
        r.resolve_all(&mut files);

        for import in &files[0].imports {
            assert_eq!(import.kind, ImportKind::Local, "{} flipped", import.raw);
            assert_eq!(import.resolved, None, "{} resolved", import.raw);
        }
    }

    #[test]
    fn full_resolve_crate_item_in_crate_root_itself_creates_no_self_loop() {
        // lib.rs referring to its own items via `use crate::Item;` (common
        // with inline modules) must not become a size-one cycle.
        let mut files = vec![
            FileNode {
                path: PathBuf::from("src/lib.rs"),
                lang: Lang::Rust,
                imports: vec![rust_import("crate::Item", ImportKind::Local)],
                symbols: Vec::new(),
                calls: Vec::new(),
            },
            node("src/util.rs"),
        ];
        let r = resolver_from_nodes(&files);
        r.resolve_all(&mut files);

        assert_eq!(files[0].imports[0].kind, ImportKind::Local);
        assert_eq!(files[0].imports[0].resolved, None);
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
    fn go_relative_package_resolution_ignores_nested_packages() {
        let got = resolve_go(
            &["pkg/service/zeta.go", "pkg/service/internal/aaa.go"],
            "./service",
            "pkg/main.go",
        );
        assert_eq!(got, Some(PathBuf::from("pkg/service/zeta.go")));
    }

    #[test]
    fn full_resolve_flips_kind_to_local() {
        // End-to-end through resolve_all: a bare local-module import is
        // upgraded from External to Local once resolved.
        let mut files = vec![
            FileNode {
                path: PathBuf::from("src/main.rs"),
                lang: Lang::Rust,
                imports: vec![
                    rust_import("cli", ImportKind::Local),
                    rust_import("cli::Command", ImportKind::External),
                ],
                symbols: Vec::new(),
                calls: Vec::new(),
            },
            node("src/cli.rs"),
        ];
        let r = resolver_from_nodes(&files);
        r.resolve_all(&mut files);
        assert_eq!(files[0].imports[1].kind, ImportKind::Local);
        assert_eq!(
            files[0].imports[1].resolved,
            Some(PathBuf::from("src/cli.rs"))
        );
    }

    #[test]
    fn full_resolve_keeps_shadowing_bare_crate_import_external() {
        let mut files = vec![
            FileNode {
                path: PathBuf::from("src/main.rs"),
                lang: Lang::Rust,
                imports: vec![rust_import("time::Duration", ImportKind::External)],
                symbols: Vec::new(),
                calls: Vec::new(),
            },
            node("src/time.rs"),
        ];
        let r = resolver_from_nodes(&files);
        r.resolve_all(&mut files);
        assert_eq!(files[0].imports[0].kind, ImportKind::External);
        assert_eq!(files[0].imports[0].resolved, None);
    }

    #[test]
    fn full_resolve_surfaces_unresolved_local_import_as_external() {
        let mut files = vec![FileNode {
            path: PathBuf::from("src/app.ts"),
            lang: Lang::TypeScript,
            imports: vec![Import {
                raw: "./missing".to_string(),
                kind: ImportKind::Local,
                resolved: None,
                span: None,
            }],
            symbols: Vec::new(),
            calls: Vec::new(),
        }];

        let r = resolver_from_nodes(&files);
        r.resolve_all(&mut files);

        assert_eq!(files[0].imports[0].kind, ImportKind::External);
        assert_eq!(files[0].imports[0].resolved, None);
    }

    #[test]
    fn resolverless_local_import_is_surfaced_as_external() {
        let mut files = vec![FileNode {
            path: PathBuf::from("src/main.m"),
            lang: Lang::ObjectiveC,
            imports: vec![Import {
                raw: "LocalThing.h".to_string(),
                kind: ImportKind::Local,
                resolved: None,
                span: None,
            }],
            symbols: Vec::new(),
            calls: Vec::new(),
        }];

        let r = resolver_from_nodes(&files);
        r.resolve_all(&mut files);

        assert_eq!(files[0].imports[0].kind, ImportKind::External);
        assert_eq!(files[0].imports[0].resolved, None);
    }

    #[test]
    fn python_bare_relative_submodule_imports_resolve_distinct_targets() {
        let mut files = vec![
            python_node_with_imports(
                "pkg/__init__.py",
                vec![python_import(".alpha"), python_import(".beta")],
            ),
            python_node_with_imports("pkg/alpha.py", Vec::new()),
            python_node_with_imports("pkg/beta/__init__.py", Vec::new()),
        ];

        let r = resolver_from_nodes(&files);
        r.resolve_all(&mut files);

        assert_eq!(
            files[0].imports[0].resolved,
            Some(PathBuf::from("pkg/alpha.py"))
        );
        assert_eq!(
            files[0].imports[1].resolved,
            Some(PathBuf::from("pkg/beta/__init__.py"))
        );
        assert_ne!(files[0].imports[0].resolved, files[0].imports[1].resolved);
    }

    #[test]
    fn python_absolute_import_ignores_shadowing_sibling_file() {
        let mut files = vec![
            python_node_with_imports("pkg/main.py", vec![python_import("requests")]),
            python_node_with_imports("pkg/requests.py", Vec::new()),
        ];

        let r = resolver_from_nodes(&files);
        r.resolve_all(&mut files);

        assert_eq!(files[0].imports[0].kind, ImportKind::External);
        assert_eq!(files[0].imports[0].resolved, None);
    }

    #[test]
    fn python_absolute_import_resolves_explicit_project_package() {
        let mut files = vec![
            python_node_with_imports("pkg/main.py", vec![python_import("pkg.requests")]),
            python_node_with_imports("pkg/requests.py", Vec::new()),
        ];

        let r = resolver_from_nodes(&files);
        r.resolve_all(&mut files);

        assert_eq!(files[0].imports[0].kind, ImportKind::Local);
        assert_eq!(
            files[0].imports[0].resolved,
            Some(PathBuf::from("pkg/requests.py"))
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
    fn typescript_resolves_mts_extension_candidates() {
        let got = resolve_ts(&["src/app.mts", "src/foo.mts"], "./foo", "src/app.mts");
        assert_eq!(got, Some(PathBuf::from("src/foo.mts")));
    }

    #[test]
    fn typescript_resolves_cts_extension_candidates() {
        let got = resolve_ts(&["src/app.cts", "src/foo.cts"], "./foo", "src/app.cts");
        assert_eq!(got, Some(PathBuf::from("src/foo.cts")));
    }

    #[test]
    fn javascript_resolution_can_find_typescript_module_files() {
        let got = resolver(&["src/app.mjs", "src/foo.mts"]).resolve(
            "./foo",
            Path::new("src/app.mjs"),
            Lang::JavaScript,
            ImportKind::Local,
        );
        assert_eq!(got, Some(PathBuf::from("src/foo.mts")));
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
        let r = Resolver::new(&root, &nodes);
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
        let r = Resolver::new(&root, &nodes);
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

    #[test]
    fn parent_relative_import_can_resolve_known_target_above_scan_root() {
        let got = resolve_ts(
            &["src/app.ts", "../shared.ts"],
            "../../shared",
            "src/app.ts",
        );
        assert_eq!(got, Some(PathBuf::from("../shared.ts")));
    }

    #[test]
    fn parent_relative_import_above_scan_root_does_not_match_root_shadow() {
        let got = resolve_ts(&["app.ts", "shared.ts"], "../shared", "app.ts");
        assert_eq!(got, None);
    }

    #[test]
    fn mts_full_resolve_produces_local_edge_target() {
        let r = resolver(&["src/main.mts", "src/util.mts"]);
        let mut files = vec![FileNode {
            path: PathBuf::from("src/main.mts"),
            lang: Lang::TypeScript,
            imports: vec![Import {
                raw: "./util".to_string(),
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
            Some(PathBuf::from("src/util.mts"))
        );
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
        let r = Resolver::new(&root, &nodes);
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
        let r = Resolver::new(&root, &nodes);
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
