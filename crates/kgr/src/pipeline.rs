use std::path::Path;

use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use kgr_core::parse::ParserRegistry;
use kgr_core::types::FileNode;

use crate::cache::ParseCache;
use crate::walk::DiscoveredFile;

pub fn parse_all(
    root: &Path,
    files: Vec<DiscoveredFile>,
    registry: &ParserRegistry,
    cache: &mut ParseCache,
    show_progress: bool,
) -> Vec<FileNode> {
    cache.retain_paths(files.iter().map(|f| f.path.as_path()));

    // ── Phase 1: split into cache hits and misses ───────────────────────────
    let mut ordered: Vec<Option<FileNode>> = vec![None; files.len()];
    let mut misses: Vec<(usize, &DiscoveredFile)> = Vec::new();

    for (i, f) in files.iter().enumerate() {
        if let Some(cached) = cache.get(&f.path, f.mtime, f.size) {
            ordered[i] = Some(FileNode {
                path: f.path.clone(),
                lang: f.lang,
                imports: cached.imports,
                symbols: cached.symbols,
                calls: cached.calls,
            });
        } else {
            misses.push((i, f));
        }
    }

    if misses.is_empty() {
        return ordered.into_iter().flatten().collect();
    }

    // ── Phase 2: parse misses in parallel ───────────────────────────────────
    let progress = if show_progress {
        let pb = ProgressBar::new(misses.len() as u64);
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

    let parsed: Vec<(usize, FileNode)> = misses
        .par_iter()
        .filter_map(|(i, f)| {
            let parser = registry.get(f.lang)?;
            let full_path = root.join(&f.path);
            let source = match std::fs::read(&full_path) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("skipping unreadable file {}: {}", full_path.display(), e);
                    return None;
                }
            };
            let imports = parser.parse(&source, &f.path);
            let symbols = parser.extract_symbols(&source, &f.path);
            let calls = parser.extract_calls(&source, &f.path);
            if let Some(ref pb) = progress {
                pb.inc(1);
            }
            Some((
                *i,
                FileNode {
                    path: f.path.clone(),
                    lang: f.lang,
                    imports,
                    symbols,
                    calls,
                },
            ))
        })
        .collect();

    if let Some(pb) = progress {
        pb.finish_and_clear();
    }

    // ── Phase 3: update cache (serial) and merge results ────────────────────
    for (i, node) in parsed {
        let f = &files[i];
        cache.insert(
            f.path.clone(),
            f.mtime,
            f.size,
            node.imports.clone(),
            node.symbols.clone(),
            node.calls.clone(),
        );
        ordered[i] = Some(node);
    }

    ordered.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use kgr_core::types::Lang;

    use super::*;

    fn mtime(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn parse_all_prunes_cache_entries_missing_from_current_walk_before_early_return() {
        let root = tempfile::tempdir().unwrap();
        let live = PathBuf::from("src/live.py");
        let stale = PathBuf::from("src/deleted.py");
        let mut cache = ParseCache::load(&root.path().join(".kgr-cache.json"));

        cache.insert(
            live.clone(),
            Some(mtime(100)),
            42,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        cache.insert(
            stale.clone(),
            Some(mtime(100)),
            42,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let before_len = serde_json::to_vec(&cache).unwrap().len();

        let nodes = parse_all(
            root.path(),
            vec![DiscoveredFile {
                path: live.clone(),
                lang: Lang::Python,
                mtime: Some(mtime(100)),
                size: 42,
            }],
            &ParserRegistry::new(),
            &mut cache,
            false,
        );
        let after_len = serde_json::to_vec(&cache).unwrap().len();

        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].path, live);
        assert!(
            cache.get(&stale, Some(mtime(100)), 42).is_none(),
            "entries for files missing from the current walk must be pruned"
        );
        assert!(
            after_len < before_len,
            "serialized cache should shrink after pruning a deleted source"
        );
    }
}
