//! The watch history as a JSON file inside the media folder.
//!
//! Same atomic write as the cache — they share `json_file` — and the opposite
//! policy about failure. Nothing here deletes anything the user could want
//! back.

use std::path::{Path, PathBuf};

use crate::adapters::json_file;
use crate::core::progress::{ProgressData, PROGRESS_VERSION};
use crate::defaults;
use crate::ports::progress::{ProgressError, ProgressStore};

pub struct JsonProgress {
    path: PathBuf,
    set_aside: PathBuf,
}

impl JsonProgress {
    pub fn in_media_root(media_root: &Path) -> Self {
        Self {
            path: media_root.join(defaults::PROGRESS_FILE_NAME),
            set_aside: media_root.join(defaults::PROGRESS_SET_ASIDE_FILE_NAME),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Move a history this build cannot read out of the way, keeping it.
    ///
    /// The cache answers the same situation by deleting the file, and that is
    /// right for a cache: it can be rebuilt by walking the stick. This cannot be
    /// rebuilt by anything, so the worst thing to do with a file we do not
    /// understand is destroy it — a version *newer* than this build is exactly
    /// what an older copy of the app on the same stick would see, and deleting
    /// it would wipe the history every time the two were used in turn.
    ///
    /// An earlier set-aside file is replaced. Keeping a numbered pile of
    /// unreadable histories on someone's stick helps nobody; the one being kept
    /// is the most recent, which is the one closest to what they had.
    fn set_aside(&self, reason: &str) {
        match std::fs::rename(&self.path, &self.set_aside) {
            Ok(()) => eprintln!(
                "watch history set aside as {} ({reason})",
                defaults::PROGRESS_SET_ASIDE_FILE_NAME
            ),
            // A read-only stick cannot move it either. Leaving it where it is
            // means this build reads no history and writes none — nothing is
            // lost, which is the promise that matters.
            Err(e) => eprintln!("watch history could not be set aside: {e}"),
        }
    }
}

impl ProgressStore for JsonProgress {
    fn load(&self) -> ProgressData {
        json_file::remove_stale_temporaries(&self.path, "watch history");

        let Ok(bytes) = std::fs::read(&self.path) else {
            // Missing is the ordinary case: nothing has been watched yet.
            return ProgressData::default();
        };
        let Ok(data) = serde_json::from_slice::<ProgressData>(&bytes) else {
            self.set_aside("unreadable");
            return ProgressData::default();
        };
        if data.version != PROGRESS_VERSION {
            self.set_aside("written in a format this build does not know");
            return ProgressData::default();
        }
        data.sanitized()
    }

    fn save(&self, data: &ProgressData) -> Result<(), ProgressError> {
        let bytes = serde_json::to_vec(data).map_err(|e| ProgressError(e.to_string()))?;
        json_file::write_atomically(&self.path, &bytes).map_err(|e| ProgressError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::progress::Mark;
    use tempfile::TempDir;

    const NOW: u64 = 1_765_000_000;

    fn sample() -> ProgressData {
        let mut data = ProgressData::default();
        data.record("Scrubs/S01/e1.mkv", 812.4, 1320.0, NOW)
            .expect("recorded");
        data.record("Inception (2010)/Inception.mkv", 5900.0, 6000.0, NOW + 5)
            .expect("recorded");
        data
    }

    #[test]
    fn stores_and_loads_the_same_history() {
        let dir = TempDir::new().expect("tempdir");
        let store = JsonProgress::in_media_root(dir.path());
        store.save(&sample()).expect("save");
        assert_eq!(store.load(), sample());
    }

    #[test]
    fn the_history_is_hidden_inside_the_media_folder() {
        let dir = TempDir::new().expect("tempdir");
        let store = JsonProgress::in_media_root(dir.path());
        assert_eq!(store.path(), dir.path().join(".reeldrive-progress.json"));
    }

    #[test]
    fn a_stick_with_no_history_yet_reads_as_an_empty_one() {
        let dir = TempDir::new().expect("tempdir");
        assert_eq!(
            JsonProgress::in_media_root(dir.path()).load(),
            ProgressData::default()
        );
    }

    #[test]
    fn an_unreadable_history_is_kept_where_the_cache_would_have_been_deleted() {
        // The difference between the two files, and the reason this one exists
        // separately: a cache can be rebuilt by walking the stick, and this
        // cannot be rebuilt by anything.
        let dir = TempDir::new().expect("tempdir");
        let store = JsonProgress::in_media_root(dir.path());
        std::fs::write(store.path(), b"{ half a fi").expect("write");

        assert_eq!(store.load(), ProgressData::default());

        let kept = dir.path().join(".reeldrive-progress.unreadable.json");
        assert!(!store.path().exists(), "the broken file was left in place");
        assert_eq!(std::fs::read(&kept).expect("read"), b"{ half a fi");
    }

    #[test]
    fn a_history_from_a_newer_build_is_set_aside_rather_than_overwritten() {
        // Two copies of the app on one stick — a release and an older one — is
        // the ordinary way this happens. Deleting would mean each wiped the
        // other's history every time they were used in turn.
        let dir = TempDir::new().expect("tempdir");
        let store = JsonProgress::in_media_root(dir.path());
        let future =
            br#"{"version":99,"items":{"A/b.mkv":{"seconds":1,"duration":2,"done":false,"at":1}}}"#;
        std::fs::write(store.path(), future).expect("write");

        assert_eq!(store.load(), ProgressData::default());

        let kept = dir.path().join(".reeldrive-progress.unreadable.json");
        assert_eq!(std::fs::read(&kept).expect("read"), future);
    }

    #[test]
    fn a_hand_edited_history_is_cleaned_on_the_way_in() {
        // The file sits on the user's stick and its keys are used as paths, so
        // the guard has to be applied where it is read, not only where it is
        // written.
        let dir = TempDir::new().expect("tempdir");
        let store = JsonProgress::in_media_root(dir.path());
        let edited = br#"{"version":1,"items":{
            "../../etc/passwd":{"seconds":1,"duration":2,"done":false,"at":1},
            "Scrubs/S01/e1.mkv":{"seconds":10,"duration":20,"done":false,"at":1}}}"#;
        std::fs::write(store.path(), edited).expect("write");

        let loaded = store.load();
        assert_eq!(
            loaded.items.keys().collect::<Vec<_>>(),
            ["Scrubs/S01/e1.mkv"]
        );
        assert!(
            store.path().exists(),
            "a file with one bad row is not an unreadable file"
        );
    }

    #[test]
    fn a_history_survives_the_cache_being_thrown_away() {
        // The whole reason for a second file. The cache deletes itself on every
        // release and sweeps its own temporaries; neither may reach this.
        use crate::adapters::json_cache::JsonCache;
        use crate::ports::cache::LibraryCache;

        let dir = TempDir::new().expect("tempdir");
        let store = JsonProgress::in_media_root(dir.path());
        store.save(&sample()).expect("save");

        let cache = JsonCache::in_media_root(dir.path());
        std::fs::write(cache.path(), b"not a cache").expect("write");
        assert_eq!(cache.load(), None, "the cache should have discarded itself");
        assert!(!cache.path().exists());

        assert_eq!(store.load(), sample(), "the history went with the cache");
    }

    #[test]
    fn saving_leaves_no_temporary_file_behind() {
        let dir = TempDir::new().expect("tempdir");
        let store = JsonProgress::in_media_root(dir.path());
        store.save(&sample()).expect("save");

        let names: Vec<String> = std::fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![defaults::PROGRESS_FILE_NAME.to_string()]);
    }

    #[test]
    fn a_temporary_left_by_a_pulled_stick_is_swept_up() {
        let dir = TempDir::new().expect("tempdir");
        let store = JsonProgress::in_media_root(dir.path());
        let orphan = json_file::temp_path(store.path());
        std::fs::write(&orphan, b"half a history").expect("write");

        store.load();
        assert!(!orphan.exists(), "the sweep no longer matches its own name");
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_stick_reports_the_failure_instead_of_panicking() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("tempdir");
        let media = dir.path().join("media");
        std::fs::create_dir(&media).expect("mkdir");
        std::fs::set_permissions(&media, std::fs::Permissions::from_mode(0o555)).expect("chmod");

        let store = JsonProgress::in_media_root(&media);
        let result = store.save(&sample());

        // Root ignores the permission bits; only assert when they are enforced.
        if result.is_ok() {
            eprintln!("skipping: permissions not enforced (running as root?)");
        } else {
            assert_eq!(
                store.load(),
                ProgressData::default(),
                "a failed save must not leave a half-written history"
            );
        }

        std::fs::set_permissions(&media, std::fs::Permissions::from_mode(0o755)).expect("restore");
    }

    #[test]
    fn a_mark_survives_the_round_trip_with_its_numbers_intact() {
        // Serialising the position as anything but a float would round a
        // thirteen-and-a-half-minute position to the minute.
        let dir = TempDir::new().expect("tempdir");
        let store = JsonProgress::in_media_root(dir.path());
        let mut data = ProgressData::default();
        data.record("A/b.mkv", 812.4, 1320.0, NOW)
            .expect("recorded");
        store.save(&data).expect("save");

        assert_eq!(
            store.load().items["A/b.mkv"],
            Mark {
                seconds: 812.4,
                duration: 1320.0,
                done: false,
                at: NOW,
            }
        );
    }
}
