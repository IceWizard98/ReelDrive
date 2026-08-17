//! Deciding how a file reaches the player.
//!
//! The webview plays video with `<video>`, which can only handle what the host
//! system's codecs cover: MP4/WebM containers, H.264 and AAC everywhere, HEVC
//! on some platforms, and nothing else. A stick full of MKVs therefore needs
//! ffmpeg in front of it — but rarely a full re-encode. Copying the streams
//! into a fragmented MP4 is almost free, and only the parts the platform
//! genuinely cannot decode have to be converted.
//!
//! This module holds that decision and nothing else: no processes, no I/O.

use serde::{Deserialize, Serialize};

/// What a file contains, as reported by the prober.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaProfile {
    pub container: String,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    /// Seconds; absent when the file carries no duration.
    pub duration: Option<f64>,
    /// Subtitle tracks embedded in the file, in stream order.
    pub subtitles: Vec<SubtitleTrack>,
    /// Audio tracks embedded in the file, in stream order. Empty means nobody
    /// listed them; `audio_codec` is then the only thing known.
    pub audio: Vec<AudioTrack>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioTrack {
    /// Index among audio streams only — what ffmpeg's `0:a:N` expects.
    pub index: u32,
    pub language: Option<String>,
    pub title: Option<String>,
    pub codec: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleTrack {
    /// Index among subtitle streams only — what ffmpeg's `0:s:N` expects.
    pub index: u32,
    pub language: Option<String>,
    pub title: Option<String>,
    /// Text-based tracks can become WebVTT; bitmap ones (PGS, VobSub) cannot.
    pub textual: bool,
}

/// How the file should be delivered to the webview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Delivery {
    /// Hand the original file over untouched. Costs nothing and keeps native
    /// seeking, so it is always preferred.
    Direct,
    /// Rewrap the existing streams into fragmented MP4. No re-encoding: the
    /// picture and sound are bit-for-bit the originals.
    Remux,
    /// Convert the audio, copy the video. Audio is cheap to convert; this is
    /// the common case for MKVs carrying AC3 or DTS.
    TranscodeAudio,
    /// Convert both. The expensive path, used only when the platform cannot
    /// decode the video at all.
    Transcode,
}

/// The same decision, for a stream that has to begin at `start` seconds and
/// carry audio track `audio`.
///
/// `Direct` hands over the untouched file, and the file can honour neither: it
/// begins at its first byte, and it carries every audio track with nothing
/// downstream to pick one — the element plays whichever the container names
/// first. Everything else is built on demand, starts where it is told to and
/// carries the track it is told to, so only this one case moves — and Remux
/// costs milliseconds, which is what makes the swap free.
///
/// Both failures are silent, which is why the decision lives here rather than
/// in any one caller: nothing downstream can tell that the film started from
/// the beginning, or came back in the wrong language.
///
/// Reported delivery and delivered stream have to come from here together: the
/// player reads the word to decide whether the element can seek on its own, so
/// a stream that says `direct` and arrives as a playlist breaks the seek bar
/// just as thoroughly as the other way round.
pub fn delivery_at(delivery: Delivery, start: f64, audio: u32) -> Delivery {
    match delivery {
        Delivery::Direct if start > 0.0 || audio != 0 => Delivery::Remux,
        other => other,
    }
}

/// Containers the webview can open on its own.
const NATIVE_CONTAINERS: &[&str] = &["mp4", "m4v", "mov", "webm"];

/// Video codecs every target decodes.
const NATIVE_VIDEO: &[&str] = &["h264", "vp8", "vp9", "av1"];

/// What the webview on this machine can decode.
///
/// HEVC is the one that pays: WebKit on macOS decodes it in hardware, so an
/// x265 file only needs rewrapping instead of a full re-encode — the difference
/// between a few milliseconds and minutes of CPU for a feature-length film. On
/// Windows it depends on an optional codec extension and on Linux on the
/// GStreamer plugins installed, so neither can be assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub hevc: bool,
}

impl Capabilities {
    pub fn current() -> Self {
        Self {
            hevc: cfg!(target_os = "macos"),
        }
    }

    fn decodes_video(&self, codec: &str) -> bool {
        NATIVE_VIDEO.contains(&codec.to_lowercase().as_str()) || (self.hevc && is_hevc(codec))
    }
}

/// Whether the webview opens an HLS playlist by itself.
///
/// WKWebView does. WebView2 on Windows and webkit2gtk on Linux do not, and
/// there the playlist has to go through hls.js and Media Source Extensions.
///
/// It is the platform that answers, not the engine, because the engine cannot
/// be asked: Chromium answers "maybe" to
/// `canPlayType("application/vnd.apple.mpegurl")` whether or not that build can
/// open one. Reading that as a yes is what left every converted film on Windows
/// handing a playlist to an element that did nothing with it, and calling the
/// film unplayable.
///
/// Separate from `Capabilities` on purpose: that one is about what the machine
/// decodes, and is the same question for every file. This one is about how the
/// bytes get in, and is settled once for the whole platform.
pub fn native_hls() -> bool {
    cfg!(target_os = "macos")
}

/// HEVC under either of the names ffprobe reports.
pub fn is_hevc(codec: &str) -> bool {
    matches!(codec.to_lowercase().as_str(), "hevc" | "h265")
}

/// Audio codecs the webview can decode. AC3, DTS and TrueHD are absent because
/// browsers do not decode them, which is what makes so many MKVs silent.
const NATIVE_AUDIO: &[&str] = &["aac", "mp3", "opus", "vorbis", "flac"];

/// Subtitle codecs that can be turned into WebVTT. Bitmap formats cannot: they
/// are images, and would need burning into the picture.
const TEXT_SUBTITLES: &[&str] = &["subrip", "srt", "ass", "ssa", "webvtt", "mov_text", "text"];

pub fn is_textual_subtitle(codec: &str) -> bool {
    TEXT_SUBTITLES.contains(&codec.to_lowercase().as_str())
}

impl MediaProfile {
    /// The cheapest treatment that will actually play on this machine, for the
    /// first audio track.
    pub fn delivery(&self, capabilities: Capabilities) -> Delivery {
        self.delivery_for(0, capabilities)
    }

    /// The same decision for a chosen audio track, whose codec may differ from
    /// the first one's — a film often carries a native track the platform plays
    /// and a dubbed one it does not.
    pub fn delivery_for(&self, audio: u32, capabilities: Capabilities) -> Delivery {
        let video_ok = self
            .video_codec
            .as_deref()
            .is_none_or(|codec| capabilities.decodes_video(codec));
        let audio_ok = self
            .audio_codec_at(audio)
            .is_none_or(|codec| NATIVE_AUDIO.contains(&codec.to_lowercase().as_str()));
        // Handing the file over untouched leaves the choice of track to the
        // element, which has no way to make it: anything but the first track
        // has to go through ffmpeg to be the one that arrives.
        let container_ok =
            NATIVE_CONTAINERS.contains(&self.container.to_lowercase().as_str()) && audio == 0;

        match (video_ok, audio_ok, container_ok) {
            (false, _, _) => Delivery::Transcode,
            (true, false, _) => Delivery::TranscodeAudio,
            (true, true, false) => Delivery::Remux,
            (true, true, true) => Delivery::Direct,
        }
    }

    /// The chosen track if the file has it, otherwise the first.
    ///
    /// ffmpeg's `0:a:N?` maps nothing at all for an N that does not exist, and
    /// the result is a film with no sound and no error. Every path that takes a
    /// track number from outside goes through here first.
    pub fn audio_track_or_first(&self, index: u32) -> u32 {
        if self.audio.iter().any(|track| track.index == index) {
            index
        } else {
            0
        }
    }

    /// Codec of one audio track. An index nobody has answers for the first
    /// track, which is what ffmpeg's `0:a:N?` would fall back to playing.
    pub fn audio_codec_at(&self, index: u32) -> Option<&str> {
        self.audio
            .iter()
            .find(|track| track.index == index)
            .and_then(|track| track.codec.as_deref())
            .or(self.audio_codec.as_deref())
    }

    /// Subtitle tracks that can be offered to the player.
    pub fn usable_subtitles(&self) -> impl Iterator<Item = &SubtitleTrack> {
        self.subtitles.iter().filter(|track| track.textual)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A platform without HEVC — Windows and Linux, and the conservative
    /// default. The macOS case gets its own test.
    fn no_hevc() -> Capabilities {
        Capabilities { hevc: false }
    }

    fn profile(container: &str, video: &str, audio: &str) -> MediaProfile {
        MediaProfile {
            container: container.to_string(),
            video_codec: Some(video.to_string()),
            audio_codec: Some(audio.to_string()),
            duration: Some(1200.0),
            subtitles: Vec::new(),
            audio: Vec::new(),
        }
    }

    fn with_audio(profile: MediaProfile, tracks: &[(&str, &str)]) -> MediaProfile {
        MediaProfile {
            audio: tracks
                .iter()
                .enumerate()
                .map(|(index, (codec, language))| AudioTrack {
                    index: index as u32,
                    codec: Some((*codec).to_string()),
                    language: Some((*language).to_string()),
                    title: None,
                })
                .collect(),
            ..profile
        }
    }

    #[test]
    fn the_delivery_follows_the_audio_track_that_was_chosen() {
        // The first track plays as it is; the second needs converting. Deciding
        // on the first one would send silence, or a codec the webview drops.
        let film = with_audio(
            profile("matroska", "h264", "aac"),
            &[("aac", "eng"), ("dts", "ita")],
        );
        assert_eq!(film.delivery_for(0, no_hevc()), Delivery::Remux);
        assert_eq!(film.delivery_for(1, no_hevc()), Delivery::TranscodeAudio);
    }

    #[test]
    fn choosing_a_second_audio_track_rules_out_handing_the_file_over() {
        // Direct means the untouched file, where nothing selects a track: the
        // element would play the first one and the choice would be silently
        // ignored. Remux costs milliseconds and actually honours it.
        let film = with_audio(
            profile("mp4", "h264", "aac"),
            &[("aac", "eng"), ("aac", "ita")],
        );
        assert_eq!(film.delivery_for(0, no_hevc()), Delivery::Direct);
        assert_eq!(film.delivery_for(1, no_hevc()), Delivery::Remux);
    }

    #[test]
    fn a_chosen_audio_track_rules_out_handing_the_file_over() {
        // The same hole as the offset, on the other parameter. The untouched
        // file carries every track and nothing downstream picks one: the
        // element plays whichever the container names first. Being handed the
        // file after asking for the Italian track is a silent no-op — the sound
        // stays in the wrong language and nothing anywhere says so.
        assert_eq!(delivery_at(Delivery::Direct, 0.0, 1), Delivery::Remux);
        assert_eq!(delivery_at(Delivery::Direct, 0.0, 0), Delivery::Direct);
        // Both at once is still just one rebuild.
        assert_eq!(delivery_at(Delivery::Direct, 631.5, 2), Delivery::Remux);
        // The other three carry the track they are given.
        for delivery in [
            Delivery::Remux,
            Delivery::TranscodeAudio,
            Delivery::Transcode,
        ] {
            assert_eq!(delivery_at(delivery, 0.0, 3), delivery);
        }
    }

    #[test]
    fn an_offset_rules_out_handing_the_file_over() {
        // Direct is the untouched file, and a file always starts at its first
        // byte: nothing in it can honour "begin at 631 s". Anything that has to
        // start midway has to be rebuilt from there.
        assert_eq!(delivery_at(Delivery::Direct, 631.5, 0), Delivery::Remux);
        assert_eq!(delivery_at(Delivery::Direct, 0.0, 0), Delivery::Direct);
        // The other three already begin where they are told to.
        for delivery in [
            Delivery::Remux,
            Delivery::TranscodeAudio,
            Delivery::Transcode,
        ] {
            assert_eq!(delivery_at(delivery, 631.5, 0), delivery);
            assert_eq!(delivery_at(delivery, 0.0, 0), delivery);
        }
    }

    #[test]
    fn a_track_index_nobody_has_falls_back_to_the_first() {
        // Not a hypothetical: ffmpeg maps nothing for a track that is not
        // there, so an index out of range would play a silent film.
        let film = with_audio(profile("mp4", "h264", "aac"), &[("aac", "eng")]);
        assert_eq!(film.audio_track_or_first(9), 0);
        assert_eq!(film.audio_track_or_first(0), 0);
    }

    #[test]
    fn a_track_index_the_file_has_is_kept() {
        let film = with_audio(
            profile("mp4", "h264", "aac"),
            &[("aac", "eng"), ("aac", "ita")],
        );
        assert_eq!(film.audio_track_or_first(1), 1);
    }

    #[test]
    fn a_file_whose_tracks_were_never_listed_keeps_the_first_track() {
        // Nothing was listed, so nothing can be chosen: asking for anything
        // else has to come back to the one track ffmpeg will find.
        assert_eq!(profile("mp4", "h264", "aac").audio_track_or_first(3), 0);
    }

    #[test]
    fn a_file_whose_tracks_were_never_listed_still_decides_on_its_codec() {
        // `audio` is empty for anything built before the tracks were recorded;
        // the single codec is all there is to go on, and it is enough.
        assert_eq!(
            profile("matroska", "h264", "dts").delivery_for(0, no_hevc()),
            Delivery::TranscodeAudio
        );
    }

    #[test]
    fn a_plain_mp4_is_handed_over_untouched() {
        assert_eq!(
            profile("mp4", "h264", "aac").delivery(no_hevc()),
            Delivery::Direct
        );
        assert_eq!(
            profile("webm", "vp9", "opus").delivery(no_hevc()),
            Delivery::Direct
        );
    }

    #[test]
    fn an_mkv_with_playable_streams_only_needs_rewrapping() {
        assert_eq!(
            profile("matroska", "h264", "aac").delivery(no_hevc()),
            Delivery::Remux
        );
    }

    #[test]
    fn surround_audio_is_converted_but_the_picture_is_copied() {
        for audio in ["ac3", "eac3", "dts", "truehd"] {
            assert_eq!(
                profile("matroska", "h264", audio).delivery(no_hevc()),
                Delivery::TranscodeAudio,
                "audio: {audio}"
            );
        }
    }

    #[test]
    fn a_codec_the_platform_cannot_decode_forces_a_full_conversion() {
        assert_eq!(
            profile("matroska", "hevc", "aac").delivery(no_hevc()),
            Delivery::Transcode
        );
        assert_eq!(
            profile("avi", "mpeg4", "mp3").delivery(no_hevc()),
            Delivery::Transcode
        );
    }

    #[test]
    fn video_conversion_wins_over_audio_conversion() {
        // Both are wrong, and re-encoding the video covers the audio anyway.
        assert_eq!(
            profile("matroska", "hevc", "dts").delivery(no_hevc()),
            Delivery::Transcode
        );
    }

    #[test]
    fn codec_names_are_matched_regardless_of_case() {
        assert_eq!(
            profile("MP4", "H264", "AAC").delivery(no_hevc()),
            Delivery::Direct
        );
    }

    #[test]
    fn a_file_without_streams_is_not_treated_as_broken_here() {
        // Deciding it is unplayable belongs to whoever tries to open it; an
        // audio-only or video-only file is legitimate.
        let audio_only = MediaProfile {
            container: "mp4".to_string(),
            video_codec: None,
            audio_codec: Some("aac".to_string()),
            duration: Some(180.0),
            subtitles: Vec::new(),
            audio: Vec::new(),
        };
        assert_eq!(audio_only.delivery(no_hevc()), Delivery::Direct);
    }

    #[test]
    fn hevc_is_rewrapped_where_the_platform_decodes_it() {
        let file = profile("matroska", "hevc", "aac");
        assert_eq!(
            file.delivery(Capabilities { hevc: true }),
            Delivery::Remux,
            "re-encoding an x265 film the machine can play is minutes of wasted CPU"
        );
        assert_eq!(file.delivery(no_hevc()), Delivery::Transcode);
    }

    #[test]
    fn hevc_is_recognised_under_both_names() {
        let capabilities = Capabilities { hevc: true };
        assert_eq!(
            profile("mp4", "h265", "aac").delivery(capabilities),
            Delivery::Direct
        );
        assert_eq!(
            profile("mp4", "HEVC", "aac").delivery(capabilities),
            Delivery::Direct
        );
    }

    #[test]
    fn bitmap_subtitles_are_not_offered() {
        let profile = MediaProfile {
            subtitles: vec![
                SubtitleTrack {
                    index: 0,
                    language: Some("ita".into()),
                    title: None,
                    textual: true,
                },
                SubtitleTrack {
                    index: 1,
                    language: Some("eng".into()),
                    title: None,
                    textual: false,
                },
            ],
            ..profile("matroska", "h264", "aac")
        };

        let usable: Vec<u32> = profile.usable_subtitles().map(|t| t.index).collect();
        assert_eq!(usable, vec![0], "PGS and VobSub are images, not text");
    }

    #[test]
    fn subtitle_codecs_are_classified_by_name() {
        for codec in ["subrip", "ASS", "mov_text", "webvtt"] {
            assert!(is_textual_subtitle(codec), "codec: {codec}");
        }
        for codec in ["hdmv_pgs_subtitle", "dvd_subtitle", "dvb_subtitle"] {
            assert!(!is_textual_subtitle(codec), "codec: {codec}");
        }
    }
}
