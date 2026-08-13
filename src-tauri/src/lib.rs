pub mod adapters;
pub mod core;
pub mod defaults;
pub mod ports;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{Manager, State};

use crate::adapters::ffmpeg;
use crate::adapters::json_cache::JsonCache;
use crate::adapters::json_progress::JsonProgress;
use crate::adapters::std_fs::{self, StdFs};
use crate::adapters::stream_server::{StreamConfig, StreamServer};
use crate::core::media::{self, Delivery, MediaProfile};
use crate::core::model::ContentDetail;
// Aliased because `progress` is also the name of a command below, and
// `progress::up_next` beside `fn up_next` reads as one thing calling itself.
use crate::core::progress as watched;
use crate::core::progress::{Mark, ProgressData, UpNext};
use crate::core::{paths, scanner};
use crate::ports::cache::LibraryCache;
use crate::ports::progress::ProgressStore;

struct AppState {
    server: Arc<StreamServer>,
    ffprobe: PathBuf,
    /// The watch history, read once at startup and written after every change.
    ///
    /// Held rather than re-read because the alternative is a load-modify-save
    /// on every twenty-second tick of a film. The lock is what keeps two
    /// commands from interleaving a read and a write of the same map; two
    /// copies of the app on one stick can still overwrite each other, which is
    /// the same bargain the cache already makes and for the same reason —
    /// locking across machines is not a thing a portable app can do.
    history: Mutex<ProgressData>,
    store: JsonProgress,
}

impl AppState {
    fn remember(&self, change: impl FnOnce(&mut ProgressData) -> Option<Mark>) -> Option<Mark> {
        remember_in(&self.history, &self.store, change)
    }

    fn history(&self) -> ProgressData {
        held(&self.history).clone()
    }
}

/// The map behind the lock, whether or not a panic elsewhere poisoned it.
///
/// A poisoned lock here means some other command panicked while holding it. The
/// map itself is still a map; refusing to remember anything for the rest of the
/// session is the worse of the two outcomes, and the alternative on the way out
/// would be a second panic.
fn held(history: &Mutex<ProgressData>) -> std::sync::MutexGuard<'_, ProgressData> {
    history
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Apply a change to the history and put it on the stick.
///
/// A failed write is logged, never returned: on a read-only stick this would
/// otherwise raise an error box every twenty seconds for the whole length of a
/// film. The session still remembers everything; the stick does not.
///
/// Free of `AppState` so that both failure paths can be tested without starting
/// a stream server and a window to reach them.
fn remember_in(
    history: &Mutex<ProgressData>,
    store: &dyn ProgressStore,
    change: impl FnOnce(&mut ProgressData) -> Option<Mark>,
) -> Option<Mark> {
    let mut history = held(history);
    let mark = change(&mut history)?;
    if let Err(e) = store.save(&history) {
        eprintln!("watch history not saved: {e}");
    }
    Some(mark)
}

/// Unix seconds, for ordering "continue watching".
///
/// A clock set before 1970 reads as 0, which costs the ordering and nothing
/// else — not a reason to fail a command that is otherwise fine.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Everything the home screen needs, including the case where there is nothing
/// to show yet: a missing media folder is a normal state, not an error.
#[derive(Debug, Serialize)]
pub struct LibraryView {
    /// Absolute path the app looked at, so the welcome screen can name it.
    media_root: String,
    media_root_exists: bool,
    contents: Vec<core::model::ContentSummary>,
    warnings: Vec<String>,
}

/// What the player needs to start: where to fetch the video, how it is being
/// delivered, and which subtitles it can offer.
#[derive(Debug, Serialize)]
pub struct PlaybackSource {
    url: String,
    delivery: Delivery,
    duration: Option<f64>,
    title: String,
    /// Source video codec, handed back so a seek can rebuild the same stream.
    video_codec: Option<String>,
    subtitles: Vec<SubtitleSource>,
    /// Audio tracks to choose from, and which one this stream carries.
    audio: Vec<AudioSource>,
    audio_track: u32,
    /// Second of the film this stream begins at, so the player can anchor its
    /// clock without assuming the request was honoured verbatim.
    offset: f64,
    /// Subtitle tracks the file carries that cannot be shown, because they are
    /// pictures rather than text — PGS on most Blu-ray rips, VobSub on DVD
    /// ones. Dropped in silence they were indistinguishable from a film with no
    /// subtitles at all, which is the same thing as subtitles being broken.
    bitmap_subtitles: usize,
}

#[derive(Debug, Serialize)]
pub struct AudioSource {
    /// Index among audio streams — what a later request names to get it back.
    index: u32,
    label: String,
    language: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SubtitleSource {
    url: String,
    label: String,
    language: Option<String>,
    /// The file this track is read from, and which track inside it. Handed back
    /// so a restarted stream can ask for the same track cut to its own offset:
    /// a track is tied to the clock of the stream it accompanies, not to the
    /// film, and after a seek the two are no longer the same.
    path: String,
    track: u32,
}

/// Media folder next to the executable. `REELDRIVE_MEDIA` overrides it,
/// which is what makes `tauri dev` usable without copying content into
/// `target/debug/`.
fn media_root() -> Result<PathBuf, String> {
    if let Some(from_env) = std::env::var_os("REELDRIVE_MEDIA") {
        return Ok(PathBuf::from(from_env));
    }
    std_fs::media_root().map_err(|e| format!("could not locate the app folder: {e}"))
}

#[tauri::command]
fn library() -> Result<LibraryView, String> {
    let root = media_root()?;
    let display_root = root.display().to_string();

    if !root.is_dir() {
        return Ok(LibraryView {
            media_root: display_root,
            media_root_exists: false,
            contents: Vec::new(),
            warnings: Vec::new(),
        });
    }

    let cache_file = JsonCache::in_media_root(&root);
    let cached = cache_file.load().unwrap_or_default();

    let (scan, fresh) = scanner::scan_library_cached(&StdFs, &root, &cached)
        .map_err(|e| format!("scan failed: {e}"))?;

    // A read-only stick simply gets no cache; that is not worth failing over.
    if fresh != cached {
        if let Err(e) = cache_file.store(&fresh) {
            eprintln!("cache not saved: {e}");
        }
    }

    Ok(LibraryView {
        media_root: display_root,
        media_root_exists: true,
        contents: scan.contents,
        warnings: scan.warnings,
    })
}

#[tauri::command]
fn content(id: String) -> Result<ContentDetail, String> {
    let root = media_root()?;
    scanner::scan_content(&StdFs, &root, &id).map_err(|e| e.to_string())
}

/// Inspect a file and hand back everything needed to play it.
///
/// The probe runs per request rather than during the library scan: it costs a
/// process launch per file, which is fine once but would make opening the app
/// crawl if it happened for every title on the stick.
#[tauri::command]
fn playback_source(
    path: String,
    external: Vec<String>,
    audio: Option<u32>,
    start: Option<f64>,
    state: State<'_, AppState>,
) -> Result<PlaybackSource, String> {
    let root = media_root()?;
    let file = paths::resolve_media_file(&root, &path).map_err(|e| e.to_string())?;

    let profile: MediaProfile = ffmpeg::probe(&state.ffprobe, &file).map_err(|e| e.to_string())?;
    // Changing the audio track re-enters here rather than getting a route of
    // its own: the delivery decision depends on the track that was chosen, and
    // so do the subtitles, which have to be re-cut to the new stream's clock.
    let track = profile.audio_track_or_first(audio.unwrap_or(0));
    let offset = clamp_start(start.unwrap_or(0.0));
    // Decided for the track, then for the offset: a stream that has to begin
    // midway cannot be the untouched file. The player reads this word to decide
    // whether the element can seek by itself, so it has to describe the stream
    // that is actually built — see `media::delivery_at`.
    let delivery = media::delivery_at(
        profile.delivery_for(track, media::Capabilities::current()),
        offset,
        track,
    );

    let mut subtitles: Vec<SubtitleSource> = profile
        .usable_subtitles()
        .map(|track| SubtitleSource {
            url: state.server.subtitle_url(&path, track.index, offset),
            label: track_label(
                track.language.as_deref(),
                track.title.as_deref(),
                track.index,
            ),
            language: track.language.clone(),
            path: path.clone(),
            track: track.index,
        })
        .collect();

    // Subtitle files sitting next to the video. The scanner already found them
    // and the detail page already counts them; without this they were listed
    // and then never offered. A standalone subtitle file holds exactly one
    // track, so it is extracted the same way an embedded one is.
    for relative in &external {
        if paths::resolve_media_file(&root, relative).is_err() {
            continue;
        }
        subtitles.push(SubtitleSource {
            url: state.server.subtitle_url(relative, 0, offset),
            label: external_subtitle_label(relative),
            language: None,
            path: relative.clone(),
            track: 0,
        });
    }

    let audio_tracks: Vec<AudioSource> = profile
        .audio
        .iter()
        .map(|t| AudioSource {
            index: t.index,
            label: track_label(t.language.as_deref(), t.title.as_deref(), t.index),
            language: t.language.clone(),
        })
        .collect();

    Ok(PlaybackSource {
        url: state
            .server
            .stream_url(
                &path,
                offset,
                delivery,
                profile.video_codec.as_deref(),
                track,
            )
            .map_err(|e| format!("could not start playback: {e}"))?,
        delivery,
        duration: profile.duration,
        title: file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string(),
        video_codec: profile.video_codec.clone(),
        subtitles,
        audio: audio_tracks,
        audio_track: track,
        offset,
        bitmap_subtitles: profile.subtitles.iter().filter(|t| !t.textual).count(),
    })
}

/// Seeking past what has been produced restarts the conversion at an offset.
///
/// An event playlist lets the player seek by itself anywhere inside the
/// segments that already exist, so this is asked for only when the target lies
/// beyond them — and the new session supersedes the one it replaces.
#[tauri::command]
fn seek_url(
    path: String,
    seconds: f64,
    delivery: Delivery,
    video_codec: Option<String>,
    audio: Option<u32>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let root = media_root()?;
    paths::resolve_media_file(&root, &path).map_err(|e| e.to_string())?;
    state
        .server
        .stream_url(
            &path,
            clamp_start(seconds),
            delivery,
            video_codec.as_deref(),
            audio.unwrap_or(0),
        )
        .map_err(|e| format!("could not start playback: {e}"))
}

/// A new stream carrying a different audio track, from `seconds` in.
///
/// Not a seek with an extra argument: which treatment the file needs depends on
/// the track that was chosen — a film often holds one track the platform plays
/// untouched and a dubbed one it cannot — so the file is probed again and the
/// delivery decided for the track actually being asked for.
#[tauri::command]
fn audio_url(
    path: String,
    seconds: f64,
    audio: u32,
    state: State<'_, AppState>,
) -> Result<AudioStream, String> {
    let root = media_root()?;
    let file = paths::resolve_media_file(&root, &path).map_err(|e| e.to_string())?;
    let profile: MediaProfile = ffmpeg::probe(&state.ffprobe, &file).map_err(|e| e.to_string())?;

    let track = profile.audio_track_or_first(audio);
    let offset = clamp_start(seconds);
    // Same two steps as `playback_source`, and for the same reason: changing
    // language mid-film asks for a stream from `offset`, which the untouched
    // file cannot give.
    let delivery = media::delivery_at(
        profile.delivery_for(track, media::Capabilities::current()),
        offset,
        track,
    );

    Ok(AudioStream {
        url: state
            .server
            .stream_url(
                &path,
                offset,
                delivery,
                profile.video_codec.as_deref(),
                track,
            )
            .map_err(|e| format!("could not start playback: {e}"))?,
        delivery,
        audio_track: track,
        offset,
    })
}

#[derive(Debug, Serialize)]
pub struct AudioStream {
    url: String,
    delivery: Delivery,
    /// The track that was actually used: asking for one the file lacks comes
    /// back as the first, and the player has to show what is playing.
    audio_track: u32,
    offset: f64,
}

/// The same subtitle track, cut to the offset a restarted stream begins at.
///
/// The element's clock restarts at zero every time ffmpeg is asked for a stream
/// from `seconds` in, and `<track>` cues are read against that clock — so a
/// track extracted from the whole film is `seconds` late, which past the first
/// seek means no subtitle ever appears again. Asked for alongside every new
/// stream, for the same reason the stream itself is asked for.
#[tauri::command]
fn subtitle_url(
    path: String,
    track: u32,
    seconds: f64,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let root = media_root()?;
    paths::resolve_media_file(&root, &path).map_err(|e| e.to_string())?;
    Ok(state
        .server
        .subtitle_url(&path, track, clamp_start(seconds)))
}

/// A start offset from the webview, reduced to something ffmpeg can be handed:
/// NaN, infinity and negatives all mean "from the beginning".
fn clamp_start(seconds: f64) -> f64 {
    if seconds.is_finite() && seconds > 0.0 {
        seconds
    } else {
        0.0
    }
}

/// Second attempt when the webview refuses what it was given.
///
/// The HEVC guess is the one that can be wrong: the platform is expected to
/// decode it, and if it does not the picture never appears and no error names
/// the cause. Converting is slow but always works.
#[tauri::command]
fn fallback_url(
    path: String,
    seconds: f64,
    audio: Option<u32>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let root = media_root()?;
    paths::resolve_media_file(&root, &path).map_err(|e| e.to_string())?;
    // Re-encoding produces H.264 whatever went in, so the source codec is of no
    // further interest here.
    state
        .server
        .stream_url(
            &path,
            clamp_start(seconds),
            Delivery::Transcode,
            None,
            audio.unwrap_or(0),
        )
        .map_err(|e| format!("could not start playback: {e}"))
}

/// Leaving the player ends the conversion that was feeding it.
///
/// Until this existed the only two things that stopped an ffmpeg were starting
/// another film and closing the window: walking out of a two-hour film after a
/// minute left it converting the whole of it into the temp directory, at full
/// speed, with nobody reading a byte.
#[tauri::command]
fn stop_playback(state: State<'_, AppState>) {
    state.server.stop();
}

/// Where to find the author. The name lives in the window, which is the only
/// place it is ever shown; a second copy here was read by nothing, and the two
/// had to be renamed together with nothing to say so if one was forgotten.
///
/// The address is here rather than there because it is not a label: it is what
/// the command below hands to a shell, and it stays a constant so there is no
/// parameter to abuse.
pub const AUTHOR_URL: &str = "https://luise.ac";

/// Open the author's site in the user's own browser.
///
/// The address is a constant, not an argument. A command that takes a URL and
/// hands it to the shell opens whatever the page can be talked into asking for,
/// and a credit line needs exactly one address — so there is no parameter to
/// abuse and nothing to validate.
///
/// A process rather than a plugin: the app already runs ffmpeg this way, and
/// one line of `open` is a smaller thing to carry than another dependency.
/// Waited on rather than abandoned, like every other child here — so the exit
/// code can say the launcher found nothing to hand the address to, which on the
/// machines this ships to is the likely answer.
///
/// `async`, and that is the whole point of the word here. A command without it
/// runs inline on the thread that answers the window, and the wait below is not
/// as short as it looks: `open` and `start` hand the URL over and return, but
/// `xdg-open` falls back to running `$BROWSER` in the foreground and does not
/// come back until that browser is closed. Blocking there froze the window —
/// no scroll, no Escape, no close button — for the length of a browsing
/// session. Every other blocking child in this app already runs on a thread of
/// its own; the stream server's ffmpeg waits on its own request thread.
#[tauri::command(async)]
fn open_author_site() -> Result<(), String> {
    let (program, leading): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(target_os = "windows") {
        // `start` is a shell builtin, and its first quoted argument is taken as
        // the window title rather than the thing to open — hence the empty one.
        //
        // The address being a constant is what keeps this safe: Rust quotes an
        // argument only when it holds a space, and `cmd` reads an unquoted `&`
        // as the end of one command and the start of the next. The day the
        // address becomes a parameter, this line is where it goes wrong first.
        ("cmd", &["/C", "start", ""])
    } else {
        ("xdg-open", &[])
    };

    let status = std::process::Command::new(program)
        .args(leading)
        .arg(AUTHOR_URL)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("could not open {AUTHOR_URL}: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("could not open {AUTHOR_URL} ({status})"))
    }
}

/// Everything the app remembers about what has been watched.
///
/// Handed over whole, once, when the library loads: it is a few hundred short
/// rows at most, and the alternative is a round trip per tile.
#[tauri::command]
fn progress(state: State<'_, AppState>) -> BTreeMap<String, Mark> {
    state.history().items
}

/// Remember where `path` got to.
///
/// Returns what was actually stored, which may be nothing: a few seconds in is
/// not a position, and the rule for that lives in `core` — see
/// `ProgressData::record`.
#[tauri::command]
fn record_progress(
    path: String,
    seconds: f64,
    duration: f64,
    state: State<'_, AppState>,
) -> Result<Option<Mark>, String> {
    let root = media_root()?;
    // The same trust boundary every other command that takes a path goes
    // through. The key is a path, and this file is read back on the next start.
    paths::resolve_media_file(&root, &path).map_err(|e| e.to_string())?;
    Ok(state.remember(|history| history.record(&path, seconds, duration, now_secs())))
}

/// Move a file to the head of the history without claiming any of it was
/// watched, which is what the end of an episode does to the next one.
///
/// Without it a series drops out of "continue watching" for the first fifteen
/// seconds of every episode — and if the viewer stops inside that window, the
/// only thing left to show is the episode they just finished.
#[tauri::command]
fn take_up(
    path: String,
    duration: f64,
    state: State<'_, AppState>,
) -> Result<Option<Mark>, String> {
    let root = media_root()?;
    paths::resolve_media_file(&root, &path).map_err(|e| e.to_string())?;
    Ok(state.remember(|history| history.touch(&path, duration, now_secs())))
}

/// The episode to play now for one content, seasons walked in order.
///
/// A deep scan of one folder, which is what the detail page already costs. The
/// answer is derived rather than remembered on purpose: see `core::progress`.
#[tauri::command]
fn up_next(id: String, state: State<'_, AppState>) -> Result<Option<UpNext>, String> {
    let root = media_root()?;
    let detail = scanner::scan_content(&StdFs, &root, &id).map_err(|e| e.to_string())?;
    Ok(watched::up_next(&detail, &state.history()))
}

/// The episode that follows `path`, whatever has or has not been watched.
///
/// Not the same question as `up_next`, and the difference is the point: that
/// one answers "where do I carry on with this series", which can be behind you,
/// while this one answers "what comes after this", which is what the skip
/// button and the end of an episode both mean.
#[tauri::command]
fn next_after(
    id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<Option<UpNext>, String> {
    let root = media_root()?;
    let detail = scanner::scan_content(&StdFs, &root, &id).map_err(|e| e.to_string())?;
    Ok(watched::after(&detail, &path, &state.history()))
}

/// Why the conversion stopped, if it did.
///
/// A `<video>` reports a failure with no reason attached — it does not have
/// one — so a conversion ffmpeg refused halfway used to be a picture that
/// simply stopped. The server keeps ffmpeg's own sentence; this is how the
/// error box gets to say it instead of "This file could not be played".
#[tauri::command]
fn playback_failure(state: State<'_, AppState>) -> Option<String> {
    state.server.last_failure()
}

fn track_label(language: Option<&str>, title: Option<&str>, index: u32) -> String {
    match (language, title) {
        (Some(language), Some(title)) => format!("{} · {title}", language.to_uppercase()),
        (Some(language), None) => language.to_uppercase(),
        (None, Some(title)) => title.to_string(),
        (None, None) => format!("Track {}", index + 1),
    }
}

/// Label for a subtitle file, from whatever the name says beyond the video's
/// own: `film.eng.srt` next to `film.mkv` reads as "ENG".
fn external_subtitle_label(relative: &str) -> String {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    match stem.rsplit_once('.') {
        Some((_, suffix)) if !suffix.is_empty() => suffix.to_uppercase(),
        _ => "External".to_string(),
    }
}

/// Build the app and run it until the window closes.
///
/// The error goes back to `main` rather than a panic here: a stick without its
/// ffmpeg, a port that will not bind or a context that fails to build are all
/// ordinary ways for this to fail on a user's machine, and a panic message with
/// a backtrace is the worst possible way to say so.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> tauri::Result<()> {
    tauri::Builder::default()
        .setup(|app| {
            let root = media_root()?;

            // Covers are served straight from the stick, so the asset protocol
            // is opened for the media folder only.
            //
            // Both the path as it is named and the path it resolves to. A media
            // folder reached through a symlink — a stick mounted elsewhere, a
            // developer pointing at content outside the tree — is named one way
            // by the window and canonicalised another way before the check, and
            // then every cover comes back forbidden and every tile falls back
            // to its generated colour with nothing anywhere saying why.
            if root.is_dir() {
                let scope = app.asset_protocol_scope();
                for path in [Some(root.clone()), root.canonicalize().ok()]
                    .into_iter()
                    .flatten()
                {
                    if let Err(e) = scope.allow_directory(&path, true) {
                        eprintln!("covers not readable from {}: {e}", path.display());
                    }
                }
            }

            // The shipped tools travel inside the bundle, next to the binary;
            // the folder the app was launched from is the fallback, so a pair
            // dropped there by hand still works.
            let exe = std_fs::running_exe().unwrap_or_else(|_| PathBuf::from("."));
            let tool_dirs = [
                exe.parent().unwrap_or(Path::new(".")).to_path_buf(),
                std_fs::app_dir_for_exe(&exe),
            ];

            let media_root = root.clone();
            let server = StreamServer::start(StreamConfig {
                media_root: root,
                ffmpeg: ffmpeg::tool_path(&tool_dirs, "ffmpeg"),
            })?;

            // Read once here rather than on every command: a film reports its
            // position every twenty seconds, and each of those would otherwise
            // be a read of the stick as well as a write.
            let store = JsonProgress::in_media_root(&media_root);
            let history = Mutex::new(store.load());

            app.manage(AppState {
                server: Arc::new(server),
                ffprobe: ffmpeg::tool_path(&tool_dirs, "ffprobe"),
                history,
                store,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            library,
            content,
            playback_source,
            seek_url,
            audio_url,
            subtitle_url,
            fallback_url,
            stop_playback,
            playback_failure,
            open_author_site,
            progress,
            record_progress,
            take_up,
            up_next,
            next_after
        ])
        .build(tauri::generate_context!())?
        .run(|app, event| {
            // A conversion is a process and a directory of segments, and neither
            // goes away by itself: the state holding them is not guaranteed to
            // be dropped on the way out, and an ffmpeg that outlives the window
            // keeps a core busy for a film nobody is watching.
            if matches!(event, tauri::RunEvent::Exit) {
                if let Some(state) = app.try_state::<AppState>() {
                    state.server.stop();
                }
            }
        });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subtitle_label_prefers_language_and_title_together() {
        assert_eq!(track_label(Some("ita"), Some("Forced"), 0), "ITA · Forced");
        assert_eq!(track_label(Some("eng"), None, 0), "ENG");
        assert_eq!(track_label(None, Some("Commentary"), 0), "Commentary");
    }

    #[test]
    fn an_unlabelled_track_is_numbered_from_one() {
        // Stream indexes count from zero, but "Track 0" reads as a bug.
        assert_eq!(track_label(None, None, 0), "Track 1");
        assert_eq!(track_label(None, None, 2), "Track 3");
    }

    /// Records what it was asked to write, and can be told to refuse — which is
    /// the read-only stick, the case that must not reach the user as an error.
    struct FakeStore {
        saved: Mutex<Vec<ProgressData>>,
        refuses: bool,
    }

    impl FakeStore {
        fn new(refuses: bool) -> Self {
            Self {
                saved: Mutex::new(Vec::new()),
                refuses,
            }
        }
        fn saves(&self) -> usize {
            self.saved.lock().expect("lock").len()
        }
    }

    impl ProgressStore for FakeStore {
        fn load(&self) -> ProgressData {
            ProgressData::default()
        }
        fn save(&self, data: &ProgressData) -> Result<(), crate::ports::progress::ProgressError> {
            if self.refuses {
                return Err(crate::ports::progress::ProgressError("read-only".into()));
            }
            self.saved.lock().expect("lock").push(data.clone());
            Ok(())
        }
    }

    #[test]
    fn a_remembered_position_is_written_out_once() {
        let history = Mutex::new(ProgressData::default());
        let store = FakeStore::new(false);

        let mark = remember_in(&history, &store, |data| {
            data.record("A/b.mkv", 100.0, 1000.0, 7)
        });

        assert_eq!(mark.map(|m| m.seconds), Some(100.0));
        assert_eq!(store.saves(), 1);
        assert_eq!(store.saved.lock().expect("lock")[0].items.len(), 1);
    }

    #[test]
    fn a_position_the_rules_refuse_is_not_written_at_all() {
        // Four seconds of a film is not a position, and writing the file anyway
        // would mean a save per timeupdate on a stick that gained nothing.
        let history = Mutex::new(ProgressData::default());
        let store = FakeStore::new(false);

        let mark = remember_in(&history, &store, |data| {
            data.record("A/b.mkv", 4.0, 1000.0, 7)
        });

        assert_eq!(mark, None);
        assert_eq!(store.saves(), 0, "an empty change still touched the stick");
    }

    #[test]
    fn a_stick_that_cannot_be_written_to_does_not_fail_the_command() {
        // Every twenty seconds of every film goes through here. Returning the
        // error would put an error box over the picture, on repeat, for a
        // condition the user cannot do anything about mid-film.
        let history = Mutex::new(ProgressData::default());
        let store = FakeStore::new(true);

        let mark = remember_in(&history, &store, |data| {
            data.record("A/b.mkv", 100.0, 1000.0, 7)
        });

        assert_eq!(mark.map(|m| m.seconds), Some(100.0));
        assert_eq!(
            held(&history).items.len(),
            1,
            "the session should still remember it"
        );
    }

    #[test]
    fn a_lock_poisoned_by_an_earlier_panic_still_remembers() {
        // Refusing for the rest of the session is worse than carrying on: the
        // map behind a poisoned lock is still a valid map.
        let history = Mutex::new(ProgressData::default());
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = history.lock().expect("lock");
            panic!("something else went wrong");
        }));
        assert!(history.is_poisoned(), "the setup did not poison the lock");

        let store = FakeStore::new(false);
        let mark = remember_in(&history, &store, |data| {
            data.record("A/b.mkv", 100.0, 1000.0, 7)
        });

        assert_eq!(mark.map(|m| m.seconds), Some(100.0));
        assert_eq!(store.saves(), 1);
    }

    #[test]
    fn an_external_subtitle_is_labelled_by_what_its_name_adds() {
        assert_eq!(external_subtitle_label("Scrubs/S01E01.eng.srt"), "ENG");
        assert_eq!(external_subtitle_label("Movie/film.forced.ass"), "FORCED");
        assert_eq!(external_subtitle_label("Movie/film.srt"), "External");
        assert_eq!(external_subtitle_label("film.srt"), "External");
    }
}
