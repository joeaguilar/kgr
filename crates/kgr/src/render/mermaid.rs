use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;

use kgr_core::graph::KGraph;
use kgr_core::types::DepGraph;

pub fn render_mermaid(
    graph: &DepGraph,
    kgraph: &KGraph,
    writer: &mut dyn Write,
) -> std::io::Result<()> {
    let cycle_edges: HashSet<(PathBuf, PathBuf)> = kgraph.cycle_edges().into_iter().collect();

    let cycle_files: HashSet<_> = graph
        .cycles
        .iter()
        .flat_map(|c| c.iter().cloned())
        .collect();

    writeln!(writer, "graph LR")?;

    // Sanitize node names for mermaid (replace / and . with _)
    let sanitize =
        |p: &PathBuf| -> String { p.display().to_string().replace(['/', '.', '-'], "_") };

    let label = |p: &PathBuf| -> String {
        p.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| p.display().to_string())
    };

    // Node declarations with labels
    for file in &graph.files {
        let id = sanitize(&file.path);
        let lbl = label(&file.path);
        writeln!(writer, "  {}[\"{}\" ]", id, lbl)?;
    }

    writeln!(writer)?;

    // Edges
    for edge in &graph.edges {
        let from_id = sanitize(&edge.from);
        let to_id = sanitize(&edge.to);
        let is_cycle = cycle_edges.contains(&(edge.from.clone(), edge.to.clone()));

        if is_cycle {
            writeln!(writer, "  {} -.-> {}", from_id, to_id)?;
        } else {
            writeln!(writer, "  {} --> {}", from_id, to_id)?;
        }
    }

    // Style cycle nodes
    for path in &cycle_files {
        let id = sanitize(path);
        writeln!(writer, "  style {} fill:#ff000022", id)?;
    }

    Ok(())
}
