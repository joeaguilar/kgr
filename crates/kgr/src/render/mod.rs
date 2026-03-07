pub mod dot;
pub mod json;
pub mod mermaid;
pub mod table;
pub mod tree;

use std::io::Write;

use kgr_core::graph::KGraph;
use kgr_core::types::DepGraph;

pub fn render(
    graph: &DepGraph,
    kgraph: &KGraph,
    format: &str,
    no_external: bool,
    writer: &mut dyn Write,
) -> std::io::Result<()> {
    match format {
        "json" => json::render_json(graph, writer),
        "tree" => tree::render_tree(graph, kgraph, no_external, writer),
        "dot" => dot::render_dot(graph, kgraph, writer),
        "table" => table::render_table(graph, kgraph, writer),
        "mermaid" => mermaid::render_mermaid(graph, kgraph, writer),
        _ => {
            writeln!(
                writer,
                "Unknown format: {}. Use json, tree, dot, table, or mermaid.",
                format
            )
        }
    }
}
