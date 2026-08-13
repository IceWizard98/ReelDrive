//! Turning a relative path coming from the frontend into an absolute one.
//! This is a trust boundary: the webview must never be able to name a file
//! outside the media folder.

use std::fmt;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    Empty,
    Escapes(String),
    NotAFile(String),
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathError::Empty => write!(f, "empty path"),
            PathError::Escapes(p) => write!(f, "path outside the media folder: {p}"),
            PathError::NotAFile(p) => write!(f, "file not found: {p}"),
        }
    }
}

impl std::error::Error for PathError {}

/// A name that can only mean a file directly inside a folder we already own.
///
/// `Path::join` is the reason this has to exist: an absolute argument replaces
/// the whole path and `..` is never normalised away, so one careless name turns
/// "inside this folder" into anywhere on the disk. Used where a name reaches a
/// directory of ours from somewhere else — an HLS segment request off the
/// network, and the list of cache files left by former releases.
pub fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Resolve a `/`-separated path relative to the media root, rejecting anything
/// that is absolute, contains `..`, or is not an existing file.
pub fn resolve_media_file(media_root: &Path, relative: &str) -> Result<PathBuf, PathError> {
    let resolved = resolve_media_path(media_root, relative)?;
    if !resolved.is_file() {
        return Err(PathError::NotAFile(relative.to_string()));
    }
    // The checks above are syntactic and a link has nothing suspicious to see:
    // `is_file()` follows one, and `std_fs` resolves linked directories on
    // purpose, so a whole tree can enter the library under a relative path with
    // no `..` in it. The boundary this file promises only holds if the real
    // file lives under the real root.
    let (real, real_root) = match (resolved.canonicalize(), media_root.canonicalize()) {
        (Ok(real), Ok(root)) => (real, root),
        // A path that cannot be resolved is not one we are willing to serve.
        _ => return Err(PathError::NotAFile(relative.to_string())),
    };
    if !real.starts_with(&real_root) {
        return Err(PathError::Escapes(relative.to_string()));
    }
    // Deliberately the path the caller named, not the target: ffmpeg and the
    // stream server must work with the file the user sees. The root is
    // canonicalised on every call because it moves — on macOS it usually
    // arrives through `/Volumes`, which is itself a link.
    Ok(resolved)
}

/// Same checks without touching the disk, so the rules can be tested on their own.
pub fn resolve_media_path(media_root: &Path, relative: &str) -> Result<PathBuf, PathError> {
    if relative.trim().is_empty() {
        return Err(PathError::Empty);
    }
    if !is_safe_relative(relative) {
        return Err(PathError::Escapes(relative.to_string()));
    }

    Ok(media_root.join(Path::new(relative)))
}

/// Whether a path could only ever mean a file below the media root.
///
/// The syntactic half of `resolve_media_path`, split out because the watch
/// history keys its marks by these paths and has to reject the same strings
/// without a root to resolve them against. Two copies of this rule would be two
/// chances to disagree, and the disagreement would be a hole.
pub fn is_safe_relative(relative: &str) -> bool {
    if relative.trim().is_empty() {
        return false;
    }
    let has_unsafe_component = Path::new(relative)
        .components()
        .any(|component| !matches!(component, Component::Normal(_)));
    // Windows accepts both separators, so a backslash could smuggle a `..`
    // past a `/`-only check.
    !has_unsafe_component && !relative.contains('\\') && !relative.contains('\0')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/stick/media")
    }

    #[test]
    fn a_plain_relative_path_resolves_under_the_root() {
        assert_eq!(
            resolve_media_path(&root(), "Scrubs/Season 1/S01E01.mkv"),
            Ok(PathBuf::from("/stick/media/Scrubs/Season 1/S01E01.mkv"))
        );
    }

    #[test]
    fn unicode_and_spaces_are_fine() {
        assert_eq!(
            resolve_media_path(&root(), "Amélie (2001)/Amélie.mkv"),
            Ok(PathBuf::from("/stick/media/Amélie (2001)/Amélie.mkv"))
        );
    }

    #[test]
    fn traversal_is_rejected() {
        for evil in [
            "../secrets.txt",
            "Scrubs/../../etc/passwd",
            "..",
            "./../x",
            "Scrubs\\..\\..\\windows\\system32",
        ] {
            assert_eq!(
                resolve_media_path(&root(), evil),
                Err(PathError::Escapes(evil.to_string())),
                "path: {evil}"
            );
        }
    }

    #[test]
    fn absolute_paths_are_rejected() {
        for evil in ["/etc/passwd", "/", "//server/share/x.mkv"] {
            assert_eq!(
                resolve_media_path(&root(), evil),
                Err(PathError::Escapes(evil.to_string())),
                "path: {evil}"
            );
        }
    }

    #[test]
    fn empty_paths_are_rejected() {
        assert_eq!(resolve_media_path(&root(), ""), Err(PathError::Empty));
        assert_eq!(resolve_media_path(&root(), "   "), Err(PathError::Empty));
    }

    #[test]
    fn embedded_nul_is_rejected() {
        let evil = "Movie/Movie.mkv\0.txt";
        assert_eq!(
            resolve_media_path(&root(), evil),
            Err(PathError::Escapes(evil.to_string()))
        );
    }

    #[test]
    fn a_missing_file_is_reported() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        assert_eq!(
            resolve_media_file(dir.path(), "missing.mkv"),
            Err(PathError::NotAFile("missing.mkv".to_string()))
        );
    }

    #[test]
    fn a_directory_is_not_a_playable_file() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        std::fs::create_dir(dir.path().join("Movie")).expect("mkdir");
        assert_eq!(
            resolve_media_file(dir.path(), "Movie"),
            Err(PathError::NotAFile("Movie".to_string()))
        );
    }

    #[test]
    fn an_existing_file_resolves() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        std::fs::create_dir(dir.path().join("Movie")).expect("mkdir");
        std::fs::write(dir.path().join("Movie/Movie.mkv"), b"x").expect("write");
        assert_eq!(
            resolve_media_file(dir.path(), "Movie/Movie.mkv"),
            Ok(dir.path().join("Movie/Movie.mkv"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_pointing_out_of_the_root_is_rejected() {
        // The syntactic checks never see a link: `is_file()` follows one, and
        // the scanner resolves linked directories on purpose, so a link is the
        // one way a name with no `..` in it still names a file elsewhere.
        let dir = tempfile::TempDir::new().expect("tempdir");
        let media = dir.path().join("media");
        std::fs::create_dir(&media).expect("mkdir");
        let secret = dir.path().join("secrets.txt");
        std::fs::write(&secret, b"not the user's film").expect("write");
        std::os::unix::fs::symlink(&secret, media.join("Movie.mkv")).expect("symlink");

        assert_eq!(
            resolve_media_file(&media, "Movie.mkv"),
            Err(PathError::Escapes("Movie.mkv".to_string()))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_file_inside_a_linked_directory_that_leaves_the_root_is_rejected() {
        // How it actually happens: `ln -s ~/Documents media/Archive` puts a
        // whole tree in the library, and every file under it resolves through
        // a relative path with nothing suspicious in it.
        let dir = tempfile::TempDir::new().expect("tempdir");
        let media = dir.path().join("media");
        let outside = dir.path().join("Documents");
        std::fs::create_dir(&media).expect("mkdir");
        std::fs::create_dir(&outside).expect("mkdir");
        std::fs::write(outside.join("taxes.mkv"), b"x").expect("write");
        std::os::unix::fs::symlink(&outside, media.join("Archive")).expect("symlink");

        assert_eq!(
            resolve_media_file(&media, "Archive/taxes.mkv"),
            Err(PathError::Escapes("Archive/taxes.mkv".to_string()))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_that_stays_inside_the_root_still_resolves() {
        // The check is "does the real file live under the real root", not "is
        // there a link involved": a user who organises the stick with links
        // must keep a working library.
        let dir = tempfile::TempDir::new().expect("tempdir");
        let media = dir.path().join("media");
        std::fs::create_dir(&media).expect("mkdir");
        std::fs::create_dir(media.join("Films")).expect("mkdir");
        std::fs::write(media.join("Films/Movie.mkv"), b"x").expect("write");
        std::os::unix::fs::symlink(media.join("Films/Movie.mkv"), media.join("Latest.mkv"))
            .expect("symlink");

        assert_eq!(
            resolve_media_file(&media, "Latest.mkv"),
            Ok(media.join("Latest.mkv")),
            "the path handed on must be the one the user sees, not the target"
        );
    }
}
