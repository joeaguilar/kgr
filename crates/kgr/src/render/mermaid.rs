use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;

use kgr_core::graph::KGraph;
use kgr_core::types::DepGraph;

/// Escape a label for use inside a double-quoted Mermaid string.
///
/// Mermaid has no backslash escapes; special characters are written as
/// HTML-style entity codes (`#35;` for `#`, `#quot;` for `"`). `#` is
/// escaped first so the `#` introduced by other entities is not
/// re-escaped. `<`/`>` are escaped too so labels can never be parsed as
/// inline HTML.
fn mermaid_escape(s: &str) -> String {
    s.replace('#', "#35;")
        .replace('"', "#quot;")
        .replace('<', "#lt;")
        .replace('>', "#gt;")
}

pub fn render_mermaid(
    graph: &DepGraph,
    kgraph: &KGraph,
    writer: &mut dyn Write,
) -> std::io::Result<()> {
    let cycle_edges: HashSet<(PathBuf, PathBuf)> = kgraph.cycle_edges().into_iter().collect();

    // Union of every path that can appear in the diagram, sorted so node
    // IDs are deterministic across runs regardless of upstream collection
    // order. Sanitizing the path into the ID (the old scheme) collided
    // distinct files (`a-b.py` and `a_b.py` both became `a_b_py`,
    // fabricating edges/cycles) and let spaces/quotes through into IDs,
    // producing invalid Mermaid.
    let mut all_paths: BTreeSet<&PathBuf> = graph.files.iter().map(|f| &f.path).collect();
    for edge in &graph.edges {
        all_paths.insert(&edge.from);
        all_paths.insert(&edge.to);
    }
    for cycle in &graph.cycles {
        for path in cycle {
            all_paths.insert(path);
        }
    }

    // Unique node ID per file: n0, n1, ... in sorted-path order.
    let ids: HashMap<&PathBuf, String> = all_paths
        .iter()
        .enumerate()
        .map(|(i, p)| (*p, format!("n{i}")))
        .collect();

    let label = |p: &PathBuf| -> String {
        let name = match p.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => p.display().to_string(),
        };
        mermaid_escape(&name)
    };

    writeln!(writer, "graph LR")?;

    // Node declarations with labels
    for file in &graph.files {
        let id = &ids[&file.path];
        let lbl = label(&file.path);
        writeln!(writer, "  {id}[\"{lbl}\"]")?;
    }

    writeln!(writer)?;

    // Edges
    for edge in &graph.edges {
        let from_id = &ids[&edge.from];
        let to_id = &ids[&edge.to];
        let is_cycle = cycle_edges.contains(&(edge.from.clone(), edge.to.clone()));

        if is_cycle {
            writeln!(writer, "  {from_id} -.-> {to_id}")?;
        } else {
            writeln!(writer, "  {from_id} --> {to_id}")?;
        }
    }

    // Style cycle nodes. BTreeSet iteration is sorted by path, so the
    // style lines come out in the same order on every run (the previous
    // HashSet iteration shuffled them run-to-run).
    let cycle_files: BTreeSet<&PathBuf> = graph.cycles.iter().flat_map(|c| c.iter()).collect();
    for path in cycle_files {
        let id = &ids[path];
        writeln!(writer, "  style {id} fill:#ff000022")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kgr_core::types::{FileNode, Import, ImportKind, Lang};

    fn file(path: &str, deps: &[&str]) -> FileNode {
        FileNode {
            path: PathBuf::from(path),
            lang: Lang::Python,
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

    fn render(files: Vec<FileNode>) -> String {
        let kgraph = KGraph::from_files(&files);
        let graph = kgraph.to_dep_graph(PathBuf::from("."), files);
        let mut out = Vec::new();
        render_mermaid(&graph, &kgraph, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn mermaid_escape_uses_entity_codes() {
        assert_eq!(mermaid_escape("plain.py"), "plain.py");
        assert_eq!(mermaid_escape("he\"llo.py"), "he#quot;llo.py");
        assert_eq!(mermaid_escape("c#.py"), "c#35;.py");
        // `#` is escaped first, so the `#` introduced by `#quot;` is not
        // re-escaped.
        assert_eq!(mermaid_escape("#\""), "#35;#quot;");
        assert_eq!(mermaid_escape("<b>"), "#lt;b#gt;");
    }

    #[test]
    fn kebab_and_snake_case_files_are_distinct_nodes() {
        let output = render(vec![file("a-b.py", &["a_b.py"]), file("a_b.py", &[])]);
        // The old sanitize-based IDs merged both files into `a_b_py`,
        // fabricating a self-edge. They must be two distinct nodes joined
        // by a single edge. Sorted-path IDs: a-b.py=n0, a_b.py=n1.
        assert!(
            output.contains("  n0[\"a-b.py\"]"),
            "missing kebab-case node:\n{output}"
        );
        assert!(
            output.contains("  n1[\"a_b.py\"]"),
            "missing snake_case node:\n{output}"
        );
        assert!(output.contains("  n0 --> n1"), "missing edge:\n{output}");
        assert!(
            !output.contains("a_b_py"),
            "old colliding ID survived:\n{output}"
        );
    }

    #[test]
    fn labels_with_spaces_quotes_and_unicode_render_valid_mermaid() {
        let output = render(vec![file("my fi\"le.py", &[]), file("héllo.py", &[])]);
        assert!(
            output.contains("[\"my fi#quot;le.py\"]"),
            "quote/space label not escaped:\n{output}"
        );
        assert!(
            output.contains("[\"héllo.py\"]"),
            "unicode label mangled:\n{output}"
        );
        // Every node declaration must contain exactly the two delimiting
        // quotes — no raw quote may survive inside a label, and no quote
        // or space may leak into the node ID.
        for line in output.lines().filter(|l| l.contains('[')) {
            assert_eq!(
                line.matches('"').count(),
                2,
                "raw quote inside label: {line}"
            );
            let id = line.trim_start().split('[').next().unwrap();
            assert!(
                id.chars().all(|c| c.is_ascii_alphanumeric()),
                "non-alphanumeric node ID: {line}"
            );
        }
    }

    #[test]
    fn cycle_style_lines_are_sorted_and_deterministic() {
        let files = || {
            vec![
                file("c.py", &["a.py"]),
                file("a.py", &["b.py"]),
                file("b.py", &["c.py"]),
            ]
        };
        let first = render(files());
        // IDs follow sorted path order (a.py=n0, b.py=n1, c.py=n2) and the
        // style lines must come out in that order.
        let style_lines: Vec<&str> = first
            .lines()
            .filter(|l| l.trim_start().starts_with("style"))
            .collect();
        assert_eq!(
            style_lines,
            vec![
                "  style n0 fill:#ff000022",
                "  style n1 fill:#ff000022",
                "  style n2 fill:#ff000022",
            ],
            "style lines not in sorted order:\n{first}"
        );
        // Cycle edges render dashed.
        assert!(first.contains("  n2 -.-> n0"), "cycle edge:\n{first}");
        // Byte-identical across repeated renders — HashSet iteration order
        // used to shuffle the style lines run-to-run.
        for _ in 0..8 {
            assert_eq!(render(files()), first, "nondeterministic output");
        }
    }
}
