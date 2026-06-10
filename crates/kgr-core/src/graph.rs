use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use petgraph::algo::tarjan_scc;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::Direction;

use crate::types::{DepEdge, DepGraph, FileNode, ImportKind, Lang};

fn is_test_entry(path: &Path) -> bool {
    const TEST_DIRS: &[&str] = &["tests", "test", "spec", "specs", "__tests__", "__mocks__"];

    for component in path.components() {
        if let std::path::Component::Normal(s) = component {
            if let Some(s) = s.to_str() {
                if TEST_DIRS.contains(&s) {
                    return true;
                }
            }
        }
    }

    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        if stem.ends_with("_test") || stem.ends_with("_spec") {
            return true;
        }
        if stem.starts_with("test_") || stem.starts_with("spec_") {
            return true;
        }
        if stem.contains(".test") || stem.contains(".spec") {
            return true;
        }
    }

    false
}

fn is_js_ts_lang(lang: Lang) -> bool {
    matches!(lang, Lang::JavaScript | Lang::TypeScript)
}

fn is_declaration_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|name| name.ends_with(".d.ts"))
}

fn path_to_slash(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(s) => Some(s.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn file_name_lower(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
}

fn file_stem_lower(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
}

fn path_components_lower(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

fn name_is_or_starts_with_dot(name: &str, prefix: &str) -> bool {
    name == prefix
        || name
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('.'))
}

fn type_companion_source(path: &Path, known_files: &HashSet<PathBuf>) -> Option<PathBuf> {
    let file_name = path.file_name()?.to_str()?;
    let base = file_name.strip_suffix(".d.ts")?;
    let dir = path.parent().unwrap_or(Path::new(""));
    for ext in ["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"] {
        let candidate = dir.join(format!("{base}.{ext}"));
        if known_files.contains(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn normalize_relative_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(s) => normalized.push(s),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {}
        }
    }
    normalized
}

fn normalize_source_ref(raw: &str) -> Option<PathBuf> {
    let raw = raw.split(['?', '#']).next().unwrap_or(raw).trim();
    if raw.is_empty() || raw.contains("://") || raw.starts_with('#') {
        return None;
    }

    let raw = raw
        .strip_prefix('/')
        .unwrap_or(raw)
        .strip_prefix("./")
        .unwrap_or(raw);

    Some(normalize_relative_path(Path::new(raw)))
}

fn quoted_values(content: &str) -> Vec<&str> {
    let mut values = Vec::new();
    let bytes = content.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let quote = bytes[i];
        if quote != b'\'' && quote != b'"' {
            i += 1;
            continue;
        }

        let start = i + 1;
        i = start;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        if i < bytes.len() {
            values.push(&content[start..i]);
        }
        i += 1;
    }

    values
}

fn html_referenced_entries(root: &Path, known_files: &HashSet<PathBuf>) -> HashSet<PathBuf> {
    let mut entries = HashSet::new();
    for html in [
        "index.html",
        "app.html",
        "src/index.html",
        "public/index.html",
    ] {
        let Ok(content) = std::fs::read_to_string(root.join(html)) else {
            continue;
        };
        for value in quoted_values(&content) {
            let Some(path) = normalize_source_ref(value) else {
                continue;
            };
            if known_files.contains(&path) {
                entries.insert(path);
            }
        }
    }
    entries
}

fn script_mentions_path(script: &str, path: &Path) -> bool {
    let path = path_to_slash(path);
    let without_ext = path
        .rsplit_once('.')
        .map_or(path.as_str(), |(stem, _)| stem);
    let dot_path = format!("./{path}");
    let dot_without_ext = format!("./{without_ext}");

    [
        path.as_str(),
        without_ext,
        dot_path.as_str(),
        dot_without_ext.as_str(),
    ]
    .iter()
    .any(|candidate| script.contains(candidate))
}

fn package_script_entries(root: &Path, files: &[FileNode]) -> HashSet<PathBuf> {
    let Ok(content) = std::fs::read_to_string(root.join("package.json")) else {
        return HashSet::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return HashSet::new();
    };
    let Some(scripts) = json.get("scripts").and_then(serde_json::Value::as_object) else {
        return HashSet::new();
    };

    let script_values: Vec<&str> = scripts
        .values()
        .filter_map(serde_json::Value::as_str)
        .collect();

    files
        .iter()
        .filter(|file| {
            is_js_ts_lang(file.lang)
                && !is_declaration_file(&file.path)
                && script_values
                    .iter()
                    .any(|script| script_mentions_path(script, &file.path))
        })
        .map(|file| file.path.clone())
        .collect()
}

fn is_config_entry(path: &Path) -> bool {
    let Some(name) = file_name_lower(path) else {
        return false;
    };

    const CONFIG_PREFIXES: &[&str] = &[
        "vite.config",
        "vitest.config",
        "jest.config",
        "playwright.config",
        "cypress.config",
        "webpack.config",
        "rollup.config",
        "tsup.config",
        "tsdown.config",
        "esbuild.config",
        "rspack.config",
        "next.config",
        "nuxt.config",
        "astro.config",
        "svelte.config",
        "tailwind.config",
        "postcss.config",
        "babel.config",
        "eslint.config",
        ".eslintrc",
        "prettier.config",
        ".prettierrc",
        "stylelint.config",
        "commitlint.config",
        "lint-staged.config",
        "wdio.conf",
        "karma.conf",
        "protractor.conf",
        "unocss.config",
    ];

    if CONFIG_PREFIXES
        .iter()
        .any(|prefix| name_is_or_starts_with_dot(&name, prefix))
    {
        return true;
    }

    let components = path_components_lower(path);
    let Some(stem) = file_stem_lower(path) else {
        return false;
    };
    components
        .iter()
        .any(|component| component == ".storybook" || component == "storybook")
        && matches!(
            stem.as_str(),
            "main" | "preview" | "manager" | "test-runner"
        )
}

fn is_setup_entry(path: &Path) -> bool {
    let Some(name) = file_name_lower(path) else {
        return false;
    };
    let Some(stem) = file_stem_lower(path) else {
        return false;
    };

    matches!(
        stem.as_str(),
        "setup" | "setuptests" | "testsetup" | "vitest.setup" | "jest.setup"
    ) || name.contains(".setup.")
}

fn is_storybook_entry(path: &Path) -> bool {
    let Some(name) = file_name_lower(path) else {
        return false;
    };
    let components = path_components_lower(path);
    name.contains(".stories.")
        || name.contains(".story.")
        || components.iter().any(|component| component == "stories")
}

fn is_named_runtime_entry(path: &Path) -> bool {
    if is_declaration_file(path) {
        return false;
    }

    let Some(stem) = file_stem_lower(path) else {
        return false;
    };
    let components = path_components_lower(path);
    let parent = components
        .len()
        .checked_sub(2)
        .and_then(|idx| components.get(idx).map(String::as_str))
        .unwrap_or("");

    let rootish_parent = parent.is_empty() || matches!(parent, "src" | "app" | "client");
    if rootish_parent
        && matches!(
            stem.as_str(),
            "main"
                | "index"
                | "bootstrap"
                | "client"
                | "server"
                | "entry-client"
                | "entry-server"
                | "renderer"
        )
    {
        return true;
    }

    matches!(
        stem.as_str(),
        "middleware" | "instrumentation" | "service-worker" | "sw"
    ) || stem.ends_with(".worker")
}

fn is_filesystem_route_entry(path: &Path) -> bool {
    if is_declaration_file(path) {
        return false;
    }

    let components = path_components_lower(path);
    if components.iter().any(|component| component == "pages") {
        return true;
    }
    if components.iter().any(|component| component == "routes") {
        return true;
    }

    let Some(stem) = file_stem_lower(path) else {
        return false;
    };
    components.iter().any(|component| component == "app")
        && matches!(
            stem.as_str(),
            "page"
                | "layout"
                | "route"
                | "loading"
                | "error"
                | "not-found"
                | "template"
                | "default"
        )
}

fn is_ambient_global_declaration(root: &Path, path: &Path, known_files: &HashSet<PathBuf>) -> bool {
    if !is_declaration_file(path) || type_companion_source(path, known_files).is_some() {
        return false;
    }

    let Some(name) = file_name_lower(path) else {
        return false;
    };
    if matches!(
        name.as_str(),
        "global.d.ts"
            | "globals.d.ts"
            | "types.d.ts"
            | "env.d.ts"
            | "vite-env.d.ts"
            | "next-env.d.ts"
            | "react-app-env.d.ts"
    ) {
        return true;
    }

    let components = path_components_lower(path);
    if components
        .iter()
        .any(|component| component == "types" || component == "@types")
    {
        return true;
    }

    std::fs::read_to_string(root.join(path)).is_ok_and(|content| {
        content.contains("declare global")
            || content.contains("declare module")
            || content.contains("/// <reference")
    })
}

fn js_ts_structural_entries(root: &Path, files: &[FileNode]) -> HashSet<PathBuf> {
    let known_files: HashSet<PathBuf> = files.iter().map(|file| file.path.clone()).collect();
    let mut entries = html_referenced_entries(root, &known_files);
    entries.extend(package_script_entries(root, files));

    for file in files {
        if !is_js_ts_lang(file.lang) {
            continue;
        }
        let path = &file.path;
        if is_config_entry(path)
            || is_setup_entry(path)
            || is_storybook_entry(path)
            || is_named_runtime_entry(path)
            || is_filesystem_route_entry(path)
            || is_ambient_global_declaration(root, path, &known_files)
        {
            entries.insert(path.clone());
        }
    }

    entries
}

pub struct KGraph {
    inner: DiGraph<PathBuf, ImportKind>,
    node_index: HashMap<PathBuf, NodeIndex>,
}

impl KGraph {
    fn has_self_loop(&self, idx: NodeIndex) -> bool {
        self.inner.find_edge(idx, idx).is_some()
    }

    fn is_cyclic_scc(&self, scc: &[NodeIndex]) -> bool {
        scc.len() > 1 || scc.first().is_some_and(|&idx| self.has_self_loop(idx))
    }

    fn is_isolated(&self, idx: NodeIndex) -> bool {
        let no_in = self
            .inner
            .neighbors_directed(idx, Direction::Incoming)
            .next()
            .is_none();
        let no_out = self
            .inner
            .neighbors_directed(idx, Direction::Outgoing)
            .next()
            .is_none();
        no_in && no_out && !self.has_self_loop(idx)
    }

    pub fn from_files(files: &[FileNode]) -> Self {
        let mut graph = DiGraph::new();
        let mut node_index = HashMap::with_capacity(files.len());

        // Add all files as nodes
        for file in files {
            let idx = graph.add_node(file.path.clone());
            node_index.insert(file.path.clone(), idx);
        }
        let known_files: HashSet<PathBuf> = node_index.keys().cloned().collect();

        // Add edges for resolved imports. Several imports can resolve to the
        // same file (e.g. `mod foo;` plus `use foo::Bar;`, or a grouped
        // `use foo::{A, B};`); collapse them to a single edge so dependent
        // counts and renderings aren't inflated by parallel edges.
        let mut seen_edges = HashSet::new();
        for file in files {
            if let Some(&from_idx) = node_index.get(&file.path) {
                for import in &file.imports {
                    if let Some(ref resolved) = import.resolved {
                        if let Some(&to_idx) = node_index.get(resolved) {
                            if seen_edges.insert((from_idx, to_idx)) {
                                graph.add_edge(from_idx, to_idx, import.kind);
                            }
                        }
                    }
                }
            }
        }

        for file in files {
            if !is_declaration_file(&file.path) {
                continue;
            }
            let Some(source) = type_companion_source(&file.path, &known_files) else {
                continue;
            };
            let (Some(&from_idx), Some(&to_idx)) =
                (node_index.get(&source), node_index.get(&file.path))
            else {
                continue;
            };
            if seen_edges.insert((from_idx, to_idx)) {
                graph.add_edge(from_idx, to_idx, ImportKind::Local);
            }
        }

        Self {
            inner: graph,
            node_index,
        }
    }

    pub fn cycles(&self) -> Vec<Vec<PathBuf>> {
        tarjan_scc(&self.inner)
            .into_iter()
            .filter(|scc| self.is_cyclic_scc(scc))
            .map(|scc| scc.into_iter().map(|idx| self.inner[idx].clone()).collect())
            .collect()
    }

    pub fn roots(&self) -> Vec<PathBuf> {
        self.node_index
            .iter()
            .filter(|(_, &idx)| {
                self.inner
                    .neighbors_directed(idx, Direction::Incoming)
                    .next()
                    .is_none()
            })
            .map(|(path, _)| path.clone())
            .collect()
    }

    pub fn orphans(&self) -> Vec<PathBuf> {
        self.orphans_excluding(&HashSet::new())
    }

    fn orphans_excluding(&self, structural_entries: &HashSet<PathBuf>) -> Vec<PathBuf> {
        self.node_index
            .iter()
            .filter(|(path, &idx)| {
                self.is_isolated(idx) && !is_test_entry(path) && !structural_entries.contains(*path)
            })
            .map(|(path, _)| path.clone())
            .collect()
    }

    pub fn test_entries(&self) -> Vec<PathBuf> {
        self.node_index
            .iter()
            .filter(|(path, &idx)| self.is_isolated(idx) && is_test_entry(path))
            .map(|(path, _)| path.clone())
            .collect()
    }

    pub fn to_dep_graph(&self, root: PathBuf, files: Vec<FileNode>) -> DepGraph {
        let mut edges = Vec::new();
        for edge in self.inner.edge_indices() {
            let (from_idx, to_idx) = self.inner.edge_endpoints(edge).unwrap();
            let kind = self.inner[edge];
            edges.push(DepEdge {
                from: self.inner[from_idx].clone(),
                to: self.inner[to_idx].clone(),
                kind,
            });
        }

        let cycles = self.cycles();
        let mut roots = self.roots();
        roots.sort();
        let structural_entries = js_ts_structural_entries(&root, &files);
        let mut orphans = self.orphans_excluding(&structural_entries);
        orphans.sort();
        let mut test_entries = self.test_entries();
        test_entries.sort();

        DepGraph {
            root,
            files,
            edges,
            cycles,
            roots,
            orphans,
            test_entries,
        }
    }

    pub fn cycle_edges(&self) -> Vec<(PathBuf, PathBuf)> {
        let sccs = tarjan_scc(&self.inner);
        let mut cycle_edges = Vec::new();

        for scc in &sccs {
            if !self.is_cyclic_scc(scc) {
                continue;
            }
            let scc_set: std::collections::HashSet<_> = scc.iter().copied().collect();
            for &node in scc {
                for neighbor in self.inner.neighbors_directed(node, Direction::Outgoing) {
                    if scc_set.contains(&neighbor) {
                        cycle_edges.push((self.inner[node].clone(), self.inner[neighbor].clone()));
                    }
                }
            }
        }

        cycle_edges
    }

    /// All transitive dependencies of a node (BFS)
    pub fn transitive_deps(&self, from: &Path, max_depth: Option<usize>) -> Vec<PathBuf> {
        let Some(&start) = self.node_index.get(from) else {
            return Vec::new();
        };

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back((start, 0usize));
        visited.insert(start);

        let mut result = Vec::new();

        while let Some((node, depth)) = queue.pop_front() {
            if let Some(max) = max_depth {
                if depth >= max {
                    continue;
                }
            }
            for neighbor in self.inner.neighbors_directed(node, Direction::Outgoing) {
                if visited.insert(neighbor) {
                    result.push(self.inner[neighbor].clone());
                    queue.push_back((neighbor, depth + 1));
                }
            }
        }

        result.sort();
        result
    }

    /// All transitive dependents (reverse graph BFS)
    pub fn transitive_dependents(&self, target: &Path) -> Vec<PathBuf> {
        let Some(&start) = self.node_index.get(target) else {
            return Vec::new();
        };

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(start);
        visited.insert(start);

        let mut result = Vec::new();

        while let Some(node) = queue.pop_front() {
            for neighbor in self.inner.neighbors_directed(node, Direction::Incoming) {
                if visited.insert(neighbor) {
                    result.push(self.inner[neighbor].clone());
                    queue.push_back(neighbor);
                }
            }
        }

        result.sort();
        result
    }

    /// Shortest path between two nodes (BFS)
    pub fn shortest_path(&self, from: &Path, to: &Path) -> Option<Vec<PathBuf>> {
        let (&start, &end) = (self.node_index.get(from)?, self.node_index.get(to)?);

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut parent: HashMap<NodeIndex, NodeIndex> = HashMap::new();

        queue.push_back(start);
        visited.insert(start);

        while let Some(node) = queue.pop_front() {
            if node == end {
                // Reconstruct path
                let mut path = vec![self.inner[end].clone()];
                let mut current = end;
                while let Some(&prev) = parent.get(&current) {
                    path.push(self.inner[prev].clone());
                    current = prev;
                }
                path.reverse();
                return Some(path);
            }
            for neighbor in self.inner.neighbors_directed(node, Direction::Outgoing) {
                if visited.insert(neighbor) {
                    parent.insert(neighbor, node);
                    queue.push_back(neighbor);
                }
            }
        }

        None
    }

    /// Files sorted by number of dependents (descending)
    pub fn heaviest(&self) -> Vec<(PathBuf, usize)> {
        let mut counts: Vec<(PathBuf, usize)> = self
            .node_index
            .iter()
            .map(|(path, &idx)| {
                let count = self
                    .inner
                    .neighbors_directed(idx, Direction::Incoming)
                    .count();
                (path.clone(), count)
            })
            .collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        counts
    }

    /// In-degree count for a specific node
    pub fn in_degree(&self, path: &Path) -> usize {
        self.node_index.get(path).map_or(0, |&idx| {
            self.inner
                .neighbors_directed(idx, Direction::Incoming)
                .count()
        })
    }

    /// Out-degree count for a specific node
    pub fn out_degree(&self, path: &Path) -> usize {
        self.node_index.get(path).map_or(0, |&idx| {
            self.inner
                .neighbors_directed(idx, Direction::Outgoing)
                .count()
        })
    }

    /// All transitive dependents with BFS depth (reverse graph BFS)
    /// depth=1 means direct dependent, depth=2 means two hops away, etc.
    pub fn transitive_dependents_with_depth(
        &self,
        target: &Path,
        max_depth: Option<usize>,
    ) -> Vec<(PathBuf, usize)> {
        let Some(&start) = self.node_index.get(target) else {
            return Vec::new();
        };

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back((start, 0usize));
        visited.insert(start);

        let mut result = Vec::new();

        while let Some((node, depth)) = queue.pop_front() {
            if let Some(max) = max_depth {
                if depth >= max {
                    continue;
                }
            }
            for neighbor in self.inner.neighbors_directed(node, Direction::Incoming) {
                if visited.insert(neighbor) {
                    let new_depth = depth + 1;
                    result.push((self.inner[neighbor].clone(), new_depth));
                    queue.push_back((neighbor, new_depth));
                }
            }
        }

        result.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        result
    }

    pub fn edges_from(&self, path: &Path) -> Vec<(PathBuf, ImportKind)> {
        if let Some(&idx) = self.node_index.get(path) {
            self.inner
                .neighbors_directed(idx, Direction::Outgoing)
                .map(|neighbor| {
                    let edge = self.inner.find_edge(idx, neighbor).unwrap();
                    (self.inner[neighbor].clone(), self.inner[edge])
                })
                .collect()
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Import, Lang};

    fn node(path: &str, deps: &[&str]) -> FileNode {
        FileNode {
            path: PathBuf::from(path),
            lang: Lang::Rust,
            imports: deps
                .iter()
                .map(|dep| Import {
                    raw: (*dep).to_string(),
                    kind: ImportKind::Local,
                    resolved: Some(PathBuf::from(dep)),
                    span: None,
                })
                .collect(),
            symbols: Vec::new(),
            calls: Vec::new(),
        }
    }

    #[test]
    fn heaviest_uses_path_tie_breaker() {
        let files = vec![
            node("src/d.rs", &["src/a.rs"]),
            node("src/c.rs", &["src/b.rs"]),
            node("src/b.rs", &[]),
            node("src/a.rs", &[]),
        ];

        let graph = KGraph::from_files(&files);
        let ranked = graph.heaviest();

        assert_eq!(
            ranked,
            vec![
                (PathBuf::from("src/a.rs"), 1),
                (PathBuf::from("src/b.rs"), 1),
                (PathBuf::from("src/c.rs"), 0),
                (PathBuf::from("src/d.rs"), 0),
            ]
        );
    }

    #[test]
    fn self_import_is_size_one_cycle_and_not_orphan() {
        let files = vec![
            node("src/self.rs", &["src/self.rs"]),
            node("src/orphan.rs", &[]),
        ];

        let graph = KGraph::from_files(&files);
        let self_path = PathBuf::from("src/self.rs");

        assert_eq!(graph.cycles(), vec![vec![self_path.clone()]]);
        assert_eq!(
            graph.cycle_edges(),
            vec![(self_path.clone(), self_path.clone())]
        );

        let mut orphans = graph.orphans();
        orphans.sort();
        assert_eq!(orphans, vec![PathBuf::from("src/orphan.rs")]);
    }
}
