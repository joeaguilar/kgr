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
        let local_in = kgraph.in_degree(&file.path);
        let local_out = file
            .imports
            .iter()
            .filter(|i| i.kind == ImportKind::Local && i.resolved.is_some())
            .count();
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
