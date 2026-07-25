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

/// Normalize an *absolute* path for display and serialization.
///
/// Unlike [`to_slash`], which deliberately leaves prefixed paths alone, this
/// also strips the Windows extended-length (`\\?\`) prefix that
/// `std::fs::canonicalize` returns. kgr canonicalizes the scan root before
/// walking it, so without this the JSON `root` field on Windows reads
/// `\\?\D:\repo\src` — a verbatim path that is correct for the Win32 API but
/// noise to anyone consuming the output, and inconsistent with the `/`-spelled
/// relative paths in the same document.
///
/// The stripped path is kept only for output. The canonical form is still what
/// gets handed to the filesystem, so extended-length paths continue to work for
/// walking and reading; only the recorded spelling is simplified.
///
/// On Unix this is the identity, for the same reason [`to_slash`] is.
#[cfg(windows)]
pub fn to_display_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let stripped = match text.strip_prefix(r"\\?\") {
        // `\\?\UNC\server\share` denotes the network path `\\server\share`.
        Some(rest) => match rest.strip_prefix(r"UNC\") {
            Some(unc) => format!(r"\\{unc}"),
            None => rest.to_string(),
        },
        None => text.to_string(),
    };
    PathBuf::from(stripped.replace('\\', "/"))
}

#[cfg(not(windows))]
pub fn to_display_path(path: &Path) -> PathBuf {
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

    /// `to_display_path` is the identity on Unix, where canonicalize returns a
    /// plain absolute path already.
    #[cfg(not(windows))]
    #[test]
    fn display_path_leaves_unix_absolute_paths_alone() {
        let path = PathBuf::from("/repo/tests/fixtures/python/simple");

        assert_eq!(to_display_path(&path), path);
    }

    /// The canonicalized scan root on Windows is verbatim-prefixed; output must
    /// not carry that through.
    #[cfg(windows)]
    #[test]
    fn display_path_strips_the_verbatim_prefix() {
        let path = PathBuf::from(r"\\?\D:\repo\tests\fixtures\python\simple");

        assert_eq!(
            to_display_path(&path).to_string_lossy(),
            "D:/repo/tests/fixtures/python/simple"
        );
    }

    #[cfg(windows)]
    #[test]
    fn display_path_rewrites_verbatim_unc_to_a_network_path() {
        let path = PathBuf::from(r"\\?\UNC\server\share\src\main.rs");

        assert_eq!(
            to_display_path(&path).to_string_lossy(),
            "//server/share/src/main.rs"
        );
    }

    #[cfg(windows)]
    #[test]
    fn display_path_normalizes_plain_drive_paths_too() {
        let path = PathBuf::from(r"D:\repo\src\main.rs");

        assert_eq!(
            to_display_path(&path).to_string_lossy(),
            "D:/repo/src/main.rs"
        );
    }
}
