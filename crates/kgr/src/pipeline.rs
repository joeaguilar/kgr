use std::path::{Path, PathBuf};

use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use kgr_core::parse::ParserRegistry;
use kgr_core::types::{FileNode, Lang};

use crate::cache::ParseCache;
use crate::walk::DiscoveredFile;

const PARSE_FAILURE_SAMPLE_LIMIT: usize = 5;

/// Parse discovered files into graph nodes.
///
/// Exit behavior: unreadable files and files without a registered parser are
/// skipped as non-fatal failures, summarized with a warning after parallel
/// parsing, and do not make the CLI fail by themselves.
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

    let parsed: Vec<ParseMissResult> = misses
        .par_iter()
        .map(|(i, f)| {
            let result = parse_miss(root, *i, f, registry);
            if let Some(ref pb) = progress {
                pb.inc(1);
            }
            result
        })
        .collect();

    if let Some(pb) = progress {
        pb.finish_and_clear();
    }

    // ── Phase 3: update cache (serial) and merge results ────────────────────
    let mut failures = Vec::new();
    for result in parsed {
        match result {
            ParseMissResult::Parsed(i, node) => {
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
            ParseMissResult::Failed(failure) => failures.push(failure),
        }
    }
    emit_parse_failure_summary(&failures);

    ordered.into_iter().flatten().collect()
}

enum ParseMissResult {
    Parsed(usize, FileNode),
    Failed(ParseFailure),
}

#[derive(Debug)]
enum ParseFailure {
    MissingParser { path: PathBuf, lang: Lang },
    Read { path: PathBuf, error: String },
}

impl ParseFailure {
    fn summary(&self) -> String {
        match self {
            Self::MissingParser { path, lang } => {
                format!("{} (no registered parser for {lang})", path.display())
            }
            Self::Read { path, error } => {
                format!("{} (read error: {error})", path.display())
            }
        }
    }
}

fn parse_miss(
    root: &Path,
    index: usize,
    file: &DiscoveredFile,
    registry: &ParserRegistry,
) -> ParseMissResult {
    let Some(parser) = registry.get(file.lang) else {
        return ParseMissResult::Failed(ParseFailure::MissingParser {
            path: file.path.clone(),
            lang: file.lang,
        });
    };

    let full_path = root.join(&file.path);
    let source = match std::fs::read(&full_path) {
        Ok(source) => source,
        Err(error) => {
            return ParseMissResult::Failed(ParseFailure::Read {
                path: file.path.clone(),
                error: error.to_string(),
            });
        }
    };

    let imports = parser.parse(&source, &file.path);
    let symbols = parser.extract_symbols(&source, &file.path);
    let calls = parser.extract_calls(&source, &file.path);
    ParseMissResult::Parsed(
        index,
        FileNode {
            path: file.path.clone(),
            lang: file.lang,
            imports,
            symbols,
            calls,
        },
    )
}

fn emit_parse_failure_summary(failures: &[ParseFailure]) {
    if let Some(summary) = parse_failure_summary(failures) {
        tracing::warn!("{summary}");
    }
}

fn parse_failure_summary(failures: &[ParseFailure]) -> Option<String> {
    if failures.is_empty() {
        return None;
    }

    let samples: Vec<String> = failures
        .iter()
        .take(PARSE_FAILURE_SAMPLE_LIMIT)
        .map(ParseFailure::summary)
        .collect();
    let omitted = failures.len().saturating_sub(samples.len());
    let omitted = if omitted == 0 {
        String::new()
    } else {
        format!("; {omitted} more omitted")
    };

    Some(format!(
        "skipped {} file(s) during parse; continuing with successfully parsed files (non-fatal). First {}: {}{}",
        failures.len(),
        samples.len(),
        samples.join("; "),
        omitted
    ))
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
    fn parse_failure_summary_reports_count_samples_and_non_fatal_behavior() {
        let failures: Vec<ParseFailure> = (0..6)
            .map(|i| ParseFailure::Read {
                path: PathBuf::from(format!("src/file{i}.py")),
                error: "permission denied".to_string(),
            })
            .collect();

        let summary = parse_failure_summary(&failures).unwrap();

        assert!(summary.contains("skipped 6 file(s) during parse"));
        assert!(summary.contains("non-fatal"));
        assert!(summary.contains("src/file0.py"));
        assert!(summary.contains("src/file4.py"));
        assert!(!summary.contains("src/file5.py"));
        assert!(summary.contains("1 more omitted"));
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
