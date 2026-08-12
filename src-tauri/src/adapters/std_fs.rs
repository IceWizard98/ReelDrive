//! Real filesystem behind the `FileSystem` port, plus locating the media
//! folder next to the executable.

use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::defaults;
use crate::ports::fs::{DirEntry, FileSystem, FsError};

pub struct StdFs;

fn to_fs_error(path: &Path, e: io::Error) -> FsError {
    match e.kind() {
        io::ErrorKind::NotFound => FsError::NotFound(path.to_path_buf()),
        io::ErrorKind::PermissionDenied => FsError::PermissionDenied(path.to_path_buf()),
        _ => FsError::Other {
            path: path.to_path_buf(),
            message: e.to_string(),
        },
    }
}

impl FileSystem for StdFs {
    fn read_dir(&self, path: &Path) -> Result<Vec<DirEntry>, FsError> {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path).map_err(|e| to_fs_error(path, e))? {
            // A single unreadable entry must not sink the whole directory.
            let Ok(entry) = entry else { continue };
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            // `file_type` does not follow symlinks, so resolve them explicitly:
            // a symlinked season folder should still read as a directory.
            let is_dir = match entry.file_type() {
                Ok(t) if t.is_symlink() => entry.path().is_dir(),
                Ok(t) => t.is_dir(),
                Err(_) => continue,
            };
            entries.push(DirEntry { name, is_dir });
        }
        Ok(entries)
    }

    fn modified_secs(&self, path: &Path) -> Result<u64, FsError> {
        let metadata = std::fs::metadata(path).map_err(|e| to_fs_error(path, e))?;
        let modified = metadata.modified().map_err(|e| to_fs_error(path, e))?;
        modified
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .map_err(|e| FsError::Other {
                path: path.to_path_buf(),
                message: e.to_string(),
            })
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
}

/// Folder the executable is shipped from. On macOS the binary lives inside
/// `ReelDrive.app/Contents/MacOS/`, but the user sees the folder holding
/// the bundle — that is where `media/` must be looked up.
pub fn app_dir_for_exe(exe: &Path) -> PathBuf {
    let parent = exe.parent().unwrap_or(Path::new("."));
    let mut candidate = parent;
    for _ in 0..3 {
        if candidate
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        {
            return candidate.parent().unwrap_or(Path::new(".")).to_path_buf();
        }
        match candidate.parent() {
            Some(next) => candidate = next,
            None => break,
        }
    }
    parent.to_path_buf()
}

pub fn media_root_for_exe(exe: &Path) -> PathBuf {
    app_dir_for_exe(exe).join(defaults::MEDIA_DIR_NAME)
}

/// The executable as the user sees it on the stick.
///
/// An AppImage runs from a temporary read-only mount, so `current_exe()` points
/// at `/tmp/.mount_*/usr/bin/...` and everything looked up beside it — `media/`,
/// the shipped tools — would be looked up inside the mount. The runtime puts
/// the real path of the `.AppImage` file in `$APPIMAGE`, and that is the one
/// that sits next to the user's content.
pub fn portable_exe(current: &Path, appimage: Option<&str>) -> PathBuf {
    match appimage {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => current.to_path_buf(),
    }
}

/// `portable_exe` for the process actually running.
pub fn running_exe() -> Result<PathBuf, io::Error> {
    let current = std::env::current_exe()?;
    let appimage = std::env::var("APPIMAGE").ok();
    Ok(portable_exe(&current, appimage.as_deref()))
}

/// Where the media folder is expected for the running executable.
pub fn media_root() -> Result<PathBuf, io::Error> {
    Ok(media_root_for_exe(&running_exe()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::scanner::{scan_library, ScanError};
    use std::fs;
    use tempfile::TempDir;

    fn tree(files: &[&str]) -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        for path in files {
            let full = dir.path().join(path);
            if path.ends_with('/') {
                fs::create_dir_all(&full).expect("mkdir");
            } else {
                if let Some(parent) = full.parent() {
                    fs::create_dir_all(parent).expect("mkdir");
                }
                fs::write(&full, b"x").expect("write");
            }
        }
        dir
    }

    #[test]
    fn media_root_sits_next_to_a_plain_executable() {
        assert_eq!(
            media_root_for_exe(Path::new("/stick/ReelDrive.exe")),
            PathBuf::from("/stick/media")
        );
        assert_eq!(
            media_root_for_exe(Path::new("/mnt/usb/ReelDrive")),
            PathBuf::from("/mnt/usb/media")
        );
    }

    #[test]
    fn media_root_escapes_the_macos_bundle() {
        assert_eq!(
            media_root_for_exe(Path::new(
                "/Volumes/SD/ReelDrive.app/Contents/MacOS/ReelDrive"
            )),
            PathBuf::from("/Volumes/SD/media")
        );
    }

    #[test]
    fn inside_an_appimage_the_mounted_path_is_ignored() {
        // The running executable lives in a read-only squashfs mount; the
        // stick is where the .AppImage file itself sits.
        assert_eq!(
            portable_exe(
                Path::new("/tmp/.mount_ReelDr12345/usr/bin/reeldrive"),
                Some("/mnt/usb/ReelDrive.AppImage")
            ),
            PathBuf::from("/mnt/usb/ReelDrive.AppImage")
        );
    }

    #[test]
    fn outside_an_appimage_the_running_executable_is_used() {
        assert_eq!(
            portable_exe(Path::new("/mnt/usb/ReelDrive"), None),
            PathBuf::from("/mnt/usb/ReelDrive")
        );
    }

    #[test]
    fn an_empty_appimage_variable_is_not_a_path() {
        assert_eq!(
            portable_exe(Path::new("/mnt/usb/ReelDrive"), Some("")),
            PathBuf::from("/mnt/usb/ReelDrive")
        );
    }

    #[test]
    fn media_root_follows_the_appimage_out_of_its_mount() {
        assert_eq!(
            media_root_for_exe(&portable_exe(
                Path::new("/tmp/.mount_ReelDr12345/usr/bin/reeldrive"),
                Some("/mnt/usb/ReelDrive.AppImage")
            )),
            PathBuf::from("/mnt/usb/media")
        );
    }

    #[test]
    fn read_dir_reports_files_and_directories() {
        let dir = tree(&["Series/Season 1/S01E01.mkv", "Series/cover.jpg"]);
        let mut entries = StdFs
            .read_dir(&dir.path().join("Series"))
            .expect("read_dir");
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], DirEntry::dir("Season 1"));
        assert_eq!(entries[1], DirEntry::file("cover.jpg"));
    }

    #[test]
    fn read_dir_on_a_missing_path_is_not_found() {
        let dir = TempDir::new().expect("tempdir");
        assert_eq!(
            StdFs.read_dir(&dir.path().join("nope")),
            Err(FsError::NotFound(dir.path().join("nope")))
        );
    }

    #[test]
    fn modified_secs_reads_a_plausible_timestamp() {
        let dir = tree(&["Movie/Movie.mkv"]);
        let secs = StdFs
            .modified_secs(&dir.path().join("Movie"))
            .expect("mtime");
        assert!(secs > 1_600_000_000, "timestamp looks wrong: {secs}");
    }

    #[test]
    fn modified_secs_on_a_missing_path_is_not_found() {
        let dir = TempDir::new().expect("tempdir");
        assert!(matches!(
            StdFs.modified_secs(&dir.path().join("nope")),
            Err(FsError::NotFound(_))
        ));
    }

    #[test]
    fn scanning_a_real_tree_classifies_content() {
        let dir = tree(&[
            "Inception (2010)/cover.jpg",
            "Inception (2010)/Inception.mkv",
            "Scrubs/poster.png",
            "Scrubs/Season 1/Scrubs S01E01 - My First Day.mkv",
            "Scrubs/Season 2/Scrubs S02E01.mkv",
            "HIMYM/HIMYM S01E01.mkv",
            "HIMYM/HIMYM S01E02.mkv",
            "Empty/",
        ]);
        let scan = scan_library(&StdFs, dir.path()).expect("scan");
        let ids: Vec<&str> = scan.contents.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["HIMYM", "Inception (2010)", "Scrubs"]);
        assert_eq!(scan.warnings.len(), 1);
        assert!(scan.warnings[0].contains("Empty"));
    }

    #[test]
    fn scanning_a_missing_media_root_is_reported() {
        let dir = TempDir::new().expect("tempdir");
        assert_eq!(
            scan_library(&StdFs, &dir.path().join("media")),
            Err(ScanError::MediaRootMissing)
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_folder_is_skipped_not_fatal() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tree(&["Ok/a.mkv", "Broken/b.mkv"]);
        let broken = dir.path().join("Broken");
        fs::set_permissions(&broken, fs::Permissions::from_mode(0o000)).expect("chmod");

        let scan = scan_library(&StdFs, dir.path()).expect("scan");
        // Running as root ignores the permission bits, so only assert the
        // outcome that actually depends on them.
        if scan.warnings.is_empty() {
            eprintln!("skipping: permissions not enforced (running as root?)");
        } else {
            assert_eq!(scan.contents.len(), 1);
            assert_eq!(scan.contents[0].id, "Ok");
            assert!(scan.warnings[0].contains("Broken"));
        }

        fs::set_permissions(&broken, fs::Permissions::from_mode(0o755)).expect("restore");
    }
}
