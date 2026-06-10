use std::io::Write;

use kgr_core::graph::KGraph;
use kgr_core::types::{DepGraph, ImportKind};

pub fn render_table(
    graph: &DepGraph,
    kgraph: &KGraph,
    show_external: bool,
    writer: &mut dyn Write,
) -> std::io::Result<()> {
    // Header
    writeln!(
        writer,
        "{:<50} {:<12} {:>8} {:>9} {:>5} {:>7} STATUS",
        "FILE", "LANG", "LOCAL-IN", "LOCAL-OUT", "EXT", "CYCLES"
    )?;
    writeln!(writer, "{}", "-".repeat(105))?;

    let cycle_files: std::collections::HashSet<_> =
        graph.cycles.iter().flat_map(|c| c.iter()).collect();

    let mut files: Vec<_> = graph.files.iter().collect();
    files.sort_by_key(|f| &f.path);

    for file in &files {
        // Use graph degrees for both directions so LOCAL-OUT agrees with
        // LOCAL-IN and the edge list: the graph collapses parallel imports
        // (e.g. `mod foo;` + `use foo::Bar;`) into a single edge, while the
        // raw import list would count them separately.
        let local_in = kgraph.in_degree(&file.path);
        let local_out = kgraph.out_degree(&file.path);
        let ext_out = file
            .imports
            .iter()
            .filter(|i| i.kind == ImportKind::External)
            .count();

        let in_cycle = cycle_files.contains(&file.path);
        let is_orphan = graph.orphans.contains(&file.path);
        let is_test_entry = graph.test_entries.contains(&file.path);
        let is_entry = graph.roots.contains(&file.path) && !is_orphan && !is_test_entry;

        let cycle_marker = if in_cycle { "\u{27f3}" } else { "\u{2014}" };
        let status = if in_cycle {
            "cycle"
        } else if is_orphan {
            "orphan"
        } else if is_test_entry {
            "test-entry"
        } else if is_entry {
            "entry"
        } else {
            "\u{2014}"
        };

        writeln!(
            writer,
            "{:<50} {:<12} {:>8} {:>9} {:>5} {:>7} {}",
            file.path.display(),
            file.lang,
            local_in,
            local_out,
            ext_out,
            cycle_marker,
            status,
        )?;

        if show_external && ext_out > 0 {
            let pkgs: Vec<&str> = file
                .imports
                .iter()
                .filter(|i| i.kind == ImportKind::External)
                .map(|i| i.raw.as_str())
                .collect();
            writeln!(writer, "  \u{2514} external: {}", pkgs.join(", "))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kgr_core::types::{FileNode, Import, Lang};
    use std::path::PathBuf;

    fn local_import(raw: &str, resolved: &str) -> Import {
        Import {
            raw: raw.to_string(),
            kind: ImportKind::Local,
            resolved: Some(PathBuf::from(resolved)),
            span: None,
        }
    }

    fn file(path: &str, imports: Vec<Import>) -> FileNode {
        FileNode {
            path: PathBuf::from(path),
            lang: Lang::Rust,
            imports,
            symbols: Vec::new(),
            calls: Vec::new(),
        }
    }

    /// Rows are `FILE LANG LOCAL-IN LOCAL-OUT EXT CYCLES STATUS`; return the
    /// whitespace-split data rows (header and separator skipped).
    fn data_rows(rendered: &str) -> Vec<Vec<String>> {
        rendered
            .lines()
            .skip(2)
            .map(|l| l.split_whitespace().map(str::to_string).collect())
            .collect()
    }

    /// `mod foo;` + `use foo::Bar;` both resolve to the same file. The graph
    /// collapses them into a single edge, so LOCAL-OUT must report 1 (matching
    /// LOCAL-IN on the target and the edge list), not the raw resolved-import
    /// count of 2.
    #[test]
    fn parallel_imports_collapse_to_single_local_out() {
        let files = vec![
            file(
                "src/main.rs",
                vec![
                    local_import("foo", "src/foo.rs"),
                    local_import("foo::Bar", "src/foo.rs"),
                ],
            ),
            file("src/foo.rs", Vec::new()),
        ];

        let kgraph = KGraph::from_files(&files);
        let graph = kgraph.to_dep_graph(PathBuf::from("src"), files);
        assert_eq!(graph.edges.len(), 1, "parallel imports collapse to 1 edge");

        let mut out = Vec::new();
        render_table(&graph, &kgraph, false, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        let rows = data_rows(&rendered);

        let main_row = rows.iter().find(|r| r[0] == "src/main.rs").unwrap();
        assert_eq!(main_row[2], "0", "main.rs LOCAL-IN");
        assert_eq!(main_row[3], "1", "main.rs LOCAL-OUT");

        let foo_row = rows.iter().find(|r| r[0] == "src/foo.rs").unwrap();
        assert_eq!(foo_row[2], "1", "foo.rs LOCAL-IN");
        assert_eq!(foo_row[3], "0", "foo.rs LOCAL-OUT");
    }

    /// On a local-only graph, the LOCAL-OUT column must sum to the same total
    /// as LOCAL-IN, and both must equal the number of edges.
    #[test]
    fn local_in_and_out_columns_balance_with_edge_count() {
        let files = vec![
            file(
                "src/a.rs",
                vec![
                    local_import("b", "src/b.rs"),
                    local_import("b::B", "src/b.rs"),
                    local_import("c", "src/c.rs"),
                ],
            ),
            file("src/b.rs", vec![local_import("c", "src/c.rs")]),
            file("src/c.rs", Vec::new()),
        ];

        let kgraph = KGraph::from_files(&files);
        let graph = kgraph.to_dep_graph(PathBuf::from("src"), files);

        let mut out = Vec::new();
        render_table(&graph, &kgraph, false, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        let rows = data_rows(&rendered);

        let sum_in: usize = rows.iter().map(|r| r[2].parse::<usize>().unwrap()).sum();
        let sum_out: usize = rows.iter().map(|r| r[3].parse::<usize>().unwrap()).sum();

        assert_eq!(sum_out, graph.edges.len(), "sum(LOCAL-OUT) == edges");
        assert_eq!(sum_in, graph.edges.len(), "sum(LOCAL-IN) == edges");
    }
}
