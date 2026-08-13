import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

/// Whether the window is filling the screen.
///
/// Not `element.requestFullscreen()`. The webview this app runs in on macOS
/// refuses that for an ordinary element — it rejects, the player caught the
/// rejection and did nothing with it, and the button was dead. The *window* is
/// the thing that can go full screen here, and the capability for it was
/// already granted in `capabilities/default.json`; it had simply never been
/// wired to anything.
///
/// The document API stays as the fallback, for the preview harness: that runs
/// in an ordinary browser with no Tauri behind it, where the reverse is true.
export async function isFullscreen() {
  try {
    return await getCurrentWindow().isFullscreen();
  } catch {
    return !!document.fullscreenElement;
  }
}

/// Fill the screen, or stop. Returns the state it ended in, which is the only
/// reliable thing to believe: a request can be refused.
export async function setFullscreen(on, element) {
  try {
    await getCurrentWindow().setFullscreen(on);
  } catch {
    try {
      if (on) await element?.requestFullscreen();
      else if (document.fullscreenElement) await document.exitFullscreen();
    } catch {
      // A window manager that refuses is not a playback failure: the picture
      // keeps running at the size it already has.
    }
  }
  return isFullscreen();
}

export const getLibrary = () => invoke("library");
export const getContent = (id) => invoke("content", { id });

/// Everything needed to start a file: stream URL, how it is being delivered,
/// duration and the subtitle tracks that can be shown. `external` are the
/// subtitle files the scanner found next to the video — the backend cannot
/// rediscover them from the video path alone.
export const getPlaybackSource = (path, external = []) =>
  invoke("playback_source", { path, external });

/// A new stream carrying a different audio track, from `seconds` in. Which
/// treatment the file needs depends on the track chosen, so the backend decides
/// it again rather than being told: a second language is often in a codec the
/// first one is not.
export const audioUrl = (path, seconds, audio) =>
  invoke("audio_url", { path, seconds, audio });

/// A new stream starting at `seconds`; converted streams carry no index, so
/// seeking means asking for a different one.
export const seekUrl = (path, seconds, delivery, videoCodec = null, audio = 0) =>
  invoke("seek_url", { path, seconds, delivery, videoCodec, audio });

/// The same subtitle track, cut to the offset a restarted stream begins at.
/// A track carries the film's timeline; the element's clock restarts at zero
/// every time ffmpeg is asked for a stream from `seconds` in, so the two have to
/// be asked for together or the cues are `seconds` late.
export const subtitleUrl = (path, track, seconds) =>
  invoke("subtitle_url", { path, track, seconds });

/// Same file, converted outright. Used when the webview refuses the cheap
/// version — the HEVC guess is the one that can be wrong.
export const fallbackUrl = (path, seconds, audio = 0) =>
  invoke("fallback_url", { path, seconds, audio });

/// Leaving a film ends the conversion behind it. Nothing else does: the ffmpeg
/// runs until another film starts or the window closes, writing the whole of a
/// film nobody is watching into the temp folder.
export const stopStream = () => invoke("stop_playback");

/// What ffmpeg said when it gave up, if it did. A media element reports a
/// failure with no reason attached, so this is the only way the error box can
/// say anything more useful than "it broke".
export const playbackFailure = () => invoke("playback_failure");

/// Everything that has been watched, as a map of media-relative path to
/// `{ seconds, duration, done, at }`. Fetched once beside the library: it is a
/// few hundred short rows at most, and the alternative is a round trip per tile.
export const getProgress = () => invoke("progress");

/// Remember where a file got to. Answers with the mark that was stored, or
/// `null` when the backend decided the position was not worth keeping — a few
/// seconds in is not a position, and that rule lives in one place.
export const recordProgress = (path, seconds, duration) =>
  invoke("record_progress", { path, seconds, duration });

/// Put a file at the head of the history without claiming any of it was
/// watched. What the end of an episode does to the one after it, so a series
/// does not leave "continue watching" during the first seconds of every episode.
export const takeUp = (path, duration) => invoke("take_up", { path, duration });

/// The episode to play now for one title: the first one not finished, seasons
/// walked in order, so the end of a season leads into the next one. `null` once
/// everything has been watched.
export const getUpNext = (id) => invoke("up_next", { id });

/// Open the author's site in the user's own browser. The address lives in the
/// backend and is not a parameter: a command that takes a URL and hands it to
/// the shell opens whatever it is asked to.
export const openAuthorSite = () => invoke("open_author_site");

/// Relative paths from the backend always use "/", so the media root is
/// normalised to match before joining. Windows accepts forward slashes too.
export function fileUrl(mediaRoot, relativePath) {
  if (!relativePath) return null;
  const root = mediaRoot.replace(/\\/g, "/").replace(/\/+$/, "");
  return convertFileSrc(`${root}/${relativePath}`);
}

/// Stable colour per title, so a cover-less content always looks the same.
export function placeholderStyle(title) {
  let hash = 0;
  for (const char of title) hash = (hash * 31 + char.codePointAt(0)) % 360;
  return `--tile-hue: ${hash}`;
}

/// Seconds as a clock: `4:07`, `1:23:45`. Minutes are padded only under an
/// hour's worth, so a film reads `1:05:00` and an episode `5:00`.
///
/// Here rather than in the player because the detail page prints a resume
/// position with it, and two spellings of the same time on two screens of the
/// same app is the kind of difference nobody can explain afterwards.
export function clock(seconds) {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const whole = Math.floor(seconds);
  const h = Math.floor(whole / 3600);
  const m = Math.floor((whole % 3600) / 60);
  const s = whole % 60;
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
  return `${h > 0 ? `${h}:` : ""}${mm}:${String(s).padStart(2, "0")}`;
}

export function initials(title) {
  return title
    .split(/\s+/)
    .filter((word) => /[\p{L}\p{N}]/u.test(word))
    .slice(0, 2)
    .map((word) => word[0].toUpperCase())
    .join("");
}
