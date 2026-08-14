//! ffmpeg and ffprobe, as processes next to the executable.
//!
//! Only the argument lists and the parsing live here as pure functions, so the
//! decisions can be tested without running anything.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::core::media::{
    is_hevc, is_textual_subtitle, AudioTrack, Delivery, MediaProfile, SubtitleTrack,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegError(pub String);

impl std::fmt::Display for FfmpegError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for FfmpegError {}

/// Where the tools are. They ship inside the app, but a developer machine has
/// them on PATH, so a bare name is a valid answer.
///
/// `dirs` is searched in order. The shipped copy lives next to the executable
/// — on macOS that is inside the bundle, where the user cannot lose it by
/// tidying up the stick — and the folder the app was launched from is the
/// fallback, so a hand-placed pair still works.
pub fn tool_path(dirs: &[PathBuf], name: &str) -> PathBuf {
    let file_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    dirs.iter()
        .map(|dir| dir.join(&file_name))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Windows flag for "run this child without giving it a console window".
///
/// The app is a GUI binary, ffmpeg and ffprobe are console ones, and Windows
/// hands a console child its own window when the parent has none. Without this
/// a black box flashes up for every probe and one stays on screen for the whole
/// length of every conversion, on top of the film.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Every process this app starts is built here, so no spawn can forget the
/// flag. A plain `Command` everywhere but Windows.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// `ffprobe` arguments for one file. JSON out, no banner, streams and format —
/// the container name and the duration both come from the format block.
pub fn probe_args(file: &Path) -> Vec<String> {
    vec![
        "-v".into(),
        "error".into(),
        "-print_format".into(),
        "json".into(),
        "-show_format".into(),
        "-show_streams".into(),
        file.to_string_lossy().into_owned(),
    ]
}

/// Turn ffprobe's JSON into the profile the delivery decision needs.
///
/// `file` is needed for its extension alone: ffprobe names the *demuxer*, and
/// one demuxer reads several containers.
pub fn parse_probe(json: &str, file: &Path) -> Result<MediaProfile, FfmpegError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| FfmpegError(format!("unreadable probe: {e}")))?;

    let streams = value
        .get("streams")
        .and_then(|s| s.as_array())
        .ok_or_else(|| FfmpegError("probe has no streams".into()))?;

    // Cover art is a video stream in every way ffprobe reports it: a tagged
    // file carries an `mjpeg` or `png` stream that is a still picture, not the
    // film. Mistaken for the film it sends an H.264 that needed nothing through
    // a full conversion.
    let is_cover = |s: &&serde_json::Value| {
        s.get("disposition")
            .and_then(|d| d.get("attached_pic"))
            .and_then(|v| v.as_i64())
            == Some(1)
    };
    let codec_of = |kind: &str| -> Option<String> {
        let of_kind = || {
            streams
                .iter()
                .filter(|s| s.get("codec_type").and_then(|t| t.as_str()) == Some(kind))
        };
        // A file whose only picture *is* the cover — an audio track with
        // artwork — still reports it: calling that "no video stream" would be
        // the bigger lie.
        of_kind()
            .find(|s| !is_cover(s))
            .or_else(|| of_kind().next())
            .and_then(|s| s.get("codec_name"))
            .and_then(|c| c.as_str())
            .map(str::to_string)
    };

    // ffprobe reports every format the demuxer handles, comma-separated:
    // "matroska,webm" for both MKV and WebM, "mov,mp4,m4a,3gp,3g2,mj2" for both
    // MP4 and MOV. Taking the first name calls every WebM a Matroska, and a
    // WebM the element opens by itself was then remuxed into MP4 — a container
    // WebKit will not decode VP8, VP9 or Opus from. The extension says which of
    // the matched formats this file actually is; when it says nothing useful,
    // the first name is still the best guess.
    let format_name = value
        .get("format")
        .and_then(|f| f.get("format_name"))
        .and_then(|n| n.as_str())
        .unwrap_or_default();
    let extension = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase();
    let container = match format_name.split(',').find(|name| *name == extension) {
        Some(named) => named,
        None => format_name.split(',').next().unwrap_or_default(),
    }
    .to_string();

    let duration = value
        .get("format")
        .and_then(|f| f.get("duration"))
        .and_then(|d| d.as_str())
        .and_then(|d| d.parse::<f64>().ok())
        .filter(|d| d.is_finite() && *d > 0.0);

    let subtitles = streams
        .iter()
        .filter(|s| s.get("codec_type").and_then(|t| t.as_str()) == Some("subtitle"))
        .enumerate()
        .map(|(index, stream)| {
            let tags = stream.get("tags");
            SubtitleTrack {
                index: index as u32,
                language: tags
                    .and_then(|t| t.get("language"))
                    .and_then(|l| l.as_str())
                    .map(str::to_string),
                title: tags
                    .and_then(|t| t.get("title"))
                    .and_then(|l| l.as_str())
                    .map(str::to_string),
                textual: stream
                    .get("codec_name")
                    .and_then(|c| c.as_str())
                    .is_some_and(is_textual_subtitle),
            }
        })
        .collect();

    // Numbered among audio streams alone: ffmpeg's `0:a:N` counts those, not
    // the file's stream numbers, and a subtitle stream sitting between two
    // audio ones would otherwise shift every index after it.
    let audio = streams
        .iter()
        .filter(|s| s.get("codec_type").and_then(|t| t.as_str()) == Some("audio"))
        .enumerate()
        .map(|(index, stream)| {
            let tags = stream.get("tags");
            AudioTrack {
                index: index as u32,
                language: tags
                    .and_then(|t| t.get("language"))
                    .and_then(|l| l.as_str())
                    .map(str::to_string),
                title: tags
                    .and_then(|t| t.get("title"))
                    .and_then(|l| l.as_str())
                    .map(str::to_string),
                codec: stream
                    .get("codec_name")
                    .and_then(|c| c.as_str())
                    .map(str::to_string),
            }
        })
        .collect();

    Ok(MediaProfile {
        container,
        video_codec: codec_of("video"),
        audio_codec: codec_of("audio"),
        duration,
        subtitles,
        audio,
    })
}

/// ffmpeg arguments that turn `file` into something the webview can play,
/// starting at `start` seconds and writing fragmented MP4 to stdout.
///
/// `-ss` before `-i` seeks by keyframe before decoding, which is what makes
/// starting mid-film cheap instead of a full scan.
///
/// `video_codec` is the source's video codec, needed only so a copied HEVC
/// stream gets the tag Apple platforms insist on. Absent means "unknown", which
/// costs nothing but the tag.
pub fn stream_args(
    file: &Path,
    delivery: Delivery,
    start: f64,
    video_codec: Option<&str>,
    audio: u32,
) -> Vec<String> {
    let mut args: Vec<String> = vec!["-v".into(), "error".into(), "-nostdin".into()];

    if start > 0.0 {
        args.push("-ss".into());
        args.push(format!("{start:.3}"));
    }

    args.push("-i".into());
    args.push(file.to_string_lossy().into_owned());

    // One video and one audio stream: the chosen one. The `?` keeps a file that
    // has neither from failing outright. Capital `V` is "video streams, not
    // attached pictures": with a lowercase `v` a tagged file maps its cover art
    // and plays a single still frame for the whole running time.
    args.extend([
        "-map".into(),
        "0:V:0?".into(),
        "-map".into(),
        format!("0:a:{audio}?"),
    ]);

    args.extend(codec_args(delivery, video_codec));

    // Fragmented MP4 so the browser can start on the first fragment instead of
    // waiting for an index it will never get on a pipe.
    args.extend([
        "-movflags".into(),
        "frag_keyframe+empty_moov+default_base_moof".into(),
        "-f".into(),
        "mp4".into(),
        "pipe:1".into(),
    ]);

    args
}

/// What to do with the two streams, shared by every output format: the codec
/// decisions are about what the webview can decode, not about how the bytes are
/// wrapped afterwards.
fn codec_args(delivery: Delivery, video_codec: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = match delivery {
        Delivery::Direct | Delivery::Remux => vec!["-c".into(), "copy".into()],
        Delivery::TranscodeAudio => vec![
            "-c:v".into(),
            "copy".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "192k".into(),
            // Downmix: browsers handle stereo AAC everywhere, and 5.1 AAC
            // is where playback goes silent on some platforms.
            "-ac".into(),
            "2".into(),
        ],
        Delivery::Transcode => vec![
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "veryfast".into(),
            "-crf".into(),
            "20".into(),
            // x265 sources are commonly 10-bit, and libx264 keeps the depth
            // it is given: without this the "always works" path emits
            // H.264 High 10, which no browser decodes.
            "-pix_fmt".into(),
            "yuv420p".into(),
            // A segment can only end on a keyframe, and x264 places one every
            // 250 frames by default: `-hls_time 4` then cannot close anything
            // before ten seconds of video have been encoded, so the first
            // segment, the one nothing appears without, costs two to three
            // times what it should. 96 frames is four seconds at 24fps and
            // less above it, which is the boundary being asked for.
            "-g".into(),
            "96".into(),
            // The other half: `-g` is an upper bound and x264 will happily
            // place them further apart on quiet footage without a floor.
            "-keyint_min".into(),
            "96".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "192k".into(),
            "-ac".into(),
            "2".into(),
        ],
    };

    // ffmpeg writes copied HEVC into MP4 tagged `hev1`; WebKit plays `hvc1`
    // only. macOS is the one platform that copies HEVC rather than converting
    // it, so without this the cheap path never reaches the picture.
    let copies_video = !matches!(delivery, Delivery::Transcode);
    if copies_video && video_codec.is_some_and(is_hevc) {
        args.extend(["-tag:v".into(), "hvc1".into()]);
    }

    args
}

/// Name of the initialisation segment, written beside the playlist.
pub const HLS_INIT: &str = "init.mp4";
/// Name of the playlist itself, inside the session directory.
pub const HLS_PLAYLIST: &str = "index.m3u8";

/// ffmpeg arguments producing an HLS playlist and its segments in `dir`.
///
/// This is what actually reaches the picture. A fragmented MP4 pushed down a
/// pipe is a valid file, but WebKit refuses an HTTP `<video>` with no
/// Content-Length and no byte ranges — the same stream plays once it is a
/// playlist of finite segments instead.
///
/// fMP4 segments rather than MPEG-TS: TS cannot carry HEVC without a re-encode,
/// and keeping `-c copy` working for HEVC and AAC is the whole point. The
/// playlist is an `event` one, so it grows as ffmpeg writes and the player can
/// seek anywhere inside what already exists.
pub fn hls_args(
    file: &Path,
    delivery: Delivery,
    start: f64,
    video_codec: Option<&str>,
    audio: u32,
    dir: &Path,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-v".into(),
        "error".into(),
        "-nostdin".into(),
        // The directory is ours and freshly made, but ffmpeg must never stop to
        // ask about a file it finds there.
        "-y".into(),
    ];

    if start > 0.0 {
        args.push("-ss".into());
        args.push(format!("{start:.3}"));
    }

    args.push("-i".into());
    args.push(file.to_string_lossy().into_owned());

    // Capital `V` skips attached pictures; see `stream_args`.
    args.extend([
        "-map".into(),
        "0:V:0?".into(),
        "-map".into(),
        format!("0:a:{audio}?"),
    ]);

    args.extend(codec_args(delivery, video_codec));

    args.extend([
        "-f".into(),
        "hls".into(),
        "-hls_time".into(),
        "4".into(),
        "-hls_playlist_type".into(),
        "event".into(),
        "-hls_segment_type".into(),
        "fmp4".into(),
        // Bare name on purpose: ffmpeg writes it into the playlist verbatim, and
        // the player resolves it against the playlist's URL. A path here would
        // become a URI pointing at this machine's filesystem.
        "-hls_fmp4_init_filename".into(),
        HLS_INIT.into(),
        "-hls_segment_filename".into(),
        dir.join("seg%05d.m4s").to_string_lossy().into_owned(),
        dir.join(HLS_PLAYLIST).to_string_lossy().into_owned(),
    ]);

    args
}

/// ffmpeg arguments extracting one subtitle track as WebVTT, from `start`
/// seconds in.
///
/// `start` is the offset of the stream this track accompanies, not a
/// convenience: a restarted stream begins at zero on the element's clock, so a
/// track still carrying the film's own timeline would be `start` seconds late —
/// which for anything past a seek means no visible subtitle at all.
///
/// Three arguments do that, and none of them is optional:
///
/// * `-ss` **before** `-i` seeks the file cheaply, without reading the hours
///   before the offset. On its own it is not a rebasing: it lands on the
///   nearest subtitle *packet* at or before `start` and then makes *that* zero.
///   A film whose first line is at 00:01:00 and a seek to 01:00:00 came back
///   shifted by a minute rather than by an hour — every cue an hour early, and
///   every line from before the seek still in the file.
/// * `-copyts` cancels that rebasing and keeps the film's own timestamps, so
///   there is something exact left to shift.
/// * `-ss` **after** `-i` then drops what falls before the offset, and
///   `-output_ts_offset` moves the rest onto the stream's clock.
///
/// A line already on screen when the seek lands is dropped rather than
/// truncated: its cue begins before `start`. One line, at the moment the user
/// jumped somewhere else.
pub fn subtitle_args(file: &Path, track: u32, start: f64) -> Vec<String> {
    let mut args: Vec<String> = vec!["-v".into(), "error".into(), "-nostdin".into()];

    if start > 0.0 {
        args.push("-ss".into());
        args.push(format!("{start:.3}"));
        args.push("-copyts".into());
    }

    args.extend([
        "-i".into(),
        file.to_string_lossy().into_owned(),
        "-map".into(),
        format!("0:s:{track}"),
    ]);

    if start > 0.0 {
        args.extend([
            "-ss".into(),
            format!("{start:.3}"),
            "-output_ts_offset".into(),
            format!("-{start:.3}"),
        ]);
    }

    args.extend(["-f".into(), "webvtt".into(), "pipe:1".into()]);

    args
}

/// What to say when one of the tools is not where it should be.
///
/// This is the failure a user actually meets: the app copied out of the folder
/// it was built into, or a build that was assembled by hand and never given its
/// pair. `No such file or directory (os error 2)` names neither the file that
/// is missing nor the folder it was wanted in, so there is nothing in it to
/// act on. `tool_path` falls back to a bare name for a developer machine that
/// has them on PATH, and from Finder there is no PATH to speak of — which is
/// why the message has to cover the bare name too.
pub fn missing_tool(tool: &Path, error: &std::io::Error) -> FfmpegError {
    let name = tool
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("the tool");
    if error.kind() != std::io::ErrorKind::NotFound {
        return FfmpegError(format!("could not run {name}: {error}"));
    }
    // A bare name is what `tool_path` hands back when it found nothing — the
    // PATH fallback for a developer machine, and from Finder there is no PATH
    // worth the name. That is the case the user meets, so it is the one that
    // has to name the folder, and the folder is where the running app is.
    let expected = tool
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or_else(|| {
            crate::adapters::std_fs::running_exe()
                .ok()
                .and_then(|exe| exe.parent().map(Path::to_path_buf))
        });
    match expected {
        Some(dir) => FfmpegError(format!(
            "{name} is missing. It belongs next to the app, in {}. \
             Rebuild with `just build`, which fetches it, or copy {name} there.",
            dir.display()
        )),
        None => FfmpegError(format!(
            "{name} is missing. It belongs next to the app. \
             Rebuild with `just build`, which fetches it."
        )),
    }
}

/// Run ffprobe and read the file's profile.
pub fn probe(ffprobe: &Path, file: &Path) -> Result<MediaProfile, FfmpegError> {
    let output = command(ffprobe)
        .args(probe_args(file))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| missing_tool(ffprobe, &e))?;

    if !output.status.success() {
        let reason = String::from_utf8_lossy(&output.stderr);
        return Err(FfmpegError(format!("ffprobe failed: {}", reason.trim())));
    }

    parse_probe(&String::from_utf8_lossy(&output.stdout), file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_tool_says_which_one_and_where_it_belongs() {
        // The path the user actually meets. `tool_path` found nothing and fell
        // back to a bare name, so there is no directory in the path itself —
        // the first version of this message therefore named nothing at all,
        // and the test that passed was testing an absolute path the app never
        // builds. "No such file or directory (os error 2)" names neither the
        // file that is missing nor the folder it was wanted in.
        // Driven through `missing_tool` rather than `probe`: a developer
        // machine has ffprobe on PATH, so the bare name resolves there and the
        // run fails on the film instead. The mapping is what is being fixed
        // here, and it must not depend on whose machine runs the test.
        let bare = tool_path(&[], "ffprobe");
        assert_eq!(bare, PathBuf::from("ffprobe"), "the case under test");

        let absent = std::io::Error::from(std::io::ErrorKind::NotFound);
        let text = missing_tool(&bare, &absent).to_string();

        assert!(text.contains("ffprobe"), "should name the tool: {text}");
        assert!(
            text.contains("next to the app"),
            "should say where it belongs: {text}"
        );
        assert!(
            text.contains("just build"),
            "should say how to get it back: {text}"
        );
        assert!(
            !text.contains("os error"),
            "an errno is not something a user can act on: {text}"
        );
    }

    #[test]
    fn a_missing_tool_names_the_folder_when_the_path_has_one() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let absent = dir.path().join("ffprobe");

        let error = probe(&absent, Path::new("/m/a.mkv")).expect_err("a tool that is not there");
        assert!(
            error
                .to_string()
                .contains(&dir.path().display().to_string()),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_tool_that_is_there_but_fails_reports_what_it_said() {
        // The opposite case must not be swallowed by the message above: the
        // tool ran, and its own complaint is the useful part.
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::TempDir::new().expect("tempdir");
        let broken = dir.path().join("ffprobe");
        std::fs::write(&broken, "#!/bin/sh\nprintf boom 1>&2\nexit 3\n").expect("write");
        std::fs::set_permissions(&broken, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        let error = probe(&broken, Path::new("/m/a.mkv")).expect_err("a tool that fails");
        assert!(error.to_string().contains("boom"), "{error}");
    }

    const TWO_AUDIO_PROBE: &str = r#"{
        "streams": [
            {"codec_type":"video","codec_name":"h264"},
            {"codec_type":"audio","codec_name":"aac","tags":{"language":"eng","title":"Original"}},
            {"codec_type":"subtitle","codec_name":"subrip","tags":{"language":"ita"}},
            {"codec_type":"audio","codec_name":"ac3","tags":{"language":"ita"}}
        ],
        "format": {"format_name":"matroska,webm","duration":"600.0"}
    }"#;

    #[test]
    fn audio_tracks_are_listed_with_what_names_them() {
        let profile = parse_probe(TWO_AUDIO_PROBE, Path::new("/m/film.mkv")).expect("probe");
        assert_eq!(profile.audio.len(), 2);
        assert_eq!(profile.audio[0].language.as_deref(), Some("eng"));
        assert_eq!(profile.audio[0].title.as_deref(), Some("Original"));
        assert_eq!(profile.audio[0].codec.as_deref(), Some("aac"));
        assert_eq!(profile.audio[1].language.as_deref(), Some("ita"));
        assert_eq!(profile.audio[1].codec.as_deref(), Some("ac3"));
    }

    #[test]
    fn audio_indexes_count_audio_streams_only() {
        // The second audio track is stream 3 in the file but `0:a:1` to ffmpeg;
        // handing it the stream number plays the wrong thing, or nothing.
        let profile = parse_probe(TWO_AUDIO_PROBE, Path::new("/m/film.mkv")).expect("probe");
        let indexes: Vec<u32> = profile.audio.iter().map(|t| t.index).collect();
        assert_eq!(indexes, vec![0, 1]);
    }

    #[test]
    fn the_chosen_audio_track_is_the_one_mapped() {
        let dir = Path::new("/tmp/session");
        let args = hls_args(Path::new("/m/a.mkv"), Delivery::Remux, 0.0, None, 1, dir);
        assert!(args.windows(2).any(|w| w == ["-map", "0:a:1?"]), "{args:?}");
        assert!(
            !args.windows(2).any(|w| w == ["-map", "0:a:0?"]),
            "only the chosen track: {args:?}"
        );
    }

    #[test]
    fn a_direct_stream_also_maps_the_chosen_audio_track() {
        let args = stream_args(Path::new("/m/a.mkv"), Delivery::Remux, 0.0, None, 2);
        assert!(args.windows(2).any(|w| w == ["-map", "0:a:2?"]), "{args:?}");
    }

    const MKV_PROBE: &str = r#"{
        "streams": [
            {"codec_type":"video","codec_name":"hevc"},
            {"codec_type":"audio","codec_name":"dts"},
            {"codec_type":"subtitle","codec_name":"subrip","tags":{"language":"ita","title":"Forzati"}},
            {"codec_type":"subtitle","codec_name":"hdmv_pgs_subtitle","tags":{"language":"eng"}}
        ],
        "format": {"format_name":"matroska,webm","duration":"7245.123"}
    }"#;

    #[test]
    fn a_probe_is_read_into_a_profile() {
        let profile = parse_probe(MKV_PROBE, Path::new("/m/a.mkv")).expect("probe");
        assert_eq!(profile.container, "matroska");
        assert_eq!(profile.video_codec.as_deref(), Some("hevc"));
        assert_eq!(profile.audio_codec.as_deref(), Some("dts"));
        assert_eq!(profile.duration, Some(7245.123));
        assert_eq!(profile.subtitles.len(), 2);
        assert_eq!(profile.subtitles[0].language.as_deref(), Some("ita"));
        assert_eq!(profile.subtitles[0].title.as_deref(), Some("Forzati"));
        assert!(profile.subtitles[0].textual);
        assert!(!profile.subtitles[1].textual, "PGS is a bitmap format");
    }

    #[test]
    fn subtitle_indexes_count_subtitle_streams_only() {
        // ffmpeg's `0:s:N` counts among subtitles, not among all streams: using
        // the absolute stream index would extract the wrong track.
        let profile = parse_probe(MKV_PROBE, Path::new("/m/a.mkv")).expect("probe");
        assert_eq!(profile.subtitles[0].index, 0);
        assert_eq!(profile.subtitles[1].index, 1);
    }

    #[test]
    fn an_embedded_cover_is_not_mistaken_for_the_film() {
        // A tagged MP4 carries its artwork as a stream of `codec_type: video`
        // marked `attached_pic`. Taken for the film it decides a full
        // conversion for an H.264 that would have played untouched, and the
        // mapping below puts a single still frame on screen for the whole
        // running time.
        let json = r#"{"streams":[
            {"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}},
            {"codec_type":"video","codec_name":"h264","disposition":{"attached_pic":0}},
            {"codec_type":"audio","codec_name":"aac"}],
            "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"1200.0"}}"#;
        let profile = parse_probe(json, Path::new("/m/a.mp4")).expect("probe");
        assert_eq!(profile.video_codec.as_deref(), Some("h264"));
    }

    #[test]
    fn a_file_whose_only_video_stream_is_a_cover_reports_it() {
        // Nothing else to fall back on: an audio file with artwork. Reporting
        // the cover is right — reporting nothing would call it a video file
        // with an unknown codec, which is a worse lie.
        let json = r#"{"streams":[
            {"codec_type":"video","codec_name":"png","disposition":{"attached_pic":1}},
            {"codec_type":"audio","codec_name":"flac"}],
            "format":{"format_name":"matroska","duration":"200.0"}}"#;
        let profile = parse_probe(json, Path::new("/m/a.mka")).expect("probe");
        assert_eq!(profile.video_codec.as_deref(), Some("png"));
    }

    #[test]
    fn the_mapping_skips_attached_pictures() {
        // `0:v:0` counts the cover; `0:V:0` is ffmpeg's "video streams, not
        // attached pictures". Without it the conversion re-encodes the artwork
        // and the film never appears.
        for args in [
            stream_args(Path::new("/m/a.mp4"), Delivery::Remux, 0.0, None, 0),
            hls_args(
                Path::new("/m/a.mp4"),
                Delivery::Transcode,
                0.0,
                None,
                0,
                Path::new("/tmp/s"),
            ),
        ] {
            assert!(
                args.contains(&"0:V:0?".to_string()),
                "video mapping should exclude cover art: {args:?}"
            );
            assert!(!args.contains(&"0:v:0?".to_string()));
        }
    }

    #[test]
    fn a_probe_without_streams_is_an_error_not_a_panic() {
        for json in ["{}", "null", "{\"streams\": 3}", "not json"] {
            assert!(
                parse_probe(json, Path::new("/m/a.mkv")).is_err(),
                "json: {json}"
            );
        }
    }

    #[test]
    fn a_missing_duration_is_absent_rather_than_zero() {
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"h264"}],
                       "format":{"format_name":"matroska","duration":"N/A"}}"#;
        assert_eq!(
            parse_probe(json, Path::new("/m/a.mkv"))
                .expect("probe")
                .duration,
            None
        );
    }

    #[test]
    fn a_container_list_falls_back_to_its_first_name() {
        // Nothing to match against — a file with no extension, or one the
        // demuxer does not list — still has to produce a container name.
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"h264"}],
                       "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"10"}}"#;
        for file in ["/m/a", "/m/a.bin"] {
            assert_eq!(
                parse_probe(json, Path::new(file)).expect("probe").container,
                "mov",
                "file: {file}"
            );
        }
    }

    #[test]
    fn a_webm_is_not_mistaken_for_a_matroska() {
        // ffprobe answers "matroska,webm" for both, because one demuxer reads
        // both. Taking the first name calls every WebM a Matroska, and a file
        // that plays untouched is then remuxed into MP4 — where WebKit will not
        // decode VP8, VP9 or Opus at all.
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"vp9"},
                                  {"codec_type":"audio","codec_name":"opus"}],
                       "format":{"format_name":"matroska,webm","duration":"10"}}"#;
        let webm = parse_probe(json, Path::new("/m/a.webm")).expect("probe");
        assert_eq!(webm.container, "webm");
        assert_eq!(
            webm.delivery(crate::core::media::Capabilities { hevc: false }),
            Delivery::Direct,
            "a WebM the element plays by itself must not go through ffmpeg"
        );

        let mkv = parse_probe(json, Path::new("/m/a.mkv")).expect("probe");
        assert_eq!(mkv.container, "matroska", "an MKV is still an MKV");
    }

    #[test]
    fn a_native_container_is_recognised_under_its_own_extension() {
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"h264"},
                                  {"codec_type":"audio","codec_name":"aac"}],
                       "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"10"}}"#;
        for (file, expected) in [
            ("/m/a.mp4", "mp4"),
            ("/m/a.mov", "mov"),
            ("/m/a.m4v", "mov"),
        ] {
            let profile = parse_probe(json, Path::new(file)).expect("probe");
            assert_eq!(profile.container, expected, "file: {file}");
            assert_eq!(
                profile.delivery(crate::core::media::Capabilities { hevc: false }),
                Delivery::Direct,
                "file: {file}"
            );
        }
    }

    #[test]
    fn an_extension_is_matched_whatever_its_case() {
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"vp9"}],
                       "format":{"format_name":"matroska,webm","duration":"10"}}"#;
        assert_eq!(
            parse_probe(json, Path::new("/m/A.WEBM"))
                .expect("probe")
                .container,
            "webm"
        );
    }

    /// Everything in a source file above its test module, with the line endings
    /// it was checked out with taken out of the question.
    ///
    /// A Windows checkout has CRLF, so a pattern written with `\n` matches
    /// nothing there and the whole file, tests included, reads as production.
    /// That is what the first CI run this project ever had on Windows found,
    /// and it found it because the caller asserts the separator was there,
    /// rather than trusting a `split` that answers the whole string when it
    /// matches nothing.
    fn above_the_tests(source: &str) -> String {
        let normalised = source.replace("\r\n", "\n");
        // The test module, not the first `#[cfg(test)]` in the file: that
        // attribute also sits on a constant halfway down `stream_server.rs`,
        // and splitting on it threw away everything below, including both spawn
        // sites in `handle`. The guard passed by looking at nothing.
        match normalised.split_once("#[cfg(test)]\nmod tests") {
            Some((above, _)) => above.to_string(),
            None => normalised,
        }
    }

    #[test]
    fn the_guard_below_reads_a_file_checked_out_on_windows_too() {
        // Same file twice, in the two shapes git hands out. Without the
        // normalisation the second one comes back whole, so the guard would
        // scan its own test module and pass on a spawn site added anywhere.
        let unix = "fn main() {}\n\n#[cfg(test)]\nmod tests {\n    Command::new(\"x\");\n}\n";
        let windows = unix.replace('\n', "\r\n");

        assert_eq!(above_the_tests(unix), "fn main() {}\n\n");
        assert_eq!(above_the_tests(&windows), "fn main() {}\n\n");
        assert!(!above_the_tests(&windows).contains("Command::new("));
    }

    #[test]
    fn nothing_this_app_runs_is_spawned_around_the_helper() {
        // The flag it carries is invisible to a test on any platform: it can
        // only be seen as a black window opening on a Windows machine, which no
        // suite here runs on. What decays is not the flag but the discipline,
        // so this guards the discipline. The four spawn sites in the playback
        // path had no console flag at all, and every one of them put a window
        // on top of the film.
        for (name, source) in [
            ("ffmpeg.rs", include_str!("ffmpeg.rs")),
            ("stream_server.rs", include_str!("stream_server.rs")),
            ("lib.rs", include_str!("../lib.rs")),
        ] {
            let production = above_the_tests(source);
            assert!(
                production.len() < source.replace("\r\n", "\n").len(),
                "{name} has no test module to stop at, so this \
                 read the tests as production and can only pass by luck"
            );
            // `command` itself is the one place that may, which is why it is
            // the only file this looks at below its own definition.
            let below_helper = if name == "ffmpeg.rs" {
                production
                    .split_once("Command::new(program)")
                    .map(|(_, rest)| rest)
            } else {
                Some(production.as_str())
            };
            assert!(
                !below_helper.unwrap_or_default().contains("Command::new("),
                "{name} spawns something without going through ffmpeg::command"
            );
        }
    }

    #[test]
    fn a_conversion_closes_a_segment_when_the_playlist_asks_for_one() {
        // `-hls_time 4` can only cut on a keyframe, and libx264's own default
        // puts one every 250 frames: ten seconds of video at 25fps, four at
        // 60fps and more. So the first segment, the one the player is waiting
        // on before anything at all appears, takes two to three times longer
        // to encode than it needs to. On macOS this never shows, because HEVC
        // is copied rather than converted there.
        let args = hls_args(
            Path::new("/m/a.mkv"),
            Delivery::Transcode,
            0.0,
            Some("hevc"),
            0,
            Path::new("/tmp/session"),
        );
        assert!(args.windows(2).any(|w| w == ["-g", "96"]), "{args:?}");
        assert!(
            args.windows(2).any(|w| w == ["-keyint_min", "96"]),
            "without a floor, x264 is free to place them further apart anyway: {args:?}"
        );
    }

    #[test]
    fn a_copied_stream_is_never_told_how_to_place_its_keyframes() {
        // `-g` on a `-c copy` is meaningless at best: there is no encoder to
        // instruct, and the segment boundaries come from the source.
        for delivery in [Delivery::Remux, Delivery::TranscodeAudio] {
            let args = hls_args(
                Path::new("/m/a.mkv"),
                delivery,
                0.0,
                Some("h264"),
                0,
                Path::new("/tmp/session"),
            );
            assert!(!args.iter().any(|a| a == "-g"), "{delivery:?}: {args:?}");
        }
    }

    #[test]
    fn a_conversion_always_lands_on_eight_bit_chroma() {
        // x265 releases are usually 10-bit. Without a pixel format libx264
        // keeps the depth and emits H.264 High 10, which no browser decodes —
        // and this is the path taken precisely because nothing else worked.
        let args = stream_args(
            Path::new("/m/a.mkv"),
            Delivery::Transcode,
            0.0,
            Some("hevc"),
            0,
        );
        assert!(
            args.windows(2).any(|w| w == ["-pix_fmt", "yuv420p"]),
            "{args:?}"
        );
    }

    #[test]
    fn copied_hevc_carries_the_tag_apple_platforms_require() {
        // ffmpeg tags HEVC in MP4 as `hev1`; WebKit only plays `hvc1`. macOS is
        // the only platform where HEVC is copied instead of converted, so
        // without the tag that whole optimisation never plays.
        for delivery in [Delivery::Direct, Delivery::Remux, Delivery::TranscodeAudio] {
            for codec in ["hevc", "H265"] {
                let args = stream_args(Path::new("/m/a.mkv"), delivery, 0.0, Some(codec), 0);
                assert!(
                    args.windows(2).any(|w| w == ["-tag:v", "hvc1"]),
                    "{delivery:?} {codec}: {args:?}"
                );
            }
        }
    }

    #[test]
    fn a_tag_is_never_forced_onto_a_stream_it_does_not_describe() {
        // The tag names the codec: putting `hvc1` on copied H.264 would produce
        // a file that describes itself wrongly.
        for codec in [Some("h264"), Some("vp9"), None] {
            let args = stream_args(Path::new("/m/a.mkv"), Delivery::Remux, 0.0, codec, 0);
            assert!(!args.iter().any(|a| a == "-tag:v"), "{codec:?}: {args:?}");
        }
        // Re-encoding produces H.264, whatever went in.
        let args = stream_args(
            Path::new("/m/a.mkv"),
            Delivery::Transcode,
            0.0,
            Some("hevc"),
            0,
        );
        assert!(!args.iter().any(|a| a == "-tag:v"), "{args:?}");
    }

    #[test]
    fn remuxing_copies_both_streams() {
        let args = stream_args(Path::new("/m/a.mkv"), Delivery::Remux, 0.0, Some("h264"), 0);
        assert!(args.windows(2).any(|w| w == ["-c", "copy"]), "{args:?}");
        assert!(!args.iter().any(|a| a == "libx264"));
        assert!(!args.iter().any(|a| a == "-ss"), "no seek means no -ss");
    }

    #[test]
    fn converting_audio_keeps_the_picture_untouched() {
        let args = stream_args(
            Path::new("/m/a.mkv"),
            Delivery::TranscodeAudio,
            0.0,
            Some("h264"),
            0,
        );
        assert!(args.windows(2).any(|w| w == ["-c:v", "copy"]), "{args:?}");
        assert!(args.windows(2).any(|w| w == ["-c:a", "aac"]), "{args:?}");
    }

    #[test]
    fn seeking_puts_ss_before_the_input() {
        let args = stream_args(
            Path::new("/m/a.mkv"),
            Delivery::Remux,
            631.5,
            Some("h264"),
            0,
        );
        let ss = args.iter().position(|a| a == "-ss").expect("-ss present");
        let input = args.iter().position(|a| a == "-i").expect("-i present");
        assert!(ss < input, "seeking after -i decodes everything first");
        assert_eq!(args[ss + 1], "631.500");
    }

    #[test]
    fn the_output_is_always_a_fragmented_mp4_on_stdout() {
        for delivery in [
            Delivery::Direct,
            Delivery::Remux,
            Delivery::TranscodeAudio,
            Delivery::Transcode,
        ] {
            let args = stream_args(Path::new("/m/a.mkv"), delivery, 0.0, Some("h264"), 0);
            assert_eq!(args.last().map(String::as_str), Some("pipe:1"));
            assert!(
                args.iter().any(|a| a.contains("frag_keyframe")),
                "delivery: {delivery:?}"
            );
        }
    }

    #[test]
    fn a_playlist_is_written_into_the_session_directory() {
        let dir = Path::new("/tmp/session");
        let args = hls_args(Path::new("/m/a.mkv"), Delivery::Remux, 0.0, None, 0, dir);
        // Spelled through `join`, not written out: Windows separates with `\`,
        // and a literal `/tmp/session/index.m3u8` fails there on the separator
        // while saying the playlist landed outside the session directory.
        let in_dir = |name: &str| dir.join(name).to_string_lossy().into_owned();
        assert_eq!(args.last(), Some(&in_dir("index.m3u8")), "{args:?}");
        assert!(
            args.windows(2)
                .any(|w| w == ["-hls_segment_filename", &in_dir("seg%05d.m4s")]),
            "{args:?}"
        );
    }

    #[test]
    fn the_init_segment_is_named_without_a_path() {
        // ffmpeg copies this string into the playlist as the URI of the
        // initialisation segment. A path would send the player looking for
        // `/tmp/...` on whatever machine is playing.
        let args = hls_args(
            Path::new("/m/a.mkv"),
            Delivery::Remux,
            0.0,
            None,
            0,
            Path::new("/tmp/session"),
        );
        let init = args
            .iter()
            .position(|a| a == "-hls_fmp4_init_filename")
            .expect("init filename");
        assert_eq!(args[init + 1], "init.mp4");
    }

    #[test]
    fn segments_are_fragmented_mp4_in_a_playlist_that_grows() {
        // TS segments cannot carry HEVC without re-encoding it, and a `vod`
        // playlist is only written once ffmpeg has finished the whole film —
        // which would mean waiting for the end before seeing the beginning.
        let args = hls_args(
            Path::new("/m/a.mkv"),
            Delivery::Remux,
            0.0,
            None,
            0,
            Path::new("/tmp/s"),
        );
        assert!(
            args.windows(2).any(|w| w == ["-hls_segment_type", "fmp4"]),
            "{args:?}"
        );
        assert!(
            args.windows(2)
                .any(|w| w == ["-hls_playlist_type", "event"]),
            "{args:?}"
        );
        assert!(args.windows(2).any(|w| w == ["-f", "hls"]), "{args:?}");
        assert!(args.windows(2).any(|w| w == ["-hls_time", "4"]), "{args:?}");
    }

    #[test]
    fn a_playlist_keeps_every_codec_decision_the_pipe_made() {
        // The wrapper changed, the reasons the codecs were chosen did not: a
        // 10-bit source still needs a pixel format, and copied HEVC still needs
        // the tag WebKit insists on.
        let dir = Path::new("/tmp/s");
        let converted = hls_args(
            Path::new("/m/a.mkv"),
            Delivery::Transcode,
            0.0,
            Some("hevc"),
            0,
            dir,
        );
        assert!(
            converted.windows(2).any(|w| w == ["-pix_fmt", "yuv420p"]),
            "{converted:?}"
        );
        assert!(!converted.iter().any(|a| a == "-tag:v"), "{converted:?}");

        let copied = hls_args(
            Path::new("/m/a.mkv"),
            Delivery::Remux,
            0.0,
            Some("hevc"),
            0,
            dir,
        );
        assert!(
            copied.windows(2).any(|w| w == ["-tag:v", "hvc1"]),
            "{copied:?}"
        );
        assert!(copied.windows(2).any(|w| w == ["-c", "copy"]), "{copied:?}");

        let audio = hls_args(
            Path::new("/m/a.mkv"),
            Delivery::TranscodeAudio,
            0.0,
            Some("h264"),
            0,
            dir,
        );
        assert!(audio.windows(2).any(|w| w == ["-c:v", "copy"]), "{audio:?}");
        assert!(audio.windows(2).any(|w| w == ["-c:a", "aac"]), "{audio:?}");
        assert!(audio.windows(2).any(|w| w == ["-ac", "2"]), "{audio:?}");
    }

    #[test]
    fn a_playlist_starting_mid_film_seeks_before_decoding() {
        let dir = Path::new("/tmp/s");
        let args = hls_args(Path::new("/m/a.mkv"), Delivery::Remux, 900.25, None, 0, dir);
        let ss = args.iter().position(|a| a == "-ss").expect("-ss present");
        let input = args.iter().position(|a| a == "-i").expect("-i present");
        assert!(ss < input, "seeking after -i decodes everything first");
        assert_eq!(args[ss + 1], "900.250");

        let from_start = hls_args(Path::new("/m/a.mkv"), Delivery::Remux, 0.0, None, 0, dir);
        assert!(!from_start.iter().any(|a| a == "-ss"), "{from_start:?}");
    }

    #[test]
    fn subtitles_are_addressed_among_subtitle_streams() {
        let args = subtitle_args(Path::new("/m/a.mkv"), 2, 0.0);
        assert!(args.windows(2).any(|w| w == ["-map", "0:s:2"]), "{args:?}");
        assert!(args.windows(2).any(|w| w == ["-f", "webvtt"]), "{args:?}");
        assert!(!args.iter().any(|a| a == "-ss"), "{args:?}");
    }

    #[test]
    fn a_track_for_a_restarted_stream_is_cut_to_the_same_offset() {
        // The element's clock restarts at zero when ffmpeg is asked for a stream
        // from `start`, but a WebVTT extracted from the whole file still carries
        // the film's own timeline: every cue would then be `start` seconds late.
        let args = subtitle_args(Path::new("/m/a.mkv"), 0, 631.5);
        let input = args.iter().position(|a| a == "-i").expect("-i present");
        let seeks: Vec<usize> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == "-ss")
            .map(|(at, _)| at)
            .collect();

        assert_eq!(
            seeks.len(),
            2,
            "one seek on each side of the input: {args:?}"
        );
        assert!(seeks[0] < input, "the cheap seek has to come before -i");
        assert!(seeks[1] > input, "the exact one has to come after it");
        assert_eq!(args[seeks[0] + 1], "631.500");
        assert_eq!(args[seeks[1] + 1], "631.500");
    }

    #[test]
    fn the_cues_of_a_restarted_stream_are_shifted_by_the_whole_offset() {
        // The bug this closes: `-ss` before `-i` alone lands on the nearest
        // subtitle packet and makes *that* zero, so a film whose first line is
        // at 60 s, seeked to 3600 s, came back shifted by 60 s — every cue an
        // hour early, and the lines from before the seek still in the file.
        // `-copyts` keeps the original timestamps and `-output_ts_offset` is
        // what actually moves them onto the stream's clock.
        let args = subtitle_args(Path::new("/m/a.mkv"), 0, 3600.0);
        assert!(args.iter().any(|a| a == "-copyts"), "{args:?}");
        assert!(
            args.windows(2)
                .any(|w| w == ["-output_ts_offset", "-3600.000"]),
            "the shift has to be the offset itself, negated: {args:?}"
        );
    }

    #[test]
    fn a_track_for_a_stream_that_starts_at_the_beginning_is_left_alone() {
        // Nothing to shift, and `-copyts` on its own would only be a way to get
        // it wrong.
        let args = subtitle_args(Path::new("/m/a.mkv"), 0, 0.0);
        for unwanted in ["-ss", "-copyts", "-output_ts_offset"] {
            assert!(!args.iter().any(|a| a == unwanted), "{unwanted}: {args:?}");
        }
    }

    fn tool_file_name() -> &'static str {
        if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        }
    }

    #[test]
    fn a_bundled_tool_wins_over_the_one_on_path() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        std::fs::write(dir.path().join(tool_file_name()), b"#!/bin/sh\n").expect("write");

        assert_eq!(
            tool_path(&[dir.path().to_path_buf()], "ffmpeg"),
            dir.path().join(tool_file_name())
        );
    }

    #[test]
    fn the_first_directory_holding_the_tool_wins() {
        let first = tempfile::TempDir::new().expect("tempdir");
        let second = tempfile::TempDir::new().expect("tempdir");
        std::fs::write(first.path().join(tool_file_name()), b"#!/bin/sh\n").expect("write");
        std::fs::write(second.path().join(tool_file_name()), b"#!/bin/sh\n").expect("write");

        assert_eq!(
            tool_path(
                &[first.path().to_path_buf(), second.path().to_path_buf()],
                "ffmpeg"
            ),
            first.path().join(tool_file_name())
        );
    }

    #[test]
    fn a_later_directory_is_used_when_the_first_has_nothing() {
        let empty = tempfile::TempDir::new().expect("tempdir");
        let holder = tempfile::TempDir::new().expect("tempdir");
        std::fs::write(holder.path().join(tool_file_name()), b"#!/bin/sh\n").expect("write");

        assert_eq!(
            tool_path(
                &[empty.path().to_path_buf(), holder.path().to_path_buf()],
                "ffmpeg"
            ),
            holder.path().join(tool_file_name())
        );
    }

    #[test]
    fn a_directory_holding_the_name_as_a_folder_is_not_taken_for_the_tool() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        std::fs::create_dir(dir.path().join(tool_file_name())).expect("mkdir");

        assert_eq!(
            tool_path(&[dir.path().to_path_buf()], "ffmpeg"),
            PathBuf::from("ffmpeg")
        );
    }

    #[test]
    fn without_a_bundled_tool_the_bare_name_is_used() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        assert_eq!(
            tool_path(&[dir.path().to_path_buf()], "ffmpeg"),
            PathBuf::from("ffmpeg")
        );
    }

    #[test]
    fn with_no_directories_at_all_the_bare_name_is_used() {
        assert_eq!(tool_path(&[], "ffprobe"), PathBuf::from("ffprobe"));
    }
}
