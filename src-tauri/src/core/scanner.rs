//! Library classification. Everything here goes through the `FileSystem` port,
//! so the rules are exercised against in-memory trees in the tests below.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use crate::core::model::{
    ContentBody, ContentDetail, ContentKind, ContentSummary, Episode, Season,
};
use crate::core::naming;
use crate::defaults;
use crate::ports::cache::{CacheEntry, LibraryCacheData};
use crate::ports::fs::{DirEntry, FileSystem, FsError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanError {
    MediaRootMissing,
    ContentNotFound(String),
    NoPlayableFile(String),
    Fs(FsError),
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScanError::MediaRootMissing => write!(f, "media folder not found"),
            ScanError::ContentNotFound(id) => write!(f, "content not found: {id}"),
            ScanError::NoPlayableFile(id) => write!(f, "no playable file in: {id}"),
            ScanError::Fs(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ScanError {}

impl From<FsError> for ScanError {
    fn from(e: FsError) -> Self {
        ScanError::Fs(e)
    }
}

/// Result of the shallow pass: what the home screen needs, plus anything that
/// was skipped so the user can be told instead of silently losing content.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryScan {
    pub contents: Vec<ContentSummary>,
    pub warnings: Vec<String>,
}

/// Shallow pass: one level into each content folder, enough for title and cover.
pub fn scan_library(fs: &dyn FileSystem, media_root: &Path) -> Result<LibraryScan, ScanError> {
    scan_library_cached(fs, media_root, &LibraryCacheData::default()).map(|(scan, _)| scan)
}

/// Same pass, but folders whose modification time matches the cache are taken
/// from it without touching the disk. Returns the refreshed cache to store.
pub fn scan_library_cached(
    fs: &dyn FileSystem,
    media_root: &Path,
    cache: &LibraryCacheData,
) -> Result<(LibraryScan, LibraryCacheData), ScanError> {
    if !fs.is_dir(media_root) {
        return Err(ScanError::MediaRootMissing);
    }

    let mut scan = LibraryScan::default();
    let mut fresh = LibraryCacheData::default();
    let mut entries = fs.read_dir(media_root)?;
    entries.sort_by(|a, b| naming::natural_cmp(&a.name, &b.name));

    for entry in entries {
        if !entry.is_dir || is_ignored_dir(&entry.name) {
            continue;
        }
        let content_dir = media_root.join(&entry.name);
        let mtime = fs.modified_secs(&content_dir).ok();

        // Unchanged folder: reuse what we already know and skip the read_dir.
        if let Some(cached) = cache.get(&entry.name, mtime) {
            match &cached.summary {
                Some(summary) => scan.contents.push(summary.clone()),
                None => scan
                    .warnings
                    .push(format!("\u{201c}{}\u{201d} contains no video", entry.name)),
            }
            fresh.entries.insert(entry.name.clone(), cached.clone());
            continue;
        }

        let children = match fs.read_dir(&content_dir) {
            Ok(children) => children,
            Err(e) => {
                scan.warnings
                    .push(format!("\u{201c}{}\u{201d} is unreadable: {e}", entry.name));
                continue;
            }
        };

        // The home must never offer something the detail view will refuse, so
        // "does this folder hold anything playable" is answered here rather than
        // guessed. With no video alongside the cover, that means one extra
        // read_dir per subfolder — paid once, then served from the cache.
        let has_video = children.iter().any(|c| !c.is_dir && is_video(&c.name));
        let playable = has_video || holds_video_below(fs, &content_dir, &children);

        if !playable {
            scan.warnings
                .push(format!("\u{201c}{}\u{201d} contains no video", entry.name));
            remember(&mut fresh, &entry.name, mtime, None);
            continue;
        }

        let summary = summarize(&entry.name, &children);
        remember(&mut fresh, &entry.name, mtime, Some(summary.clone()));
        scan.contents.push(summary);
    }

    scan.contents
        .sort_by(|a, b| naming::natural_cmp(&a.title, &b.title));
    Ok((scan, fresh))
}

/// Only folders with a readable modification time are cacheable: without one
/// there is nothing to invalidate against.
fn remember(
    cache: &mut LibraryCacheData,
    id: &str,
    mtime: Option<u64>,
    summary: Option<ContentSummary>,
) {
    if let Some(mtime) = mtime {
        cache
            .entries
            .insert(id.to_string(), CacheEntry { mtime, summary });
    }
}

/// Deep pass for a single content folder: movie or series with seasons.
pub fn scan_content(
    fs: &dyn FileSystem,
    media_root: &Path,
    id: &str,
) -> Result<ContentDetail, ScanError> {
    if !is_safe_id(id) {
        return Err(ScanError::ContentNotFound(id.to_string()));
    }
    let content_dir = media_root.join(id);
    if !fs.is_dir(&content_dir) {
        return Err(ScanError::ContentNotFound(id.to_string()));
    }

    let entries = fs
        .read_dir(&content_dir)
        .map_err(|_| ScanError::ContentNotFound(id.to_string()))?;
    let summary = summarize(id, &entries);

    let mut season_dirs: Vec<(u32, String)> = entries
        .iter()
        .filter(|e| e.is_dir && !is_ignored_dir(&e.name))
        .filter_map(|e| naming::parse_season_dir(&e.name).map(|n| (n, e.name.clone())))
        .collect();
    season_dirs.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| naming::natural_cmp(&a.1, &b.1)));

    let root_videos: Vec<String> = entries
        .iter()
        .filter(|e| !e.is_dir && is_video(&e.name))
        .map(|e| e.name.clone())
        .collect();

    // Folders named in a way `parse_season_dir` does not recognise ("Prima
    // Stagione", "Parte 1") still hold real episodes. They become seasons in
    // natural order, but only when nothing better was found — a recognised
    // season name always wins.
    if season_dirs.is_empty() && root_videos.is_empty() {
        season_dirs = other_dirs(&entries)
            .into_iter()
            .enumerate()
            .map(|(i, name)| ((i + 1) as u32, name))
            .collect();
    }

    // A lone video with no episode marker is a movie; everything else is a series.
    if season_dirs.is_empty() {
        if root_videos.is_empty() {
            return Err(ScanError::NoPlayableFile(id.to_string()));
        }
        if root_videos.len() == 1 && naming::parse_episode(stem_of(&root_videos[0])).is_none() {
            let file = &root_videos[0];
            return Ok(ContentDetail {
                summary,
                body: ContentBody::Movie {
                    file: join_rel(id, file),
                    subtitles: subtitles_for(id, stem_of(file), &entries),
                },
            });
        }
    }

    let mut seasons: Vec<Season> = Vec::new();

    // Videos sitting next to the season folders still belong somewhere: their
    // own marker decides, falling back to season 1.
    let mut by_season: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for name in &root_videos {
        let number = naming::parse_episode(stem_of(name))
            .and_then(|r| r.season)
            .unwrap_or(1);
        by_season.entry(number).or_default().push(name.clone());
    }
    for (number, names) in by_season {
        seasons.push(Season {
            number,
            title: String::new(),
            episodes: build_episodes(id, &names, &entries),
        });
    }

    for (number, dir_name) in &season_dirs {
        let season_dir = content_dir.join(dir_name);
        let children = match fs.read_dir(&season_dir) {
            Ok(children) => children,
            Err(_) => continue,
        };
        let rel_dir = join_rel(id, dir_name);
        let names: Vec<String> = children
            .iter()
            .filter(|e| !e.is_dir && is_video(&e.name))
            .map(|e| e.name.clone())
            .collect();
        if names.is_empty() {
            continue;
        }
        let episodes = build_episodes(&rel_dir, &names, &children);
        match seasons.iter_mut().find(|s| s.number == *number) {
            Some(existing) => existing.episodes.extend(episodes),
            // Empty unless the folder name carries something the number does
            // not already say: the window falls back to "Season N", which is
            // the better label for a folder called `S02`.
            None => seasons.push(Season {
                number: *number,
                title: naming::season_dir_title(dir_name).unwrap_or_default(),
                episodes,
            }),
        }
    }

    seasons.retain(|s| !s.episodes.is_empty());
    if seasons.is_empty() {
        return Err(ScanError::NoPlayableFile(id.to_string()));
    }
    seasons.sort_by_key(|s| s.number);

    Ok(ContentDetail {
        summary,
        body: ContentBody::Series { seasons },
    })
}

fn summarize(id: &str, entries: &[DirEntry]) -> ContentSummary {
    let (title, year) = naming::clean_title(id);
    ContentSummary {
        id: id.to_string(),
        title,
        year,
        cover: pick_cover(id, entries),
        kind: classify(entries),
    }
}

/// Movie or series, decided from one directory listing. The deep scan reaches
/// the same verdict by the same rules, so a folder never changes type when it
/// is opened.
fn classify(entries: &[DirEntry]) -> ContentKind {
    let has_season_dir = entries
        .iter()
        .filter(|e| e.is_dir && !is_ignored_dir(&e.name))
        .any(|e| naming::parse_season_dir(&e.name).is_some());
    if has_season_dir {
        return ContentKind::Series;
    }

    let videos: Vec<&str> = entries
        .iter()
        .filter(|e| !e.is_dir && is_video(&e.name))
        .map(|e| e.name.as_str())
        .collect();

    match videos.as_slice() {
        [only] if naming::parse_episode(stem_of(only)).is_none() => ContentKind::Movie,
        [_] => ContentKind::Series,
        // No video here: the episodes live in subfolders, which the deep scan
        // turns into seasons. Both passes must agree.
        [] if !other_dirs(entries).is_empty() => ContentKind::Series,
        [] => ContentKind::Movie,
        _ => ContentKind::Series,
    }
}

/// True when any subfolder directly holds a video. Unreadable subfolders count
/// as empty: a folder nobody can open is not playable content either.
fn holds_video_below(fs: &dyn FileSystem, content_dir: &Path, children: &[DirEntry]) -> bool {
    children
        .iter()
        .filter(|c| c.is_dir && !is_ignored_dir(&c.name))
        .any(|c| match fs.read_dir(&content_dir.join(&c.name)) {
            Ok(grandchildren) => grandchildren.iter().any(|g| !g.is_dir && is_video(&g.name)),
            Err(_) => false,
        })
}

/// Subfolders that are not recognised as seasons, in natural order.
fn other_dirs(entries: &[DirEntry]) -> Vec<String> {
    let mut names: Vec<String> = entries
        .iter()
        .filter(|e| e.is_dir && !is_ignored_dir(&e.name))
        .filter(|e| naming::parse_season_dir(&e.name).is_none())
        .map(|e| e.name.clone())
        .collect();
    names.sort_by(|a, b| naming::natural_cmp(a, b));
    names
}

/// Preferred cover names first; any other image is better than none.
fn pick_cover(id: &str, entries: &[DirEntry]) -> Option<String> {
    let mut images: Vec<&str> = entries
        .iter()
        .filter(|e| !e.is_dir && is_image(&e.name))
        .map(|e| e.name.as_str())
        .collect();
    images.sort_by(|a, b| naming::natural_cmp(a, b));

    for preferred in defaults::COVER_BASENAMES {
        if let Some(found) = images
            .iter()
            .find(|name| stem_of(name).eq_ignore_ascii_case(preferred))
        {
            return Some(join_rel(id, found));
        }
    }
    images.first().map(|name| join_rel(id, name))
}

/// Numbers come from the filenames when every file in the season has a marker;
/// otherwise natural order decides, so numbering can never collide.
fn build_episodes(rel_dir: &str, names: &[String], siblings: &[DirEntry]) -> Vec<Episode> {
    let mut sorted: Vec<&String> = names.iter().collect();
    sorted.sort_by(|a, b| naming::natural_cmp(a, b));

    let markers: Vec<Option<u32>> = sorted
        .iter()
        .map(|name| naming::parse_episode(stem_of(name)).map(|r| r.episode))
        .collect();
    let use_markers = markers.iter().all(Option::is_some);

    let mut episodes: Vec<Episode> = sorted
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let stem = stem_of(name);
            Episode {
                number: if use_markers {
                    markers[i].unwrap_or(0)
                } else {
                    (i + 1) as u32
                },
                title: naming::episode_title(stem),
                file: join_rel(rel_dir, name),
                subtitles: subtitles_for(rel_dir, stem, siblings),
            }
        })
        .collect();

    episodes.sort_by(|a, b| {
        a.number
            .cmp(&b.number)
            .then_with(|| naming::natural_cmp(&a.file, &b.file))
    });
    episodes
}

/// `film.srt` and `film.eng.srt` belong to `film.mkv`; `altro.srt` does not.
fn subtitles_for(rel_dir: &str, video_stem: &str, siblings: &[DirEntry]) -> Vec<String> {
    let target = video_stem.to_lowercase();
    let mut found: Vec<&str> = siblings
        .iter()
        .filter(|e| !e.is_dir && is_subtitle(&e.name))
        .filter(|e| {
            let stem = stem_of(&e.name).to_lowercase();
            stem == target || stem.starts_with(&format!("{target}."))
        })
        .map(|e| e.name.as_str())
        .collect();
    found.sort_by(|a, b| naming::natural_cmp(a, b));
    found
        .into_iter()
        .map(|name| join_rel(rel_dir, name))
        .collect()
}

/// Ids are single folder names: anything else could walk out of the media root.
fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && !id.contains('/')
        && !id.contains('\\')
        && !id.contains('\0')
}

fn is_ignored_dir(name: &str) -> bool {
    name.starts_with('.') || defaults::SYSTEM_DIR_NAMES.contains(&name.to_lowercase().as_str())
}

fn extension_of(name: &str) -> Option<String> {
    name.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, ext)| ext.to_lowercase())
}

fn stem_of(name: &str) -> &str {
    name.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map_or(name, |(stem, _)| stem)
}

fn has_extension_in(name: &str, list: &[&str]) -> bool {
    extension_of(name).is_some_and(|ext| list.contains(&ext.as_str()))
}

fn is_video(name: &str) -> bool {
    has_extension_in(name, defaults::VIDEO_EXTENSIONS)
}

fn is_subtitle(name: &str) -> bool {
    has_extension_in(name, defaults::SUBTITLE_EXTENSIONS)
}

fn is_image(name: &str) -> bool {
    has_extension_in(name, defaults::IMAGE_EXTENSIONS)
}

/// Relative paths always use `/`, whatever the host separator is.
fn join_rel(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

#[cfg(test)]
pub(crate) mod fake_fs {
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};

    use crate::ports::fs::{DirEntry, FileSystem, FsError};

    /// In-memory tree built from `"a/b/c.mkv"` style paths.
    #[derive(Default)]
    pub struct FakeFs {
        dirs: HashMap<PathBuf, Vec<DirEntry>>,
        known_dirs: HashSet<PathBuf>,
        unreadable: HashSet<PathBuf>,
        mtimes: HashMap<PathBuf, u64>,
    }

    impl FakeFs {
        pub fn new() -> Self {
            Self::default()
        }

        /// `root` is the media root; each path is relative to it. A trailing
        /// `/` marks an explicitly empty directory.
        pub fn with_tree(root: &str, paths: &[&str]) -> Self {
            let mut fs = Self::new();
            fs.ensure_dir(Path::new(root));
            for path in paths {
                let is_dir = path.ends_with('/');
                let trimmed = path.trim_end_matches('/');
                let mut current = PathBuf::from(root);
                let parts: Vec<&str> = trimmed.split('/').collect();
                for (i, part) in parts.iter().enumerate() {
                    let last = i == parts.len() - 1;
                    let entry_is_dir = !last || is_dir;
                    fs.add_entry(&current, part, entry_is_dir);
                    current = current.join(part);
                    if entry_is_dir {
                        fs.ensure_dir(&current);
                    }
                }
            }
            fs
        }

        pub fn make_unreadable(&mut self, path: &Path) {
            self.unreadable.insert(path.to_path_buf());
        }

        pub fn set_mtime(&mut self, path: &Path, secs: u64) {
            self.mtimes.insert(path.to_path_buf(), secs);
        }

        fn ensure_dir(&mut self, path: &Path) {
            self.known_dirs.insert(path.to_path_buf());
            self.dirs.entry(path.to_path_buf()).or_default();
        }

        fn add_entry(&mut self, parent: &Path, name: &str, is_dir: bool) {
            let entries = self.dirs.entry(parent.to_path_buf()).or_default();
            if !entries.iter().any(|e| e.name == name) {
                entries.push(DirEntry {
                    name: name.to_string(),
                    is_dir,
                });
            }
        }
    }

    impl FileSystem for FakeFs {
        fn read_dir(&self, path: &Path) -> Result<Vec<DirEntry>, FsError> {
            if self.unreadable.contains(path) {
                return Err(FsError::PermissionDenied(path.to_path_buf()));
            }
            self.dirs
                .get(path)
                .cloned()
                .ok_or_else(|| FsError::NotFound(path.to_path_buf()))
        }

        fn modified_secs(&self, path: &Path) -> Result<u64, FsError> {
            self.mtimes
                .get(path)
                .copied()
                .ok_or_else(|| FsError::NotFound(path.to_path_buf()))
        }

        fn is_dir(&self, path: &Path) -> bool {
            self.known_dirs.contains(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake_fs::FakeFs;
    use super::*;
    use crate::core::model::{ContentBody, Season};
    use std::path::PathBuf;

    const ROOT: &str = "/stick/media";

    fn root() -> PathBuf {
        PathBuf::from(ROOT)
    }

    fn scan(paths: &[&str]) -> LibraryScan {
        let fs = FakeFs::with_tree(ROOT, paths);
        scan_library(&fs, &root()).expect("library scan")
    }

    fn detail(paths: &[&str], id: &str) -> ContentDetail {
        let fs = FakeFs::with_tree(ROOT, paths);
        scan_content(&fs, &root(), id).expect("content scan")
    }

    fn seasons(detail: &ContentDetail) -> Vec<Season> {
        match &detail.body {
            ContentBody::Series { seasons } => seasons.clone(),
            other => panic!("expected a series, got {other:?}"),
        }
    }

    #[test]
    fn missing_media_root_is_reported() {
        let fs = FakeFs::new();
        assert_eq!(
            scan_library(&fs, &PathBuf::from("/nowhere")),
            Err(ScanError::MediaRootMissing)
        );
    }

    #[test]
    fn empty_media_root_yields_no_content() {
        let scan = scan(&[]);
        assert!(scan.contents.is_empty());
        assert!(scan.warnings.is_empty());
    }

    #[test]
    fn empty_content_folder_is_skipped_with_a_warning() {
        let scan = scan(&["Empty/"]);
        assert!(scan.contents.is_empty());
        assert_eq!(scan.warnings.len(), 1);
        assert!(scan.warnings[0].contains("Empty"));
    }

    #[test]
    fn folder_with_only_a_cover_is_skipped() {
        let scan = scan(&["Empty/cover.jpg"]);
        assert!(scan.contents.is_empty());
        assert_eq!(scan.warnings.len(), 1);
    }

    #[test]
    fn loose_files_in_the_media_root_are_ignored() {
        let scan = scan(&["readme.txt", "Inception/Inception.mkv"]);
        assert_eq!(scan.contents.len(), 1);
        assert_eq!(scan.contents[0].id, "Inception");
    }

    #[test]
    fn summary_carries_title_year_and_cover() {
        let scan = scan(&[
            "Inception (2010)/cover.jpg",
            "Inception (2010)/Inception.mkv",
        ]);
        let content = &scan.contents[0];
        assert_eq!(content.id, "Inception (2010)");
        assert_eq!(content.title, "Inception");
        assert_eq!(content.year, Some(2010));
        assert_eq!(content.cover.as_deref(), Some("Inception (2010)/cover.jpg"));
    }

    #[test]
    fn the_shallow_scan_tells_movies_from_series() {
        let cases: [(&[&str], &str, ContentKind); 5] = [
            (&["Movie/Movie.mkv"], "Movie", ContentKind::Movie),
            (
                &["Movie/cover.jpg", "Movie/Movie.mkv"],
                "Movie",
                ContentKind::Movie,
            ),
            (
                &["Show/Season 1/a.mkv", "Show/Season 2/b.mkv"],
                "Show",
                ContentKind::Series,
            ),
            (
                &["Show/Show S01E01.mkv", "Show/Show S01E02.mkv"],
                "Show",
                ContentKind::Series,
            ),
            (&["Show/Show S01E01.mkv"], "Show", ContentKind::Series),
        ];

        for (paths, id, expected) in cases {
            let scan = scan(paths);
            assert_eq!(scan.contents.len(), 1, "paths: {paths:?}");
            assert_eq!(scan.contents[0].id, id);
            assert_eq!(scan.contents[0].kind, expected, "paths: {paths:?}");
        }
    }

    #[test]
    fn the_shallow_kind_agrees_with_the_deep_scan() {
        let trees: [&[&str]; 6] = [
            &["X/X.mkv"],
            &["X/Season 1/a S01E01.mkv"],
            &["X/a S01E01.mkv", "X/a S01E02.mkv"],
            &["X/a S01E01.mkv"],
            &["X/Specials/x.mkv", "X/Season 1/a S01E01.mkv"],
            &["X/Prima Stagione/ep01.mkv"],
        ];

        for paths in trees {
            let fs = FakeFs::with_tree(ROOT, paths);
            let shallow = scan_library(&fs, &root()).expect("library").contents[0].kind;
            let deep = scan_content(&fs, &root(), "X").expect("content");
            let deep_kind = match deep.body {
                ContentBody::Movie { .. } => ContentKind::Movie,
                ContentBody::Series { .. } => ContentKind::Series,
            };
            assert_eq!(shallow, deep_kind, "paths: {paths:?}");
            assert_eq!(deep.summary.kind, deep_kind, "detail summary disagrees");
        }
    }

    #[test]
    fn cover_is_optional() {
        let scan = scan(&["Inception/Inception.mkv"]);
        assert_eq!(scan.contents[0].cover, None);
    }

    #[test]
    fn preferred_cover_names_win_over_other_images() {
        let scan = scan(&["Movie/aaa.png", "Movie/poster.jpg", "Movie/Movie.mkv"]);
        assert_eq!(scan.contents[0].cover.as_deref(), Some("Movie/poster.jpg"));
    }

    #[test]
    fn any_image_is_used_when_no_preferred_cover_exists() {
        let scan = scan(&["Movie/zzz.png", "Movie/aaa.jpg", "Movie/Movie.mkv"]);
        assert_eq!(scan.contents[0].cover.as_deref(), Some("Movie/aaa.jpg"));
    }

    #[test]
    fn cover_lookup_is_case_insensitive() {
        let scan = scan(&["Movie/COVER.JPG", "Movie/Movie.mkv"]);
        assert_eq!(scan.contents[0].cover.as_deref(), Some("Movie/COVER.JPG"));
    }

    #[test]
    fn system_and_hidden_folders_are_skipped() {
        let scan = scan(&[
            "System Volume Information/x.bin",
            "$RECYCLE.BIN/y.bin",
            ".Trashes/z.bin",
            ".hidden/a.mkv",
            "Scrubs/S01E01.mkv",
        ]);
        assert_eq!(scan.contents.len(), 1);
        assert_eq!(scan.contents[0].id, "Scrubs");
        assert!(
            scan.warnings.is_empty(),
            "system folders must not produce warnings"
        );
    }

    #[test]
    fn unreadable_content_folder_is_skipped_with_a_warning() {
        let mut fs = FakeFs::with_tree(ROOT, &["Broken/a.mkv", "Ok/b.mkv"]);
        fs.make_unreadable(&root().join("Broken"));
        let scan = scan_library(&fs, &root()).expect("library scan");
        assert_eq!(scan.contents.len(), 1);
        assert_eq!(scan.contents[0].id, "Ok");
        assert_eq!(scan.warnings.len(), 1);
        assert!(scan.warnings[0].contains("Broken"));
    }

    #[test]
    fn library_is_sorted_naturally_by_title() {
        let scan = scan(&["Volume 10/a.mkv", "Volume 2/a.mkv", "alpha/a.mkv"]);
        let titles: Vec<&str> = scan.contents.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, vec!["alpha", "Volume 2", "Volume 10"]);
    }

    #[test]
    fn unicode_names_survive() {
        let scan = scan(&["Amélie (2001)/Amélie.mkv"]);
        assert_eq!(scan.contents[0].title, "Amélie");
    }

    #[test]
    fn single_video_is_a_movie() {
        let detail = detail(
            &["Inception/cover.jpg", "Inception/Inception.mkv"],
            "Inception",
        );
        match detail.body {
            ContentBody::Movie { file, subtitles } => {
                assert_eq!(file, "Inception/Inception.mkv");
                assert!(subtitles.is_empty());
            }
            other => panic!("expected a movie, got {other:?}"),
        }
    }

    #[test]
    fn movie_picks_up_external_subtitles() {
        let detail = detail(
            &[
                "Movie/Movie.mkv",
                "Movie/Movie.srt",
                "Movie/Movie.eng.srt",
                "Movie/Other.srt",
            ],
            "Movie",
        );
        match detail.body {
            ContentBody::Movie { subtitles, .. } => {
                assert_eq!(subtitles, vec!["Movie/Movie.eng.srt", "Movie/Movie.srt"]);
            }
            other => panic!("expected a movie, got {other:?}"),
        }
    }

    #[test]
    fn single_video_with_an_episode_marker_is_a_one_episode_series() {
        let detail = detail(&["Series/Series S01E01.mkv"], "Series");
        let seasons = seasons(&detail);
        assert_eq!(seasons.len(), 1);
        assert_eq!(seasons[0].number, 1);
        assert_eq!(seasons[0].episodes.len(), 1);
    }

    #[test]
    fn several_videos_in_the_root_form_a_single_season() {
        let detail = detail(
            &[
                "HIMYM/poster.png",
                "HIMYM/HIMYM S01E01.mkv",
                "HIMYM/HIMYM S01E02.mkv",
            ],
            "HIMYM",
        );
        let seasons = seasons(&detail);
        assert_eq!(seasons.len(), 1);
        assert_eq!(seasons[0].number, 1);
        assert_eq!(seasons[0].episodes.len(), 2);
        assert_eq!(seasons[0].episodes[0].file, "HIMYM/HIMYM S01E01.mkv");
    }

    #[test]
    fn season_folders_are_recognised_in_mixed_spellings() {
        let detail = detail(
            &[
                "Series/Season 1/a S01E01.mkv",
                "Series/S02/b S02E01.mkv",
                "Series/Stagione 3/c S03E01.mkv",
            ],
            "Series",
        );
        let numbers: Vec<u32> = seasons(&detail).iter().map(|s| s.number).collect();
        assert_eq!(numbers, vec![1, 2, 3]);
    }

    #[test]
    fn specials_folder_becomes_season_zero_and_sorts_first() {
        let detail = detail(
            &["Series/Specials/x.mkv", "Series/Season 1/a S01E01.mkv"],
            "Series",
        );
        let numbers: Vec<u32> = seasons(&detail).iter().map(|s| s.number).collect();
        assert_eq!(numbers, vec![0, 1]);
    }

    #[test]
    fn root_videos_and_season_folders_can_coexist() {
        let detail = detail(
            &["Series/Season 2/b S02E01.mkv", "Series/a S01E01.mkv"],
            "Series",
        );
        let seasons = seasons(&detail);
        let numbers: Vec<u32> = seasons.iter().map(|s| s.number).collect();
        assert_eq!(numbers, vec![1, 2]);
        assert_eq!(seasons[0].episodes[0].file, "Series/a S01E01.mkv");
    }

    /// Every spelling of a season folder this app claims to read, crossed with
    /// every episode marker. One combination failing means a real library
    /// quietly loses a season, and the failure is invisible: the folder is not
    /// refused, it falls through to the fallback that numbers unnamed folders
    /// by position, so season 5 on its own comes out as season 1.
    #[test]
    fn every_season_folder_spelling_works_with_every_episode_marker() {
        let folders = [
            "Season 2",
            "season 02",
            "SEASON  2",
            "S02",
            "s2",
            "Stagione 2",
            "Season_2",
            "Season.2",
            "Season-2",
            "Season2",
            "Season 2 - inizio",
            "Season 2 — La vendetta",
            "stagione 2 (2004)",
            "S02 - Finale",
        ];
        // Every one of these names episode 3. Half carry a season of their own
        // and half do not; inside a season folder the folder decides either way.
        let files = [
            "Show S02E03.mkv",
            "Show 2x03.mkv",
            "Show - Episodio 3.mkv",
            "Show - Episode 3.mkv",
            "Ep 3 - Titolo.mkv",
            "E03.mkv",
            "03 - Titolo.mkv",
        ];

        for folder in folders {
            for file in files {
                let path = format!("Series/{folder}/{file}");
                let detail = detail(&[&path], "Series");
                let seasons = seasons(&detail);
                assert_eq!(
                    seasons.len(),
                    1,
                    "{folder} / {file}: expected one season, got {seasons:?}"
                );
                assert_eq!(seasons[0].number, 2, "{folder} / {file}: wrong season");
                assert_eq!(
                    seasons[0].episodes.len(),
                    1,
                    "{folder} / {file}: wrong episode count"
                );
                assert_eq!(
                    seasons[0].episodes[0].number, 3,
                    "{folder} / {file}: wrong episode number"
                );
                assert_eq!(seasons[0].episodes[0].file, path);
            }
        }
    }

    #[test]
    fn a_labelled_season_folder_hands_its_name_to_the_window() {
        // The number alone was reaching the window, so a folder called
        // "Season 1 - inizio" was listed as "Season 1" and two seasons the user
        // had deliberately named differently looked identical.
        let detail = detail(
            &[
                "Series/Season 1 - inizio/a S01E01.mkv",
                "Series/Season 2/b S02E01.mkv",
                "Series/Specials/c E01.mkv",
            ],
            "Series",
        );
        let found = seasons(&detail);
        let titles: Vec<&str> = found.iter().map(|s| s.title.as_str()).collect();
        // Season 2 says nothing its number does not, so the window builds the
        // label itself and this stays empty.
        assert_eq!(titles, vec!["Specials", "Season 1 - inizio", ""]);
    }

    #[test]
    fn a_season_folder_outranks_a_marker_that_disagrees_with_it() {
        // The folder is the one thing the user arranged by hand, so it wins.
        // The episode number still comes from the file, which is the part the
        // folder cannot say.
        let detail = detail(&["Series/Season 4/Show S01E07.mkv"], "Series");
        let seasons = seasons(&detail);
        assert_eq!(seasons.len(), 1);
        assert_eq!(seasons[0].number, 4);
        assert_eq!(seasons[0].episodes[0].number, 7);
    }

    #[test]
    fn a_labelled_season_folder_keeps_its_number_among_others() {
        // The bug as it actually showed up: one folder in a series named with a
        // label after the number. Unrecognised, it fell to the fallback and the
        // whole set was renumbered by position — three seasons named 1, 2, 3
        // whatever their folders said.
        let detail = detail(
            &[
                "Series/Season 1 - inizio/a S01E01.mkv",
                "Series/Season 2/b S02E01.mkv",
                "Series/Season 10 - la fine/c S10E01.mkv",
            ],
            "Series",
        );
        let numbers: Vec<u32> = seasons(&detail).iter().map(|s| s.number).collect();
        assert_eq!(numbers, vec![1, 2, 10]);
    }

    #[test]
    fn seasons_can_come_from_the_markers_alone_with_no_folders_at_all() {
        // A series nobody sorted into folders: every episode loose in the
        // content root, its own marker the only thing saying where it belongs.
        // The seasons that exist are the ones the files name — 1 and 3 here,
        // with no 2 invented to sit between them.
        let detail = detail(
            &["Series/Show S03E01.mkv", "Series/Show S01E01.mkv"],
            "Series",
        );
        let seasons = seasons(&detail);
        let numbers: Vec<u32> = seasons.iter().map(|s| s.number).collect();
        assert_eq!(numbers, vec![1, 3]);
        assert_eq!(seasons[0].episodes[0].file, "Series/Show S01E01.mkv");
        assert_eq!(seasons[1].episodes[0].file, "Series/Show S03E01.mkv");
        assert_eq!(seasons[1].episodes[0].number, 1);
    }

    #[test]
    fn loose_episodes_with_no_season_marker_all_land_in_season_one() {
        // Nothing in the name says which season, so inventing several would be
        // guessing. One season is the honest reading, and the episode numbers
        // are still the file's own.
        let detail = detail(&["Series/E02.mkv", "Series/E01.mkv"], "Series");
        let seasons = seasons(&detail);
        assert_eq!(seasons.len(), 1);
        assert_eq!(seasons[0].number, 1);
        let numbers: Vec<u32> = seasons[0].episodes.iter().map(|e| e.number).collect();
        assert_eq!(numbers, vec![1, 2]);
    }

    #[test]
    fn episodes_are_numbered_from_their_markers() {
        let detail = detail(
            &[
                "Series/Series S01E03.mkv",
                "Series/Series S01E01.mkv",
                "Series/Series S01E02.mkv",
            ],
            "Series",
        );
        let numbers: Vec<u32> = seasons(&detail)[0]
            .episodes
            .iter()
            .map(|e| e.number)
            .collect();
        assert_eq!(numbers, vec![1, 2, 3]);
    }

    #[test]
    fn episodes_keep_their_marker_numbers_even_when_they_have_gaps() {
        let detail = detail(
            &["Series/ep10.mkv", "Series/ep2.mkv", "Series/ep1.mkv"],
            "Series",
        );
        let episodes = &seasons(&detail)[0].episodes;
        let files: Vec<&str> = episodes.iter().map(|e| e.file.as_str()).collect();
        assert_eq!(
            files,
            vec!["Series/ep1.mkv", "Series/ep2.mkv", "Series/ep10.mkv"]
        );
        let numbers: Vec<u32> = episodes.iter().map(|e| e.number).collect();
        assert_eq!(numbers, vec![1, 2, 10]);
    }

    #[test]
    fn episodes_without_markers_are_numbered_by_natural_order() {
        let detail = detail(
            &["Series/Zenith.mkv", "Series/Alpha.mkv", "Series/Mid.mkv"],
            "Series",
        );
        let episodes = &seasons(&detail)[0].episodes;
        let files: Vec<&str> = episodes.iter().map(|e| e.file.as_str()).collect();
        assert_eq!(
            files,
            vec!["Series/Alpha.mkv", "Series/Mid.mkv", "Series/Zenith.mkv"]
        );
        let numbers: Vec<u32> = episodes.iter().map(|e| e.number).collect();
        assert_eq!(numbers, vec![1, 2, 3]);
        assert_eq!(episodes[0].title, "Alpha");
    }

    #[test]
    fn a_season_where_only_some_files_have_markers_falls_back_to_position() {
        let detail = detail(
            &["Series/S01E05 - Cinque.mkv", "Series/Extra scena.mkv"],
            "Series",
        );
        let episodes = &seasons(&detail)[0].episodes;
        let numbers: Vec<u32> = episodes.iter().map(|e| e.number).collect();
        assert_eq!(
            numbers,
            vec![1, 2],
            "mixed naming must not produce duplicate numbers"
        );
    }

    #[test]
    fn episode_titles_come_from_the_filename_when_present() {
        let detail = detail(
            &[
                "Series/Series S01E01 - Pilot.mkv",
                "Series/Series S01E02.mkv",
            ],
            "Series",
        );
        let episodes = &seasons(&detail)[0].episodes;
        assert_eq!(episodes[0].title, "Pilot");
        assert_eq!(episodes[1].title, "");
    }

    #[test]
    fn episodes_pick_up_their_own_subtitles() {
        let detail = detail(
            &[
                "Series/Season 1/S01E01.mkv",
                "Series/Season 1/S01E01.it.srt",
                "Series/Season 1/S01E02.mkv",
            ],
            "Series",
        );
        let episodes = &seasons(&detail)[0].episodes;
        assert_eq!(episodes[0].subtitles, vec!["Series/Season 1/S01E01.it.srt"]);
        assert!(episodes[1].subtitles.is_empty());
    }

    #[test]
    fn unknown_extensions_are_ignored() {
        let detail = detail(
            &["Movie/Movie.mkv", "Movie/note.txt", "Movie/thumbs.db"],
            "Movie",
        );
        assert!(matches!(detail.body, ContentBody::Movie { .. }));
    }

    #[test]
    fn subfolders_that_are_not_named_like_seasons_still_play() {
        // "Prima Stagione" does not match the season patterns, but the episodes
        // inside it are real content and must not become unreachable.
        let detail = detail(
            &[
                "Series/Prima Stagione/ep01.mkv",
                "Series/Prima Stagione/ep02.mkv",
            ],
            "Series",
        );
        let seasons = seasons(&detail);
        assert_eq!(seasons.len(), 1);
        assert_eq!(seasons[0].title, "Prima Stagione");
        assert_eq!(seasons[0].episodes.len(), 2);
    }

    #[test]
    fn several_unrecognised_subfolders_become_seasons_in_natural_order() {
        let detail = detail(
            &[
                "Series/Parte 10/c.mkv",
                "Series/Parte 2/b.mkv",
                "Series/Parte 1/a.mkv",
            ],
            "Series",
        );
        let found = seasons(&detail);
        let titles: Vec<&str> = found.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["Parte 1", "Parte 2", "Parte 10"]);
    }

    #[test]
    fn a_folder_holding_only_subfolders_is_a_series() {
        let scan = scan(&["Series/Prima Stagione/ep01.mkv"]);
        assert_eq!(scan.contents.len(), 1);
        assert_eq!(scan.contents[0].kind, ContentKind::Series);
    }

    #[test]
    fn a_folder_whose_subfolders_hold_no_video_is_skipped() {
        // The home grid must not offer something the detail view will refuse.
        let scan = scan(&["Junk/notes/readme.txt", "Junk/cover.jpg"]);
        assert!(scan.contents.is_empty());
        assert_eq!(scan.warnings.len(), 1);
        assert!(scan.warnings[0].contains("Junk"));
    }

    #[test]
    fn recognised_season_folders_still_win_over_plain_subfolders() {
        let detail = detail(
            &["Series/Season 1/a S01E01.mkv", "Series/Bonus/b.mkv"],
            "Series",
        );
        let numbers: Vec<u32> = seasons(&detail).iter().map(|s| s.number).collect();
        // "Bonus" is in the specials list, so it is season 0, not an invented one.
        assert_eq!(numbers, vec![0, 1]);
    }

    #[test]
    fn non_season_subfolders_are_ignored() {
        let detail = detail(
            &["Movie/Movie.mkv", "Movie/Behind the scenes/making-of.mkv"],
            "Movie",
        );
        assert!(matches!(detail.body, ContentBody::Movie { .. }));
    }

    #[test]
    fn content_without_any_video_is_an_error() {
        let fs = FakeFs::with_tree(ROOT, &["Movie/cover.jpg"]);
        assert_eq!(
            scan_content(&fs, &root(), "Movie"),
            Err(ScanError::NoPlayableFile("Movie".to_string()))
        );
    }

    #[test]
    fn unknown_content_id_is_an_error() {
        let fs = FakeFs::with_tree(ROOT, &["Movie/Movie.mkv"]);
        assert_eq!(
            scan_content(&fs, &root(), "Unknown"),
            Err(ScanError::ContentNotFound("Unknown".to_string()))
        );
    }

    #[test]
    fn content_id_cannot_escape_the_media_root() {
        let fs = FakeFs::with_tree(ROOT, &["Movie/Movie.mkv"]);
        for id in ["../secrets", "..", "sub/../../etc", "/etc", "C:\\Windows"] {
            assert_eq!(
                scan_content(&fs, &root(), id),
                Err(ScanError::ContentNotFound(id.to_string())),
                "id: {id}"
            );
        }
    }

    fn fs_with_mtimes(paths: &[&str], mtime: u64) -> FakeFs {
        let mut fs = FakeFs::with_tree(ROOT, paths);
        for path in paths {
            let id = path.split('/').next().expect("id");
            fs.set_mtime(&root().join(id), mtime);
        }
        fs
    }

    #[test]
    fn a_fresh_scan_produces_a_cache_entry_per_folder() {
        let fs = fs_with_mtimes(&["Movie/Movie.mkv", "Empty/cover.jpg"], 100);
        let (scan, cache) =
            scan_library_cached(&fs, &root(), &LibraryCacheData::default()).expect("scan");

        assert_eq!(scan.contents.len(), 1);
        assert_eq!(cache.entries.len(), 2);
        assert!(cache.entries["Movie"].summary.is_some());
        assert!(
            cache.entries["Empty"].summary.is_none(),
            "skipped folders are cached too"
        );
    }

    #[test]
    fn an_unchanged_folder_is_served_from_cache_without_reading_it() {
        let paths = ["Movie/Movie.mkv"];
        let mut fs = fs_with_mtimes(&paths, 100);
        let (_, cache) =
            scan_library_cached(&fs, &root(), &LibraryCacheData::default()).expect("first scan");

        // If the cache is consulted the folder is never opened, so making it
        // unreadable must change nothing.
        fs.make_unreadable(&root().join("Movie"));
        let (scan, _) = scan_library_cached(&fs, &root(), &cache).expect("cached scan");

        assert_eq!(scan.contents.len(), 1);
        assert_eq!(scan.contents[0].id, "Movie");
        assert!(scan.warnings.is_empty());
    }

    #[test]
    fn a_changed_mtime_forces_a_rescan() {
        let mut fs = fs_with_mtimes(&["Movie/Movie.mkv"], 100);
        let (_, cache) =
            scan_library_cached(&fs, &root(), &LibraryCacheData::default()).expect("first scan");

        fs.set_mtime(&root().join("Movie"), 200);
        fs.make_unreadable(&root().join("Movie"));
        let (scan, _) = scan_library_cached(&fs, &root(), &cache).expect("rescan");

        assert!(scan.contents.is_empty(), "stale cache must not be trusted");
        assert_eq!(scan.warnings.len(), 1);
    }

    #[test]
    fn a_cached_skip_is_still_reported_as_a_warning() {
        let fs = fs_with_mtimes(&["Empty/cover.jpg"], 100);
        let (_, cache) =
            scan_library_cached(&fs, &root(), &LibraryCacheData::default()).expect("first scan");
        let (scan, _) = scan_library_cached(&fs, &root(), &cache).expect("cached scan");

        assert!(scan.contents.is_empty());
        assert_eq!(scan.warnings.len(), 1);
    }

    #[test]
    fn a_tampered_cache_entry_is_ignored_and_the_folder_rescanned() {
        // The cache file sits on the stick and anyone can edit it; a cover
        // pointing outside its folder must not survive into the UI.
        let fs = fs_with_mtimes(&["Movie/Movie.mkv"], 100);
        let mut poisoned = LibraryCacheData::default();
        poisoned.entries.insert(
            "Movie".to_string(),
            CacheEntry {
                mtime: 100,
                summary: Some(ContentSummary {
                    id: "Movie".to_string(),
                    title: "Movie".to_string(),
                    year: None,
                    cover: Some("../../../etc/passwd".to_string()),
                    kind: ContentKind::Movie,
                }),
            },
        );

        let (scan, _) = scan_library_cached(&fs, &root(), &poisoned).expect("scan");
        assert_eq!(scan.contents.len(), 1);
        assert_eq!(
            scan.contents[0].cover, None,
            "poisoned cover must not reach the UI"
        );
    }

    #[test]
    fn entries_for_removed_folders_are_dropped_from_the_cache() {
        let fs = fs_with_mtimes(&["Movie/Movie.mkv"], 100);
        let mut stale = LibraryCacheData::default();
        stale.entries.insert(
            "Gone".to_string(),
            CacheEntry {
                mtime: 100,
                summary: None,
            },
        );

        let (_, cache) = scan_library_cached(&fs, &root(), &stale).expect("scan");
        assert!(!cache.entries.contains_key("Gone"));
        assert!(cache.entries.contains_key("Movie"));
    }

    #[test]
    fn folders_without_a_readable_mtime_are_scanned_every_time() {
        // No mtimes registered: nothing can be invalidated, so nothing is cached.
        let fs = FakeFs::with_tree(ROOT, &["Movie/Movie.mkv"]);
        let (scan, cache) =
            scan_library_cached(&fs, &root(), &LibraryCacheData::default()).expect("scan");

        assert_eq!(scan.contents.len(), 1);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn detail_repeats_the_summary() {
        let detail = detail(
            &["Inception (2010)/cover.jpg", "Inception (2010)/f.mkv"],
            "Inception (2010)",
        );
        assert_eq!(detail.summary.title, "Inception");
        assert_eq!(detail.summary.year, Some(2010));
        assert_eq!(
            detail.summary.cover.as_deref(),
            Some("Inception (2010)/cover.jpg")
        );
    }
}
