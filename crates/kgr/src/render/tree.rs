use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;

use kgr_core::graph::KGraph;
use kgr_core::types::{DepGraph, ImportKind};

/// Immutable rendering context threaded through the recursive tree walk.
struct TreeCtx<'a> {
    kgraph: &'a KGraph,
    cycle_edges: &'a HashSet<(PathBuf, PathBuf)>,
    no_external: bool,
    show_external: bool,
    ext_map: &'a HashMap<&'a PathBuf, Vec<&'a str>>,
}

pub fn render_tree(
    graph: &DepGraph,
    kgraph: &KGraph,
    no_external: bool,
    show_external: bool,
    writer: &mut dyn Write,
) -> std::io::Result<()> {
    let cycle_edges: HashSet<(PathBuf, PathBuf)> = kgraph.cycle_edges().into_iter().collect();

    // Build a map of file -> external dep names for --show-external.
    let ext_map: HashMap<&PathBuf, Vec<&str>> = if show_external {
        graph
            .files
            .iter()
            .filter_map(|f| {
                let pkgs: Vec<&str> = f
                    .imports
                    .iter()
                    .filter(|i| i.kind == ImportKind::External)
                    .map(|i| i.raw.as_str())
                    .collect();
                if pkgs.is_empty() {
                    None
                } else {
                    Some((&f.path, pkgs))
                }
            })
            .collect()
    } else {
        HashMap::new()
    };

    let roots = &graph.roots;

    if roots.is_empty() {
        writeln!(writer, "(no entry points found)")?;
        return Ok(());
    }

    let ctx = TreeCtx {
        kgraph,
        cycle_edges: &cycle_edges,
        no_external,
        show_external,
        ext_map: &ext_map,
    };

    for root in roots {
        // Skip orphans and test entries from root display
        if graph.orphans.contains(root) || graph.test_entries.contains(root) {
            continue;
        }

        writeln!(writer, "{}  [entry]", root.display())?;
        let mut visited = HashSet::new();
        visited.insert(root.clone());
        render_children(&ctx, root, "", &mut visited, writer)?;
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
    ctx: &TreeCtx<'_>,
    node: &PathBuf,
    prefix: &str,
    visited: &mut HashSet<PathBuf>,
    writer: &mut dyn Write,
) -> std::io::Result<()> {
    let mut edges = ctx.kgraph.edges_from(node);
    if ctx.no_external {
        edges.retain(|(_, kind)| *kind != ImportKind::External);
    }
    edges.sort_by(|a, b| a.0.cmp(&b.0));

    // Determine if we'll append an external block after local children.
    let ext_pkgs: &[&str] = if ctx.show_external {
        ctx.ext_map.get(node).map(|v| v.as_slice()).unwrap_or(&[])
    } else {
        &[]
    };

    let total = edges.len() + ext_pkgs.len();
    for (i, (target, _kind)) in edges.iter().enumerate() {
        let is_last = i == total - 1;
        let connector = if is_last {
            "\u{2514}\u{2500}\u{2500} "
        } else {
            "\u{251c}\u{2500}\u{2500} "
        };
        let child_prefix = if is_last { "    " } else { "\u{2502}   " };

        let is_cycle = ctx.cycle_edges.contains(&(node.clone(), target.clone()));

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
            render_children(ctx, target, &new_prefix, visited, writer)?;
        }
    }

    // Append external deps as leaf nodes after local children.
    for (i, pkg) in ext_pkgs.iter().enumerate() {
        let edge_offset = edges.len();
        let is_last = (edge_offset + i) == total - 1;
        let connector = if is_last {
            "\u{2514}\u{2500}\u{2500} "
        } else {
            "\u{251c}\u{2500}\u{2500} "
        };
        writeln!(writer, "{}{}{} [ext]", prefix, connector, pkg)?;
    }

    Ok(())
}
