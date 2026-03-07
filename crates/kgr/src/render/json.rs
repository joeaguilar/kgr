use std::collections::BTreeMap;
use std::io::Write;

use kgr_core::types::{DepGraph, ImportKind};

pub fn render_json(graph: &DepGraph, writer: &mut dyn Write) -> std::io::Result<()> {
    let mut value = serde_json::to_value(graph).map_err(std::io::Error::other)?;

    // Build a per-file map of external package names for convenient agent consumption.
    let ext_deps: BTreeMap<String, Vec<String>> = graph
        .files
        .iter()
        .filter_map(|f| {
            let pkgs: Vec<String> = f
                .imports
                .iter()
                .filter(|i| i.kind == ImportKind::External)
                .map(|i| i.raw.clone())
                .collect();
            if pkgs.is_empty() {
                None
            } else {
                Some((f.path.to_string_lossy().into_owned(), pkgs))
            }
        })
        .collect();

    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "external_deps".to_string(),
            serde_json::to_value(&ext_deps).unwrap(),
        );
    }

    serde_json::to_writer_pretty(writer, &value).map_err(std::io::Error::other)
}
