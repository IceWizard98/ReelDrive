//! Watch-history port.
//!
//! The opposite of the cache port in the one way that matters. A cache is an
//! optimisation, so every failure there means "scan again" and anything
//! unreadable is deleted. This is the only thing on the stick the user cannot
//! reproduce by pressing a button: it is never deleted, never thrown away for
//! being written by another build, and a file this build cannot read is set
//! aside rather than overwritten.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressError(pub String);

impl fmt::Display for ProgressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ProgressError {}

pub trait ProgressStore {
    /// Never fails: a missing, unreadable or future-version history reads as an
    /// empty one, because there is nothing else the app could usefully do with
    /// it and refusing to start would be worse than forgetting.
    fn load(&self) -> crate::core::progress::ProgressData;

    /// Failure is survivable — a read-only stick simply remembers nothing —
    /// but it is reported, unlike the cache: silently forgetting where someone
    /// got to is the whole feature not working.
    fn save(&self, data: &crate::core::progress::ProgressData) -> Result<(), ProgressError>;
}
