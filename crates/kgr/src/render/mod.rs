pub mod dot;
pub mod json;
pub mod mermaid;
pub mod table;
pub mod tree;

use std::io::Write;

use kgr_core::graph::KGraph;
use kgr_core::types::{DepGraph, FileNode, ImportKind};

pub(crate) fn external_pkgs(file: &FileNode) -> impl Iterator<Item = &str> {
    file.imports
        .iter()
        .filter(|i| i.kind == ImportKind::External)
        .map(|i| i.raw.as_str())
}

fn show_external_enabled(no_external: bool, show_external: bool) -> bool {
    show_external && !no_external
}

fn warn_external_flags(format: &str, no_external: bool, show_external: bool) {
    if no_external && show_external {
        eprintln!("Warning: --no-external overrides --show-external.");
    }

    if show_external_enabled(no_external, show_external) && matches!(format, "dot" | "mermaid") {
        eprintln!("Warning: --show-external only affects tree/table output.");
    }
}

pub fn render(
    graph: &DepGraph,
    kgraph: &KGraph,
    format: &str,
    no_external: bool,
    show_external: bool,
    writer: &mut dyn Write,
) -> std::io::Result<()> {
    warn_external_flags(format, no_external, show_external);
    let show_external = show_external_enabled(no_external, show_external);

    match format {
        "json" => json::render_json(graph, writer),
        "tree" => tree::render_tree(graph, kgraph, no_external, show_external, writer),
        "dot" => dot::render_dot(graph, kgraph, writer),
        "table" => table::render_table(graph, kgraph, no_external, show_external, writer),
        "mermaid" => mermaid::render_mermaid(graph, kgraph, writer),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Unknown format: {format}. Use json, tree, dot, table, or mermaid."),
        )),
    }
}
