use std::path::Path;

use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use kgr_core::parse::ParserRegistry;
use kgr_core::types::FileNode;

use crate::walk::DiscoveredFile;

pub fn parse_all(
    root: &Path,
    files: Vec<DiscoveredFile>,
    registry: &ParserRegistry,
    show_progress: bool,
) -> Vec<FileNode> {
    let progress = if show_progress {
        let pb = ProgressBar::new(files.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} files parsed")
                .unwrap()
                .progress_chars("#>-"),
        );
        Some(pb)
    } else {
        None
    };

    let result: Vec<FileNode> = files
        .par_iter()
        .filter_map(|f| {
            let parser = registry.get(f.lang)?;
            let full_path = root.join(&f.path);
            let source = std::fs::read(&full_path).ok()?;
            let imports = parser.parse(&source, &f.path);
            if let Some(ref pb) = progress {
                pb.inc(1);
            }
            Some(FileNode {
                path: f.path.clone(),
                lang: f.lang,
                imports,
            })
        })
        .collect();

    if let Some(pb) = progress {
        pb.finish_and_clear();
    }

    result
}
