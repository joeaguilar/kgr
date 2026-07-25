//! Path separator normalization for kgr's output contract.

#[cfg(windows)]
use std::path::Component;
use std::path::{Path, PathBuf};

/// Rewrite a relative path so its components are joined with `/` rather than
/// the platform separator.
///
/// kgr's output is a cross-machine contract: JSON `from`/`to` edges, tree
/// lines, orphan lists, `.kgr-baseline.json` entries, and `.kgr-cache.json`
/// keys all name files by path. If a Windows run emits `src\main.rs` where a
/// Linux run emits `src/main.rs`, the same repository produces two
/// incompatible graphs — baselines stop suppressing, caches miss wholesale,
/// and agents consuming the JSON have to guess the separator. Normalizing at
/// the points where paths are *stored* means every renderer downstream
/// inherits one spelling for free.
///
/// This is safe to do only on Windows, where `\` is not a legal filename
/// character. On Unix `\` *is* legal — `a\b.py` is a single file named
/// `a\b.py` — so rewriting it there would invent a directory that does not
/// exist. On Unix this function is the identity.
///
/// Paths that begin with a prefix component (`C:\...`, or a verbatim
/// `\\?\...` path whose separators must stay backslashes) are returned
/// unchanged: only relative paths carry the output contract.
#[cfg(windows)]
pub fn to_slash(path: &Path) -> PathBuf {
    if matches!(path.components().next(), Some(Component::Prefix(_))) {
        return path.to_path_buf();
    }
    let text = path.to_string_lossy();
    if !text.contains('\\') {
        return path.to_path_buf();
    }
    PathBuf::from(text.replace('\\', "/"))
}

#[cfg(not(windows))]
pub fn to_slash(path: &Path) -> PathBuf {
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_use_forward_slashes() {
        // Built the way the walker builds them, so the platform separator is
        // whatever `join` produces.
        let path = Path::new("src").join("models").join("user.rs");

        assert_eq!(to_slash(&path).to_string_lossy(), "src/models/user.rs");
    }

    #[test]
    fn already_normalized_paths_are_unchanged() {
        let path = PathBuf::from("src/main.rs");

        assert_eq!(to_slash(&path), path);
    }

    #[test]
    fn bare_filenames_are_unchanged() {
        let path = PathBuf::from("main.rs");

        assert_eq!(to_slash(&path), path);
    }

    /// On Unix a backslash is part of the filename, not a separator, so
    /// normalization must leave it alone.
    #[cfg(not(windows))]
    #[test]
    fn unix_backslash_filenames_survive_normalization() {
        let path = PathBuf::from("weird\\name.py");

        assert_eq!(to_slash(&path), path);
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_prefixed_paths_keep_their_backslashes() {
        let path = PathBuf::from(r"\\?\C:\repo\src\main.rs");

        assert_eq!(to_slash(&path), path);
    }
}
