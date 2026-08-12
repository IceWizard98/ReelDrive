//! Domain types. Paths are always relative to the media root and use `/` as
//! separator, so they survive serialization and never leak the host layout.
//!
//! `serde` is derived here on purpose: it describes the data, it does not tie
//! the core to any I/O technology.

use serde::{Deserialize, Serialize};

/// Movie or series. Known from the shallow scan, which already reads each
/// content folder to find its cover — so the home screen can label and group
/// without a single extra disk access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentKind {
    Movie,
    Series,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentSummary {
    /// Folder name inside the media root — unique, and used as the lookup key.
    pub id: String,
    pub title: String,
    pub year: Option<u16>,
    /// Cover image relative to the media root, if the folder has one.
    pub cover: Option<String>,
    pub kind: ContentKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Episode {
    /// Episode number when the filename carries one, otherwise its 1-based
    /// position in natural order.
    pub number: u32,
    pub title: String,
    /// Video file relative to the media root.
    pub file: String,
    /// External subtitle files relative to the media root.
    pub subtitles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Season {
    /// 0 means specials/extras.
    pub number: u32,
    pub title: String,
    pub episodes: Vec<Episode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ContentBody {
    Movie {
        file: String,
        subtitles: Vec<String>,
    },
    Series {
        seasons: Vec<Season>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentDetail {
    pub summary: ContentSummary,
    pub body: ContentBody,
}

impl ContentSummary {
    /// True when this summary could have been produced by scanning `id`.
    ///
    /// Summaries can arrive from the cache file on the stick, which anyone can
    /// edit. The cover is the field that gets used as a path, so it must stay
    /// inside the folder it claims to describe.
    pub fn is_consistent_with(&self, id: &str) -> bool {
        if self.id != id {
            return false;
        }
        match &self.cover {
            None => true,
            Some(cover) => {
                let prefix = format!("{id}/");
                cover.starts_with(&prefix)
                    && !cover.contains("..")
                    && !cover.contains('\\')
                    && !cover.contains('\0')
                    && cover.len() > prefix.len()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(id: &str, cover: Option<&str>) -> ContentSummary {
        ContentSummary {
            id: id.to_string(),
            title: id.to_string(),
            year: None,
            cover: cover.map(str::to_string),
            kind: ContentKind::Movie,
        }
    }

    #[test]
    fn a_summary_scanned_from_its_own_folder_is_consistent() {
        assert!(summary("Movie", Some("Movie/cover.jpg")).is_consistent_with("Movie"));
        assert!(summary("Movie", None).is_consistent_with("Movie"));
    }

    #[test]
    fn a_summary_filed_under_another_id_is_rejected() {
        assert!(!summary("Movie", Some("Movie/cover.jpg")).is_consistent_with("Other"));
    }

    #[test]
    fn a_cover_reaching_outside_its_folder_is_rejected() {
        for cover in [
            "../secrets/id_rsa.jpg",
            "Movie/../../etc/passwd",
            "Other/cover.jpg",
            "/etc/passwd",
            "Movie\\..\\..\\windows",
            "Movie/",
        ] {
            assert!(
                !summary("Movie", Some(cover)).is_consistent_with("Movie"),
                "cover: {cover}"
            );
        }
    }
}
