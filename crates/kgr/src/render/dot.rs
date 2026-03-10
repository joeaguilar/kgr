use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;

use kgr_core::graph::KGraph;
use kgr_core::types::{DepGraph, Lang};

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
        if is_cycle_node {
            writeln!(
                writer,
                "  \"{}\" [label=\"{}\" color=\"{}\" style=filled fillcolor=\"#ff000022\"];",
                file.path.display(),
                label,
                color
            )?;
        } else {
            writeln!(
                writer,
                "  \"{}\" [label=\"{}\" color=\"{}\"];",
                file.path.display(),
                label,
                color
            )?;
        }
    }

    writeln!(writer)?;

    // Edges
    for edge in &graph.edges {
        let is_cycle_edge = cycle_edges.contains(&(edge.from.clone(), edge.to.clone()));
        if is_cycle_edge {
            writeln!(
                writer,
                "  \"{}\" -> \"{}\" [color=\"#ff0000\" style=dashed];",
                edge.from.display(),
                edge.to.display()
            )?;
        } else {
            writeln!(
                writer,
                "  \"{}\" -> \"{}\";",
                edge.from.display(),
                edge.to.display()
            )?;
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
