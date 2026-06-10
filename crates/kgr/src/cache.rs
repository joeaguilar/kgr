use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use kgr_core::types::{CallRef, Import, Symbol};

const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Cache version tag: package version plus a fingerprint of the running
/// binary (mtime + size of `current_exe`). Any rebuild of kgr changes the
/// fingerprint and invalidates all caches, so a cache written by an older
/// build can never mask changed parser behavior — even when the package
/// version has not bumped (e.g. a warm `.kgr-cache.json` left in a fixture
/// directory between dev test runs).
static CACHE_VERSION: LazyLock<String> = LazyLock::new(|| {
    let fingerprint = std::env::current_exe()
        .ok()
        .and_then(|exe| std::fs::metadata(exe).ok())
        .map(|meta| {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            format!("{mtime}.{}", meta.len())
        })
        .unwrap_or_default();
    format!("{PKG_VERSION}+{fingerprint}")
});

#[derive(Serialize, Deserialize)]
struct Entry {
    mtime_secs: u64,
    #[serde(default)]
    mtime_nanos: u32,
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

/// Returns true when the `KGR_NO_CACHE` environment variable disables the
/// parse cache (set to any non-empty value other than `0`). With the cache
/// disabled, `load` always returns an empty cache and `save` never writes —
/// every run re-parses sources. The test suite sets this so stale warm caches
/// can never mask a parser regression.
fn cache_disabled() -> bool {
    std::env::var_os("KGR_NO_CACHE").is_some_and(|v| !v.is_empty() && v != "0")
}

/// Persistent per-file parse cache. Keyed on (path_string, mtime, size).
/// Stored as JSON at `.kgr-cache.json` in the scanned root.
/// Automatically invalidated when the kgr version or binary build changes.
/// Disabled entirely when `KGR_NO_CACHE` is set (see [`cache_disabled`]).
#[derive(Serialize, Deserialize, Default)]
pub struct ParseCache {
    version: String,
    // String keys required because serde_json only supports string-keyed maps.
    #[serde(default)]
    entries: HashMap<String, Entry>,
}

fn mtime_parts(mtime: Option<SystemTime>) -> Option<(u64, u32)> {
    let duration = mtime?.duration_since(UNIX_EPOCH).ok()?;
    Some((duration.as_secs(), duration.subsec_nanos()))
}

impl ParseCache {
    /// An empty cache tagged with the current version + build fingerprint.
    fn fresh() -> Self {
        Self {
            version: CACHE_VERSION.clone(),
            entries: HashMap::new(),
        }
    }

    /// Load cache from disk. Returns an empty cache on any error, on
    /// version/build mismatch, or when `KGR_NO_CACHE` disables caching.
    pub fn load(path: &Path) -> Self {
        if cache_disabled() {
            return Self::fresh();
        }
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<ParseCache>(&b).ok())
            .filter(|c| c.version == *CACHE_VERSION)
            .unwrap_or_else(Self::fresh)
    }

    /// Persist cache to disk. Silently ignores write errors (cache is
    /// best-effort). No-ops when `KGR_NO_CACHE` disables caching.
    pub fn save(&self, path: &Path) {
        if cache_disabled() {
            return;
        }
        if let Ok(bytes) = serde_json::to_vec(self) {
            let _ = std::fs::write(path, bytes);
        }
    }

    /// Returns cached parse data if `path` has matching `mtime` and `size`.
    pub fn get(&self, path: &Path, mtime: Option<SystemTime>, size: u64) -> Option<CachedParse> {
        let (mtime_secs, mtime_nanos) = mtime_parts(mtime)?;
        let key = path.to_string_lossy();
        let e = self.entries.get(key.as_ref())?;
        (e.mtime_secs == mtime_secs && e.mtime_nanos == mtime_nanos && e.size == size).then(|| {
            CachedParse {
                imports: e.imports.clone(),
                symbols: e.symbols.clone(),
                calls: e.calls.clone(),
            }
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
        let Some((mtime_secs, mtime_nanos)) = mtime_parts(mtime) else {
            return;
        };
        self.entries.insert(
            path.to_string_lossy().into_owned(),
            Entry {
                mtime_secs,
                mtime_nanos,
                size,
                imports,
                symbols,
                calls,
            },
        );
    }

    /// Drop cache entries for paths that were not discovered in the current walk.
    pub fn retain_paths<P>(&mut self, paths: impl IntoIterator<Item = P>)
    where
        P: AsRef<Path>,
    {
        let live_paths: HashSet<String> = paths
            .into_iter()
            .map(|path| path.as_ref().to_string_lossy().into_owned())
            .collect();
        self.entries.retain(|path, _| live_paths.contains(path));
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use kgr_core::types::ImportKind;

    use super::*;

    fn mtime(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn mtime_with_nanos(secs: u64, nanos: u32) -> SystemTime {
        UNIX_EPOCH + Duration::new(secs, nanos)
    }

    fn sample_import() -> Import {
        Import {
            raw: "helper".to_owned(),
            kind: ImportKind::Local,
            resolved: Some(PathBuf::from("helper.py")),
            span: None,
        }
    }

    fn warm_cache(path: &Path) -> ParseCache {
        let mut cache = ParseCache::fresh();
        cache.insert(
            path.to_path_buf(),
            Some(mtime(100)),
            42,
            vec![sample_import()],
            Vec::new(),
            Vec::new(),
        );
        cache
    }

    #[test]
    fn get_hits_on_matching_path_mtime_and_size() {
        let path = PathBuf::from("src/main.py");
        let cache = warm_cache(&path);

        let hit = cache
            .get(&path, Some(mtime(100)), 42)
            .expect("expected cache hit");
        assert_eq!(hit.imports.len(), 1);
        assert_eq!(hit.imports[0].raw, "helper");
        assert!(hit.symbols.is_empty());
        assert!(hit.calls.is_empty());
    }

    #[test]
    fn get_misses_when_mtime_changes() {
        let path = PathBuf::from("src/main.py");
        let cache = warm_cache(&path);

        assert!(
            cache.get(&path, Some(mtime(101)), 42).is_none(),
            "an mtime change must invalidate the entry"
        );
    }

    #[test]
    fn get_misses_when_same_second_same_size_mtime_nanos_change() {
        let path = PathBuf::from("src/main.py");
        let mut cache = ParseCache::fresh();
        cache.insert(
            path.clone(),
            Some(mtime_with_nanos(100, 123)),
            42,
            vec![sample_import()],
            Vec::new(),
            Vec::new(),
        );

        assert!(
            cache
                .get(&path, Some(mtime_with_nanos(100, 456)), 42)
                .is_none(),
            "a same-second same-size edit must invalidate the entry"
        );
    }

    #[test]
    fn get_misses_when_size_changes() {
        let path = PathBuf::from("src/main.py");
        let cache = warm_cache(&path);

        assert!(
            cache.get(&path, Some(mtime(100)), 43).is_none(),
            "a size change must invalidate the entry"
        );
    }

    #[test]
    fn get_misses_when_mtime_unavailable() {
        let path = PathBuf::from("src/main.py");
        let cache = warm_cache(&path);

        assert!(
            cache.get(&path, None, 42).is_none(),
            "an unavailable mtime must never produce a hit"
        );
    }

    #[test]
    fn retain_paths_drops_entries_not_seen_in_current_walk() {
        let live = PathBuf::from("src/main.py");
        let stale = PathBuf::from("src/deleted.py");
        let mut cache = warm_cache(&live);
        cache.insert(
            stale.clone(),
            Some(mtime(100)),
            42,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );

        cache.retain_paths(std::iter::once(live.as_path()));

        assert!(
            cache.get(&live, Some(mtime(100)), 42).is_some(),
            "a path discovered in the current walk must stay cached"
        );
        assert!(
            cache.get(&stale, Some(mtime(100)), 42).is_none(),
            "a path missing from the current walk must be pruned"
        );
    }

    #[test]
    fn save_then_load_preserves_entries_for_current_version() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join(".kgr-cache.json");
        let path = PathBuf::from("src/main.py");
        warm_cache(&path).save(&cache_path);

        let loaded = ParseCache::load(&cache_path);
        let hit = loaded
            .get(&path, Some(mtime(100)), 42)
            .expect("round-tripped entry should hit");
        assert_eq!(hit.imports[0].raw, "helper");
    }

    #[test]
    fn load_discards_cache_written_by_a_different_version() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join(".kgr-cache.json");
        let path = PathBuf::from("src/main.py");
        let mut stale = warm_cache(&path);
        stale.version = "0.0.0-stale".to_owned();
        stale.save(&cache_path);

        let loaded = ParseCache::load(&cache_path);
        assert!(
            loaded.get(&path, Some(mtime(100)), 42).is_none(),
            "entries from a different kgr version must be discarded"
        );
        assert_eq!(loaded.version, *CACHE_VERSION);
    }

    #[test]
    fn load_discards_cache_written_by_a_different_build_of_same_version() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join(".kgr-cache.json");
        let path = PathBuf::from("src/main.py");
        let mut stale = warm_cache(&path);
        // Same package version, different (older) build fingerprint — the
        // exact shape of a cache left behind by a pre-regression dev build.
        stale.version = format!("{PKG_VERSION}+stale-build");
        stale.save(&cache_path);

        let loaded = ParseCache::load(&cache_path);
        assert!(
            loaded.get(&path, Some(mtime(100)), 42).is_none(),
            "entries from a different build of the same version must be discarded"
        );
    }

    #[test]
    fn cache_version_includes_a_build_fingerprint() {
        let fingerprint = CACHE_VERSION
            .split_once('+')
            .map(|(_, fp)| fp)
            .unwrap_or_default();
        assert!(
            !fingerprint.is_empty(),
            "CACHE_VERSION should carry a build fingerprint, got {:?}",
            *CACHE_VERSION
        );
        assert!(CACHE_VERSION.starts_with(PKG_VERSION));
    }

    #[test]
    fn load_returns_empty_cache_on_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join(".kgr-cache.json");
        std::fs::write(&cache_path, b"{ not json").unwrap();

        let loaded = ParseCache::load(&cache_path);
        assert!(loaded.entries.is_empty());
        assert_eq!(loaded.version, *CACHE_VERSION);
    }
}
