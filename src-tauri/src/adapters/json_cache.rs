//! JSON index cache, hidden inside the media folder so the stick root stays
//! clean. Every failure path degrades to "no cache", never to an error.

use std::path::{Path, PathBuf};

use crate::adapters::json_file;
use crate::core::paths;
use crate::defaults;
use crate::ports::cache::{
    CacheError, LibraryCache, LibraryCacheData, BUILD_NUMBER, CACHE_VERSION,
};

pub struct JsonCache {
    path: PathBuf,
}

impl JsonCache {
    pub fn in_media_root(media_root: &Path) -> Self {
        Self {
            path: media_root.join(defaults::CACHE_FILE_NAME),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl JsonCache {
    /// Remove a cache this build cannot use, and any left by a former name.
    ///
    /// The stick is the user's, and a file nothing will ever read again is
    /// rubbish we left there. Failure is ignored on purpose: a read-only stick
    /// keeps its old file, which is untidy but harmless.
    fn discard(&self, reason: &str) {
        if let Err(e) = std::fs::remove_file(&self.path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("cache not removed: {e}");
            }
        } else {
            eprintln!("cache discarded ({reason})");
        }
    }

    /// Take out the cache files a former release left in the media folder.
    ///
    /// `names` is a parameter rather than the constant so the guard below can
    /// be shown to hold for entries the constant does not contain — which is
    /// the only version of this worth testing.
    fn remove_legacy_files(&self, names: &[&str]) {
        let Some(dir) = self.path.parent() else {
            return;
        };
        for name in names {
            // The list is ours, but this is the one place in the project that
            // destroys the user's files, and `dir.join` would follow an
            // absolute name or a `..` straight out of the media folder. A name
            // that cannot mean a file directly in this folder is not one we put
            // there, so it is not ours to delete.
            if !paths::is_plain_name(name) {
                eprintln!("cache of a former name ignored, not a plain name: {name}");
                continue;
            }
            let stale = dir.join(name);
            if stale.is_file() {
                if let Err(e) = std::fs::remove_file(&stale) {
                    eprintln!("cache of a former name not removed: {e}");
                }
            }
        }
    }
}

impl LibraryCache for JsonCache {
    fn load(&self) -> Option<LibraryCacheData> {
        self.remove_legacy_files(defaults::LEGACY_CACHE_FILE_NAMES);
        json_file::remove_stale_temporaries(&self.path, "cache");

        let bytes = std::fs::read(&self.path).ok()?;
        let Ok(data) = serde_json::from_slice::<LibraryCacheData>(&bytes) else {
            self.discard("unreadable");
            return None;
        };
        if data.version != CACHE_VERSION {
            self.discard("written by another version of the format");
            return None;
        }
        if data.build != BUILD_NUMBER {
            self.discard("written by another build");
            return None;
        }
        Some(data)
    }

    fn store(&self, data: &LibraryCacheData) -> Result<(), CacheError> {
        let bytes = serde_json::to_vec(data).map_err(|e| CacheError(e.to_string()))?;
        json_file::write_atomically(&self.path, &bytes).map_err(|e| CacheError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{ContentKind, ContentSummary};
    use crate::ports::cache::CacheEntry;
    use tempfile::TempDir;

    fn sample() -> LibraryCacheData {
        let mut data = LibraryCacheData::default();
        data.entries.insert(
            "Movie".to_string(),
            CacheEntry {
                mtime: 42,
                summary: Some(ContentSummary {
                    id: "Movie".to_string(),
                    title: "Movie".to_string(),
                    year: Some(2010),
                    cover: Some("Movie/cover.jpg".to_string()),
                    kind: ContentKind::Movie,
                }),
            },
        );
        data.entries.insert(
            "Empty".to_string(),
            CacheEntry {
                mtime: 7,
                summary: None,
            },
        );
        data
    }

    #[test]
    fn stores_and_loads_the_same_data() {
        let dir = TempDir::new().expect("tempdir");
        let cache = JsonCache::in_media_root(dir.path());
        cache.store(&sample()).expect("store");
        assert_eq!(cache.load(), Some(sample()));
    }

    #[test]
    fn the_cache_file_is_hidden_inside_the_media_folder() {
        let dir = TempDir::new().expect("tempdir");
        let cache = JsonCache::in_media_root(dir.path());
        let name = cache
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .expect("name");
        assert!(name.starts_with('.'), "cache file should be hidden: {name}");
        assert_eq!(cache.path().parent(), Some(dir.path()));
    }

    #[test]
    fn a_missing_cache_reads_as_none() {
        let dir = TempDir::new().expect("tempdir");
        assert_eq!(JsonCache::in_media_root(dir.path()).load(), None);
    }

    #[test]
    fn a_corrupt_cache_reads_as_none() {
        let dir = TempDir::new().expect("tempdir");
        let cache = JsonCache::in_media_root(dir.path());
        std::fs::write(cache.path(), b"{ this is not json").expect("write");
        assert_eq!(cache.load(), None);
    }

    #[test]
    fn a_truncated_cache_reads_as_none() {
        let dir = TempDir::new().expect("tempdir");
        let cache = JsonCache::in_media_root(dir.path());
        cache.store(&sample()).expect("store");
        let bytes = std::fs::read(cache.path()).expect("read");
        std::fs::write(cache.path(), &bytes[..bytes.len() / 2]).expect("truncate");
        assert_eq!(cache.load(), None);
    }

    #[test]
    fn a_cache_from_another_version_is_ignored() {
        let dir = TempDir::new().expect("tempdir");
        let cache = JsonCache::in_media_root(dir.path());
        let mut old = sample();
        old.version = CACHE_VERSION + 1;
        std::fs::write(cache.path(), serde_json::to_vec(&old).expect("json")).expect("write");
        assert_eq!(cache.load(), None);
    }

    #[test]
    fn a_cache_this_build_cannot_read_is_removed_not_just_ignored() {
        // Ignoring it silently leaves an unreadable file on the user's stick
        // for good. It is rewritten by the scan that follows.
        let dir = TempDir::new().expect("tempdir");
        let cache = JsonCache::in_media_root(dir.path());
        let mut old = sample();
        old.version = CACHE_VERSION + 1;
        std::fs::write(cache.path(), serde_json::to_vec(&old).expect("json")).expect("write");

        assert_eq!(cache.load(), None);
        assert!(
            !cache.path().exists(),
            "the unreadable cache should be gone"
        );
    }

    #[test]
    fn a_cache_written_by_another_build_is_discarded() {
        // The build number moves with every release, so a format change nobody
        // remembered to declare cannot outlive the release that made it.
        let dir = TempDir::new().expect("tempdir");
        let cache = JsonCache::in_media_root(dir.path());
        let mut other = sample();
        other.build = BUILD_NUMBER.wrapping_add(1);
        std::fs::write(cache.path(), serde_json::to_vec(&other).expect("json")).expect("write");

        assert_eq!(cache.load(), None);
        assert!(!cache.path().exists());
    }

    #[test]
    fn a_corrupt_cache_is_removed_too() {
        let dir = TempDir::new().expect("tempdir");
        let cache = JsonCache::in_media_root(dir.path());
        std::fs::write(cache.path(), b"{ this is not json").expect("write");

        assert_eq!(cache.load(), None);
        assert!(!cache.path().exists());
    }

    #[test]
    fn a_cache_left_under_a_former_name_is_removed() {
        // The app has been renamed; a stick used with the old one still carries
        // its cache, which nothing will ever read again.
        let dir = TempDir::new().expect("tempdir");
        let legacy = dir.path().join(defaults::LEGACY_CACHE_FILE_NAMES[0]);
        std::fs::write(&legacy, b"{\"version\":2,\"entries\":{}}").expect("write");

        let cache = JsonCache::in_media_root(dir.path());
        assert_eq!(cache.load(), None);
        assert!(!legacy.exists(), "the cache of the old name should be gone");
    }

    #[test]
    fn a_readable_cache_survives_the_cleanup() {
        // The cleanup must not take the file it was meant to keep.
        let dir = TempDir::new().expect("tempdir");
        let legacy = dir.path().join(defaults::LEGACY_CACHE_FILE_NAMES[0]);
        std::fs::write(&legacy, b"{}").expect("write");
        let cache = JsonCache::in_media_root(dir.path());
        cache.store(&sample()).expect("store");

        assert_eq!(cache.load(), Some(sample()));
        assert!(cache.path().exists());
        assert!(!legacy.exists());
    }

    #[test]
    fn a_former_name_that_is_not_a_plain_file_name_is_left_alone() {
        // The list is ours, so today every entry is a bare name. This is about
        // the day someone adds one that is not: `Path::join` throws the whole
        // path away for an absolute argument and normalises nothing for `..`,
        // so a careless entry would reach outside the media folder and delete
        // something that was never ours. It is the only code in the project
        // that destroys the user's files; convention is not enough to hold it.
        let dir = TempDir::new().expect("tempdir");
        let media = dir.path().join("media");
        std::fs::create_dir(&media).expect("mkdir");

        let outside = dir.path().join("not-ours.json");
        std::fs::write(&outside, b"someone else's").expect("write");
        let nested = media.join("Movie");
        std::fs::create_dir(&nested).expect("mkdir");
        std::fs::write(nested.join("cover.jpg"), b"art").expect("write");

        JsonCache::in_media_root(&media).remove_legacy_files(&[
            "../not-ours.json",
            "Movie/cover.jpg",
            "..",
            "",
        ]);

        assert!(outside.exists(), "the cleanup climbed out of the folder");
        assert!(nested.join("cover.jpg").exists(), "it descended into one");
    }

    #[test]
    fn a_half_written_cache_left_by_a_pulled_stick_is_swept_up() {
        // `store` writes beside the target and renames. Pull the stick between
        // the two and the temporary stays for good — the one failure the atomic
        // write exists to survive is the one that litters.
        let dir = TempDir::new().expect("tempdir");
        let orphan = dir.path().join(".reeldrive-cache.4242.tmp");
        std::fs::write(&orphan, b"half a cache").expect("write");

        // Nothing else in the folder is ours, whatever it is called.
        let theirs = dir.path().join("holiday.tmp");
        let looks_close = dir.path().join(".reeldrive-cache-notes.json");
        std::fs::write(&theirs, b"not ours").expect("write");
        std::fs::write(&looks_close, b"not ours either").expect("write");

        let cache = JsonCache::in_media_root(dir.path());
        cache.store(&sample()).expect("store");
        assert_eq!(cache.load(), Some(sample()));

        assert!(!orphan.exists(), "the orphan is still there");
        assert!(cache.path().exists(), "the cache itself was taken");
        assert!(theirs.exists(), "someone else's file was taken");
        assert!(looks_close.exists(), "a near-miss name was taken");
    }

    #[test]
    fn what_this_build_wrote_is_read_back() {
        let dir = TempDir::new().expect("tempdir");
        let cache = JsonCache::in_media_root(dir.path());
        cache.store(&LibraryCacheData::default()).expect("store");
        let loaded = cache.load().expect("loaded");
        assert_eq!(loaded.build, BUILD_NUMBER);
        assert_eq!(loaded.version, CACHE_VERSION);
    }

    #[test]
    fn the_temporary_is_still_swept_up_by_the_cleanup() {
        // Both halves live in `json_file` now, and the thing this still pins is
        // that the cache hands it the same path both times: a stem the sweep
        // does not recognise would litter the user's stick for good.
        let dir = TempDir::new().expect("tempdir");
        let cache = JsonCache::in_media_root(dir.path());
        let orphan = json_file::temp_path(cache.path());
        std::fs::write(&orphan, b"half a cache").expect("write");

        assert_eq!(cache.load(), None);
        assert!(!orphan.exists(), "the sweep no longer matches its own name");
    }

    #[test]
    fn storing_leaves_no_temporary_file_behind() {
        let dir = TempDir::new().expect("tempdir");
        let cache = JsonCache::in_media_root(dir.path());
        cache.store(&sample()).expect("store");
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![defaults::CACHE_FILE_NAME.to_string()]);
    }

    #[cfg(unix)]
    #[test]
    fn storing_on_a_read_only_folder_fails_without_panicking() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("tempdir");
        let media = dir.path().join("media");
        std::fs::create_dir(&media).expect("mkdir");
        std::fs::set_permissions(&media, std::fs::Permissions::from_mode(0o555)).expect("chmod");

        let cache = JsonCache::in_media_root(&media);
        let result = cache.store(&sample());

        // Root ignores the permission bits; only assert when they are enforced.
        if result.is_ok() {
            eprintln!("skipping: permissions not enforced (running as root?)");
        } else {
            assert_eq!(
                cache.load(),
                None,
                "a failed store must not leave a usable cache"
            );
        }

        std::fs::set_permissions(&media, std::fs::Permissions::from_mode(0o755)).expect("restore");
    }
}
