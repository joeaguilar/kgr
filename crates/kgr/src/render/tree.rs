use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;

use super::external_pkgs;
use kgr_core::graph::KGraph;
use kgr_core::types::DepGraph;

/// Immutable rendering context threaded through the recursive tree walk.
struct TreeCtx<'a> {
    kgraph: &'a KGraph,
    cycle_edges: &'a HashSet<(PathBuf, PathBuf)>,
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
    let show_external = show_external && !no_external;

    // Build a map of file -> external dep names for --show-external.
    let ext_map: HashMap<&PathBuf, Vec<&str>> = if show_external {
        graph
            .files
            .iter()
            .filter_map(|f| {
                let pkgs: Vec<&str> = external_pkgs(f).collect();
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

    if roots.is_empty() && graph.cycles.is_empty() {
        writeln!(writer, "(no entry points found)")?;
        return Ok(());
    }

    let ctx = TreeCtx {
        kgraph,
        cycle_edges: &cycle_edges,
        show_external,
        ext_map: &ext_map,
    };

    if roots.is_empty() {
        // Every file has an incoming edge, so the graph is (at least at its
        // entry layer) fully cyclic. Bailing out here would hide the single
        // most important fact about the codebase, so list the cycles in
        // place of the root tree and fall through to the trailing sections.
        writeln!(writer, "(no entry points: dependency cycles detected)")?;
        writeln!(writer)?;
        writeln!(writer, "Cycles:")?;
        for cycle in &graph.cycles {
            write!(writer, "  ")?;
            for (i, member) in cycle.iter().enumerate() {
                if i > 0 {
                    write!(writer, " -> ")?;
                }
                write!(writer, "{}", member.display())?;
            }
            match cycle.first() {
                Some(first) => writeln!(writer, " -> {}", first.display())?,
                None => writeln!(writer)?,
            }
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use kgr_core::types::{FileNode, Import, ImportKind, Lang};

    fn file(path: &str, imports: &[&str]) -> FileNode {
        FileNode {
            path: PathBuf::from(path),
            lang: Lang::TypeScript,
            imports: imports
                .iter()
                .map(|target| Import {
                    raw: (*target).to_string(),
                    kind: ImportKind::Local,
                    resolved: Some(PathBuf::from(target)),
                    span: None,
                })
                .collect(),
            symbols: Vec::new(),
            calls: Vec::new(),
        }
    }

    fn external_import(raw: &str) -> Import {
        Import {
            raw: raw.to_string(),
            kind: ImportKind::External,
            resolved: None,
            span: None,
        }
    }

    fn render_with_flags(files: Vec<FileNode>, no_external: bool, show_external: bool) -> String {
        let kgraph = KGraph::from_files(&files);
        let graph = kgraph.to_dep_graph(PathBuf::from("."), files);
        let mut out = Vec::new();
        render_tree(&graph, &kgraph, no_external, show_external, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn render(files: Vec<FileNode>) -> String {
        render_with_flags(files, false, false)
    }

    #[test]
    fn empty_graph_reports_no_entry_points() {
        let out = render(Vec::new());
        assert_eq!(out, "(no entry points found)\n");
    }

    #[test]
    fn fully_cyclic_graph_lists_cycles_instead_of_bailing_out() {
        let out = render(vec![
            file("a.ts", &["b.ts"]),
            file("b.ts", &["c.ts"]),
            file("c.ts", &["a.ts"]),
        ]);
        assert!(
            !out.contains("(no entry points found)"),
            "fully-cyclic graph must not bail out:\n{out}"
        );
        assert!(out.contains("Cycles:"), "missing Cycles section:\n{out}");
        for member in ["a.ts", "b.ts", "c.ts"] {
            assert!(
                out.contains(member),
                "missing cycle member {member}:\n{out}"
            );
        }
    }

    #[test]
    fn rooted_graph_does_not_emit_cycles_section() {
        // main.ts -> a.ts <-> b.ts: a root exists, so the cycle is annotated
        // inline on edges and no standalone Cycles section is rendered.
        let out = render(vec![
            file("main.ts", &["a.ts"]),
            file("a.ts", &["b.ts"]),
            file("b.ts", &["a.ts"]),
        ]);
        assert!(
            out.contains("main.ts  [entry]"),
            "missing root entry:\n{out}"
        );
        assert!(
            !out.contains("Cycles:"),
            "unexpected Cycles section:\n{out}"
        );
    }

    #[test]
    fn no_external_overrides_show_external_leaf_nodes() {
        let mut main = file("main.ts", &["helper.ts"]);
        main.imports.push(external_import("react"));
        let helper = file("helper.ts", &[]);

        let out = render_with_flags(vec![main, helper], true, true);

        assert!(
            out.contains("main.ts  [entry]"),
            "missing root entry:\n{out}"
        );
        assert!(
            !out.contains("react [ext]"),
            "--no-external should suppress external leaves:\n{out}"
        );
    }
}
