use std::io::Write;

use kgr_core::types::DepGraph;

pub fn render_json(graph: &DepGraph, writer: &mut dyn Write) -> std::io::Result<()> {
    serde_json::to_writer_pretty(writer, graph)
        .map_err(std::io::Error::other)
}
