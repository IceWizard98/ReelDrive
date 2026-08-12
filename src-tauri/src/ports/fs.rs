//! Filesystem port. The core scans through this trait only, so the whole
//! classification logic is testable against an in-memory tree.

use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

impl DirEntry {
    pub fn dir(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_dir: true,
        }
    }

    pub fn file(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_dir: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    NotFound(PathBuf),
    PermissionDenied(PathBuf),
    Other { path: PathBuf, message: String },
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FsError::NotFound(p) => write!(f, "not found: {}", p.display()),
            FsError::PermissionDenied(p) => write!(f, "permission denied: {}", p.display()),
            FsError::Other { path, message } => write!(f, "{}: {}", path.display(), message),
        }
    }
}

impl std::error::Error for FsError {}

pub trait FileSystem {
    /// Entries directly inside `path`, in unspecified order.
    fn read_dir(&self, path: &Path) -> Result<Vec<DirEntry>, FsError>;

    /// Modification time of `path` in seconds since the Unix epoch.
    fn modified_secs(&self, path: &Path) -> Result<u64, FsError>;

    fn is_dir(&self, path: &Path) -> bool;
}
