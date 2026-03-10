use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use kgr_core::types::{CallRef, Import, Symbol};

const CACHE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize, Deserialize)]
struct Entry {
    mtime_secs: u64,
    size: u64,
    imports: Vec<Import>,
    #[serde(default)]
    symbols: Vec<Symbol>,
    #[serde(default)]
    calls: Vec<CallRef>,
}

/// Data returned from a cache hit.
pub struct CachedParse {
    pub imports: Vec<Import>,
    pub symbols: Vec<Symbol>,
    pub calls: Vec<CallRef>,
}

/// Persistent per-file parse cache. Keyed on (path_string, mtime_secs, size).
/// Stored as JSON at `.kgr-cache.json` in the scanned root.
/// Automatically invalidated when the kgr version changes.
#[derive(Serialize, Deserialize, Default)]
pub struct ParseCache {
    version: String,
    // String keys required because serde_json only supports string-keyed maps.
    #[serde(default)]
    entries: HashMap<String, Entry>,
}

impl ParseCache {
    /// Load cache from disk. Returns an empty cache on any error or version mismatch.
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<ParseCache>(&b).ok())
            .filter(|c| c.version == CACHE_VERSION)
            .unwrap_or_else(|| Self {
                version: CACHE_VERSION.to_owned(),
                entries: HashMap::new(),
            })
    }

    /// Persist cache to disk. Silently ignores write errors (cache is best-effort).
    pub fn save(&self, path: &Path) {
        if let Ok(bytes) = serde_json::to_vec(self) {
            let _ = std::fs::write(path, bytes);
        }
    }

    /// Returns cached parse data if `path` has matching `mtime` and `size`.
    pub fn get(&self, path: &Path, mtime: Option<SystemTime>, size: u64) -> Option<CachedParse> {
        let mtime_secs = mtime?.duration_since(UNIX_EPOCH).ok()?.as_secs();
        let key = path.to_string_lossy();
        let e = self.entries.get(key.as_ref())?;
        (e.mtime_secs == mtime_secs && e.size == size).then(|| CachedParse {
            imports: e.imports.clone(),
            symbols: e.symbols.clone(),
            calls: e.calls.clone(),
        })
    }

    /// Insert or update a cache entry. No-ops if `mtime` is unavailable.
    pub fn insert(
        &mut self,
        path: PathBuf,
        mtime: Option<SystemTime>,
        size: u64,
        imports: Vec<Import>,
        symbols: Vec<Symbol>,
        calls: Vec<CallRef>,
    ) {
        let Some(mtime_secs) = mtime
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
        else {
            return;
        };
        self.entries.insert(
            path.to_string_lossy().into_owned(),
            Entry {
                mtime_secs,
                size,
                imports,
                symbols,
                calls,
            },
        );
    }
}
