//! Writing a small file onto a stick that can be pulled out mid-write.
//!
//! Two files in `media/` now need this — the scan cache and the watch history —
//! and the part worth getting right is identical for both: never leave a
//! truncated file where a good one was, and never leave litter behind. One copy
//! of the reasoning, so the two cannot drift into disagreeing about it.

use std::path::{Path, PathBuf};

/// Where `write_atomically` writes before renaming over the real file.
///
/// The process id alone is not enough and the reason is this app in
/// particular: it is meant to be carried around, so the ordinary case is two
/// *different machines* with the same stick mounted, handing out process ids
/// that know nothing of each other. Same number, same name, interleaved
/// writes, one truncated JSON. The random tail is what makes the name unique
/// where the pid cannot. The pid stays because it says which copy left a
/// temporary behind when one is found.
pub fn temp_path(path: &Path) -> PathBuf {
    let mut salt = [0u8; 4];
    // A failure here would only cost uniqueness, and a zeroed salt is still a
    // valid name — no reason to fail a write over it.
    let _ = getrandom::fill(&mut salt);
    let salt: String = salt.iter().map(|b| format!("{b:02x}")).collect();
    path.with_extension(format!("{}-{salt}.tmp", std::process::id()))
}

/// Write beside the target and rename over it.
///
/// A stick pulled between the two steps leaves the previous file intact instead
/// of a truncated one, and two copies of the app writing at once each get their
/// own temporary, so neither can overwrite the other's half-written file.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temp = temp_path(path);
    std::fs::write(&temp, bytes)?;
    if let Err(e) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    Ok(())
}

/// Sweep up temporaries `write_atomically` never got to rename.
///
/// Matched narrowly and by both ends — the target's own stem in front, `.tmp`
/// behind, directly in this folder — because everything else there belongs to
/// the user. `.reeldrive-cache.json` fails the suffix, someone else's `.tmp`
/// fails the stem.
///
/// A second copy of the app writing its own temporary at this exact moment
/// loses it and logs that the file was not saved: survivable, self-cured by the
/// next start, and cheaper than teaching this to tell a live write from an
/// abandoned one on a filesystem whose timestamps are two seconds wide.
///
/// `what` names the file in the log, which is the only way a user reading it
/// can tell which of the two was being tidied.
pub fn remove_stale_temporaries(path: &Path, what: &str) {
    let (Some(dir), Some(stem)) = (path.parent(), path.file_stem().and_then(|s| s.to_str())) else {
        return;
    };
    let prefix = format!("{stem}.");

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with(&prefix) && name.ends_with(".tmp") && entry.path().is_file() {
            if let Err(e) = std::fs::remove_file(entry.path()) {
                eprintln!("half-written {what} not removed: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn two_writers_never_pick_the_same_temporary_name() {
        // The pid is unique per machine, and this app travels: the same stick
        // mounted on two computers gets two unrelated pids that collide as a
        // matter of course. Both would write the same temporary, interleaved,
        // and the rename would publish a truncated file.
        let path = Path::new("/stick/media/.reeldrive-cache.json");
        let names: std::collections::HashSet<PathBuf> = (0..64).map(|_| temp_path(path)).collect();
        assert_eq!(names.len(), 64, "two temporaries shared a name");
    }

    #[test]
    fn a_write_leaves_only_the_file_it_was_asked_for() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join(".thing.json");
        write_atomically(&path, b"{}").expect("write");

        let names: Vec<String> = std::fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![".thing.json".to_string()]);
        assert_eq!(std::fs::read(&path).expect("read"), b"{}");
    }

    #[test]
    fn a_second_write_replaces_the_first_whole() {
        // The rename is what makes this true: a plain write of a shorter
        // document over a longer one leaves the tail of the old one behind.
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join(".thing.json");
        write_atomically(&path, b"{\"a\":1,\"b\":2}").expect("write");
        write_atomically(&path, b"{}").expect("write");
        assert_eq!(std::fs::read(&path).expect("read"), b"{}");
    }

    #[test]
    fn a_temporary_left_by_a_pulled_stick_is_swept_up_and_nothing_else_is() {
        // The sweep has to match its own names — a rule it no longer recognises
        // would litter the user's stick for good — and only its own.
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join(".reeldrive-cache.json");
        let orphan = temp_path(&path);
        std::fs::write(&orphan, b"half a file").expect("write");

        let theirs = dir.path().join("holiday.tmp");
        let looks_close = dir.path().join(".reeldrive-cache-notes.json");
        std::fs::write(&theirs, b"not ours").expect("write");
        std::fs::write(&looks_close, b"not ours either").expect("write");

        remove_stale_temporaries(&path, "cache");

        assert!(!orphan.exists(), "the sweep no longer matches its own name");
        assert!(theirs.exists(), "someone else's file was taken");
        assert!(looks_close.exists(), "a near-miss name was taken");
    }

    #[test]
    fn the_sweep_stays_in_its_own_folder() {
        let dir = TempDir::new().expect("tempdir");
        let media = dir.path().join("media");
        let nested = media.join("Movie");
        std::fs::create_dir_all(&nested).expect("mkdir");

        let outside = dir.path().join(".reeldrive-cache.1.tmp");
        let below = nested.join(".reeldrive-cache.2.tmp");
        std::fs::write(&outside, b"x").expect("write");
        std::fs::write(&below, b"x").expect("write");

        remove_stale_temporaries(&media.join(".reeldrive-cache.json"), "cache");

        assert!(outside.exists(), "the sweep climbed out of the folder");
        assert!(below.exists(), "the sweep descended into one");
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_folder_fails_without_panicking() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("tempdir");
        let media = dir.path().join("media");
        std::fs::create_dir(&media).expect("mkdir");
        std::fs::set_permissions(&media, std::fs::Permissions::from_mode(0o555)).expect("chmod");

        let path = media.join(".thing.json");
        let result = write_atomically(&path, b"{}");

        // Root ignores the permission bits; only assert when they are enforced.
        if result.is_ok() {
            eprintln!("skipping: permissions not enforced (running as root?)");
        } else {
            assert!(!path.exists(), "a failed write must not leave a file");
        }

        std::fs::set_permissions(&media, std::fs::Permissions::from_mode(0o755)).expect("restore");
    }
}
