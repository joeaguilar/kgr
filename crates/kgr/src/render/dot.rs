use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;

use kgr_core::graph::KGraph;
use kgr_core::types::{DepGraph, Lang};

/// Escape a string for use inside a double-quoted DOT ID or label.
///
/// Per the DOT grammar, backslashes and double quotes must be escaped
/// inside double-quoted strings. Backslashes are escaped first so the
/// backslash introduced for a quote is not doubled afterwards.
fn dot_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn render_dot(
    graph: &DepGraph,
    kgraph: &KGraph,
    writer: &mut dyn Write,
) -> std::io::Result<()> {
    let cycle_edges: HashSet<(PathBuf, PathBuf)> = kgraph.cycle_edges().into_iter().collect();

    writeln!(writer, "digraph kgr {{")?;
    writeln!(writer, "  rankdir=LR;")?;
    writeln!(
        writer,
        "  node [shape=box fontname=\"monospace\" fontsize=10];"
    )?;
    writeln!(writer, "  edge [arrowsize=0.6];")?;
    writeln!(writer)?;

    // Nodes
    for file in &graph.files {
        let color = lang_color(file.lang);
        let label = file
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| file.path.display().to_string());

        let is_cycle_node = graph.cycles.iter().any(|c| c.contains(&file.path));
        let id = dot_escape(&file.path.display().to_string());
        let label = dot_escape(&label);
        if is_cycle_node {
            writeln!(
                writer,
                "  \"{id}\" [label=\"{label}\" color=\"{color}\" style=filled fillcolor=\"#ff000022\"];",
            )?;
        } else {
            writeln!(writer, "  \"{id}\" [label=\"{label}\" color=\"{color}\"];")?;
        }
    }

    writeln!(writer)?;

    // Edges
    for edge in &graph.edges {
        let is_cycle_edge = cycle_edges.contains(&(edge.from.clone(), edge.to.clone()));
        let from = dot_escape(&edge.from.display().to_string());
        let to = dot_escape(&edge.to.display().to_string());
        if is_cycle_edge {
            writeln!(
                writer,
                "  \"{from}\" -> \"{to}\" [color=\"#ff0000\" style=dashed];",
            )?;
        } else {
            writeln!(writer, "  \"{from}\" -> \"{to}\";")?;
        }
    }

    writeln!(writer, "}}")?;
    Ok(())
}

fn lang_color(lang: Lang) -> &'static str {
    match lang {
        Lang::Python => "#3776ab",
        Lang::TypeScript => "#3178c6",
        Lang::JavaScript => "#f7df1e",
        Lang::Java => "#b07219",
        Lang::C => "#555555",
        Lang::Cpp => "#f34b7d",
        Lang::Rust => "#dea584",
        Lang::Go => "#00add8",
        Lang::Zig => "#f7a41d",
        Lang::CSharp => "#178600",
        Lang::ObjectiveC => "#438eff",
        Lang::Swift => "#f05138",
        Lang::Ruby => "#cc342d",
        Lang::Php => "#4f5d95",
        Lang::Scala => "#c22d40",
        Lang::Lua => "#000080",
        Lang::Elixir => "#6e4a7e",
        Lang::Haskell => "#5e5086",
        Lang::Bash => "#89e051",
        Lang::Unknown => "#999999",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kgr_core::types::{FileNode, Import, ImportKind};

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
        render_dot(&graph, &kgraph, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn dot_escape_handles_quotes_and_backslashes() {
        assert_eq!(dot_escape("plain.py"), "plain.py");
        assert_eq!(dot_escape("he\"llo.py"), "he\\\"llo.py");
        assert_eq!(dot_escape("a\\b.py"), "a\\\\b.py");
        // Backslash-then-quote must not double-escape the quote's backslash.
        assert_eq!(dot_escape("a\\\".py"), "a\\\\\\\".py");
    }

    #[test]
    fn escapes_quotes_in_node_ids_labels_and_edges() {
        let output = render(vec![
            file("src/he\"llo.py", &["src/main.py"]),
            file("src/main.py", &[]),
        ]);

        // Node ID and label are escaped.
        assert!(
            output.contains("  \"src/he\\\"llo.py\" [label=\"he\\\"llo.py\""),
            "node line not escaped:\n{output}"
        );
        // Edge endpoints are escaped.
        assert!(
            output.contains("  \"src/he\\\"llo.py\" -> \"src/main.py\";"),
            "edge line not escaped:\n{output}"
        );
        // No unescaped quote remains inside an ID.
        assert!(!output.contains("he\"llo"), "unescaped quote:\n{output}");
    }

    #[test]
    fn escapes_backslashes_in_node_ids() {
        let output = render(vec![file("src\\win.py", &[])]);
        assert!(
            output.contains("\"src\\\\win.py\""),
            "backslash not escaped:\n{output}"
        );
    }

    #[test]
    fn escapes_cycle_nodes_and_cycle_edges() {
        let output = render(vec![
            file("src/he\"llo.py", &["src/main.py"]),
            file("src/main.py", &["src/he\"llo.py"]),
        ]);

        // Cycle node line (style=filled) is escaped.
        assert!(
            output.contains(
                "  \"src/he\\\"llo.py\" [label=\"he\\\"llo.py\" color=\"#3776ab\" style=filled"
            ),
            "cycle node line not escaped:\n{output}"
        );
        // Cycle edge line (dashed red) is escaped.
        assert!(
            output.contains(
                "  \"src/he\\\"llo.py\" -> \"src/main.py\" [color=\"#ff0000\" style=dashed];"
            ),
            "cycle edge line not escaped:\n{output}"
        );
    }

    #[test]
    fn safe_paths_are_unchanged() {
        let output = render(vec![
            file("src/app.py", &["src/util.py"]),
            file("src/util.py", &[]),
        ]);
        assert!(output.contains("  \"src/app.py\" [label=\"app.py\" color=\"#3776ab\"];"));
        assert!(output.contains("  \"src/app.py\" -> \"src/util.py\";"));
    }
}
