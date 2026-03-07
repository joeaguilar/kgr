use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;

use kgr_core::graph::KGraph;
use kgr_core::types::{DepGraph, ImportKind};

pub fn render_tree(
    graph: &DepGraph,
    kgraph: &KGraph,
    no_external: bool,
    writer: &mut dyn Write,
) -> std::io::Result<()> {
    let cycle_edges: HashSet<(PathBuf, PathBuf)> = kgraph.cycle_edges().into_iter().collect();

    let roots = &graph.roots;

    if roots.is_empty() {
        writeln!(writer, "(no entry points found)")?;
        return Ok(());
    }

    for root in roots {
        // Skip orphans and test entries from root display
        if graph.orphans.contains(root) || graph.test_entries.contains(root) {
            continue;
        }

        writeln!(writer, "{}  [entry]", root.display())?;
        let mut visited = HashSet::new();
        visited.insert(root.clone());
        render_children(
            kgraph,
            root,
            "",
            &cycle_edges,
            &mut visited,
            no_external,
            writer,
        )?;
    }

    if !graph.test_entries.is_empty() {
        writeln!(writer)?;
        writeln!(writer, "Test entry points:")?;
        for entry in &graph.test_entries {
            writeln!(writer, "  {}", entry.display())?;
        }
    }

    if !graph.orphans.is_empty() {
        writeln!(writer)?;
        writeln!(writer, "Orphaned files:")?;
        for orphan in &graph.orphans {
            writeln!(writer, "  {}", orphan.display())?;
        }
    }

    Ok(())
}

fn render_children(
    kgraph: &KGraph,
    node: &PathBuf,
    prefix: &str,
    cycle_edges: &HashSet<(PathBuf, PathBuf)>,
    visited: &mut HashSet<PathBuf>,
    no_external: bool,
    writer: &mut dyn Write,
) -> std::io::Result<()> {
    let mut edges = kgraph.edges_from(node);
    if no_external {
        edges.retain(|(_, kind)| *kind != ImportKind::External);
    }
    edges.sort_by(|a, b| a.0.cmp(&b.0));

    let count = edges.len();
    for (i, (target, _kind)) in edges.iter().enumerate() {
        let is_last = i == count - 1;
        let connector = if is_last {
            "\u{2514}\u{2500}\u{2500} "
        } else {
            "\u{251c}\u{2500}\u{2500} "
        };
        let child_prefix = if is_last { "    " } else { "\u{2502}   " };

        let is_cycle = cycle_edges.contains(&(node.clone(), target.clone()));

        if is_cycle {
            writeln!(
                writer,
                "{}{}{} \u{27f3} CYCLE",
                prefix,
                connector,
                target.display()
            )?;
        } else if visited.contains(target) {
            writeln!(
                writer,
                "{}{}{} (already shown)",
                prefix,
                connector,
                target.display()
            )?;
        } else {
            writeln!(writer, "{}{}{}", prefix, connector, target.display())?;
            visited.insert(target.clone());
            let new_prefix = format!("{}{}", prefix, child_prefix);
            render_children(
                kgraph,
                target,
                &new_prefix,
                cycle_edges,
                visited,
                no_external,
                writer,
            )?;
        }
    }

    Ok(())
}
