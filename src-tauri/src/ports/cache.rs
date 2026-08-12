//! Library cache port. The cache is an optimisation and never a source of
//! truth: every operation is allowed to fail, and failing means "scan again".

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::core::model::ContentSummary;

/// Bumped by hand whenever the cached shape changes.
pub const CACHE_VERSION: u32 = 2;

/// Which build wrote the cache. Set by the release workflow, which passes an
/// number that grows with every run; a build from a developer's machine reports
/// 0.
///
/// It exists so that a change to what is cached cannot outlive the release that
/// made it: remembering to bump `CACHE_VERSION` is exactly the kind of step
/// that gets forgotten, and a cache read back into a shape that no longer means
/// the same thing is a bug nobody can see. A release therefore starts from a
/// fresh scan.
///
/// That costs one walk of the stick only while a single build uses it. Two —
/// an old copy kept beside a new one, or a release alongside a build from a
/// developer's machine, which reports 0 — each throw the other's cache away
/// and rescan on every start, for as long as both are in use. The cost is a
/// walk of the folder tree at startup, and correctness is the thing being
/// bought; but it is not "once per release" in that case, and the comment
/// used to say it was.
pub const BUILD_NUMBER: u32 = match option_env!("REELDRIVE_BUILD") {
    Some(text) => parse_build(text),
    None => 0,
};

/// `str::parse` is not available in a constant, and this runs at compile time.
/// Anything that is not a plain number reads as 0, the same as a build that was
/// never given one.
const fn parse_build(text: &str) -> u32 {
    let bytes = text.as_bytes();
    // Nine digits is more than any run counter will reach, and it keeps the
    // multiplication below from overflowing into a compile error.
    if bytes.is_empty() || bytes.len() > 9 {
        return 0;
    }
    let mut value = 0u32;
    let mut i = 0;
    while i < bytes.len() {
        let digit = bytes[i];
        if digit < b'0' || digit > b'9' {
            return 0;
        }
        value = value * 10 + (digit - b'0') as u32;
        i += 1;
    }
    value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Modification time of the content folder when it was scanned.
    ///
    /// Only as precise as the filesystem allows: FAT32 stores it in two-second
    /// steps, so a folder changed within the same step as the last scan keeps
    /// serving the cached entry until it changes again. Deleting the cache file
    /// forces a full rescan.
    pub mtime: u64,
    /// `None` records a folder that held nothing playable, so it is not
    /// re-examined on every start.
    pub summary: Option<ContentSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryCacheData {
    pub version: u32,
    /// The build that wrote this file. A cache from any other build is thrown
    /// away, on the assumption that what is cached may have changed meaning.
    #[serde(default)]
    pub build: u32,
    pub entries: BTreeMap<String, CacheEntry>,
}

impl Default for LibraryCacheData {
    fn default() -> Self {
        Self {
            version: CACHE_VERSION,
            build: BUILD_NUMBER,
            entries: BTreeMap::new(),
        }
    }
}

impl LibraryCacheData {
    /// The cache file lives on the stick, so it is untrusted input: a hand-edited
    /// one must not be able to name a cover outside the media folder, or claim an
    /// id that does not match the folder it was found under.
    pub fn get(&self, id: &str, mtime: Option<u64>) -> Option<&CacheEntry> {
        let mtime = mtime?;
        let entry = self.entries.get(id).filter(|entry| entry.mtime == mtime)?;
        match &entry.summary {
            Some(summary) if !summary.is_consistent_with(id) => None,
            _ => Some(entry),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_number_is_read_from_its_digits() {
        assert_eq!(parse_build("0"), 0);
        assert_eq!(parse_build("1"), 1);
        assert_eq!(parse_build("4269"), 4269);
        // A run counter is written by the workflow, not by a person: leading
        // zeros are the shape it can plausibly arrive in.
        assert_eq!(parse_build("007"), 7);
    }

    #[test]
    fn the_longest_number_it_accepts_cannot_overflow() {
        // Nine digits is the guard that keeps `value * 10` inside u32, whose
        // ceiling is 4_294_967_295. This is the pair that shows where the line
        // is: the largest number it will read, and the first one it refuses.
        // Without the guard the first of these overflows, which in a `const fn`
        // is a compile error rather than a wrap — loud either way, but only if
        // something asks for it.
        assert_eq!(parse_build("999999999"), 999_999_999);
        assert_eq!(parse_build("1000000000"), 0, "ten digits is refused whole");
    }

    #[test]
    fn anything_that_is_not_a_plain_number_reads_as_no_build() {
        // Zero is what an unset variable gives, so every unusable value has to
        // land there too: the alternative is a number nobody chose, silently
        // deciding whether the cache on a stick is thrown away.
        for text in [
            "", "abc", "4a2", "-1", "+1", " 42", "42 ", "4.2", "1_000", "42\n",
            // Not ASCII: the bytes are read one at a time, and a multibyte
            // character is several non-digits rather than one.
            "42€", "٤٢",
        ] {
            assert_eq!(parse_build(text), 0, "text: {text:?}");
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheError(pub String);

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for CacheError {}

pub trait LibraryCache {
    /// Missing, unreadable, corrupt or outdated caches all read as `None`, and
    /// anything unreadable is removed: leaving it behind litters the user's
    /// stick with a hidden file nothing will ever open again.
    fn load(&self) -> Option<LibraryCacheData>;

    /// Failure here is survivable: a read-only stick simply gets no cache.
    fn store(&self, data: &LibraryCacheData) -> Result<(), CacheError>;
}
