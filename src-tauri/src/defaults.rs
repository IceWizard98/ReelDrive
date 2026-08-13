//! Every convention the app relies on lives here. There is no config file on
//! purpose: the user drops the executable and a `media` folder on the stick and
//! it works. If configurability is ever needed, read an optional file here and
//! override these constants — nothing else has to change.

/// Folder that holds the content, expected next to the executable.
pub const MEDIA_DIR_NAME: &str = "media";

/// Hidden index cache, written inside the media folder.
pub const CACHE_FILE_NAME: &str = ".reeldrive-cache.json";

/// Hidden watch history, written inside the media folder beside the cache so it
/// travels with the stick: where you got to is only useful on the machine you
/// carry it to next.
///
/// A file of its own rather than a field in the cache, because the two have
/// opposite lifetimes — the cache is discarded on every release, and this is
/// the one thing here the user cannot reproduce.
pub const PROGRESS_FILE_NAME: &str = ".reeldrive-progress.json";

/// Where a history this build cannot read is put, instead of being deleted.
pub const PROGRESS_SET_ASIDE_FILE_NAME: &str = ".reeldrive-progress.unreadable.json";

/// Cache files written by earlier releases under a different name. Nothing will
/// ever read these again, so they are removed rather than left on the stick.
/// Add to this list whenever `CACHE_FILE_NAME` changes.
pub const LEGACY_CACHE_FILE_NAMES: &[&str] = &[".pnetflix-cache.json"];

/// Lowercase extensions treated as playable video.
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "avi", "m4v", "mov", "webm", "ts", "wmv", "mpg", "mpeg", "flv", "ogv",
];

/// Lowercase extensions treated as external subtitles.
pub const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "sub", "vtt"];

/// Lowercase extensions treated as images (cover art).
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "gif", "avif"];

/// Preferred cover basenames, in priority order. Any other image in the content
/// root is used as a fallback; a missing cover is not an error.
pub const COVER_BASENAMES: &[&str] = &["cover", "poster", "folder", "fanart", "banner", "thumb"];

/// Directory names created by operating systems on removable media.
pub const SYSTEM_DIR_NAMES: &[&str] = &[
    "system volume information",
    "$recycle.bin",
    "recycler",
    ".trashes",
    ".spotlight-v100",
    ".fseventsd",
    ".temporaryitems",
    "found.000",
    "lost+dir",
    "lost+found",
];
