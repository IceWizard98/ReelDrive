<script>
  import { tick } from "svelte";
  import {
    audioUrl,
    clock,
    fallbackUrl,
    isFullscreen,
    playbackFailure,
    seekUrl,
    setFullscreen,
    subtitleUrl,
  } from "./api.js";
  import Icon from "./Icon.svelte";

  let {
    source,
    relativePath,
    onexit,
    // Second of the film to open at. Not passed to the backend as a start
    // offset on purpose: asking for a stream that begins midway turns a file
    // the platform plays untouched into a repackaged one, so a direct file
    // would spawn an ffmpeg to resume something it could simply seek. Handled
    // here instead, through the seek path that already exists — which is also
    // the path that knows when a restart really is needed.
    startAt = 0,
    // Where the film has got to, for whoever wants to remember it. Reported
    // rather than saved here: this component knows the clock, not the stick.
    // Not `onprogress`: the element already has one of those, and it is about
    // how much has downloaded.
    //
    // It names its own file, and that is not redundant. The parting report goes
    // out as this component is destroyed, which on the way into the next
    // episode happens after the caller already holds the *new* file — and a
    // caller reading the path from its own state would file a finished
    // episode's position against an episode nobody has watched, marking it seen
    // and skipping it. That it currently survives is an accident of the order
    // Svelte tears things down in, which is not a thing to depend on.
    onposition = () => {},
    // The film reached its end, as opposed to any other way out of the player.
    // Separate from `onexit` because only one of the two can lead into the next
    // episode.
    onfinished = null,
  } = $props();

  let video = $state(null);
  let stage = $state(null);
  let visible = $state(true);
  let hideTimer;
  let menu = $state(null);
  let error = $state(null);

  // A converted film arrives as an HLS playlist that grows while ffmpeg writes
  // it, and it begins at the second the session was asked for: `offset` is
  // where that is in the real film. Seeking inside what has been produced is
  // the element's own job; past it, a new session has to start there.
  //
  // Seeded from the stream the backend actually built, not from zero. Today
  // nothing asks `playback_source` to start midway, so this is always zero on
  // arrival — but the field is handed over precisely so the player need not
  // assume its request was honoured verbatim, and ignoring it meant every
  // position this component reports would be wrong by the offset the day
  // anything did ask.
  // svelte-ignore state_referenced_locally
  let offset = $state(source.offset ?? 0);
  let currentTime = $state(0);
  let paused = $state(false);
  let volume = $state(1);
  let muted = $state(false);
  let speed = $state(1);
  let buffering = $state(true);
  let buffered = $state(0);
  let scrubbing = $state(false);
  let scrubValue = $state(0);
  let subtitle = $state(null);
  let fullscreen = $state(false);
  // One `src` per track, kept apart from `source.subtitles` because they are
  // re-cut every time the stream restarts: a track is on the clock of the
  // stream it accompanies, and a restarted stream has a new one.
  //
  // Seeded from `source` and then owned by this component, so it cannot be a
  // `$derived`: the four below are the same. A new film arrives as a new
  // player — `App` keys the component on the file — which is what makes
  // reading the prop once here correct.
  // svelte-ignore state_referenced_locally
  let trackUrls = $state(source.subtitles.map((track) => track.url));
  // Last track the user turned on, so the caption key can put back what they
  // chose rather than always falling to the first track.
  let lastSubtitle = 0;
  // How large the cues are drawn. A single number is a guess about someone
  // else's eyes and someone else's screen — the one that shipped was sixteen
  // pixels, and it was wrong for everybody. Three sizes, chosen where the
  // tracks are chosen.
  let cueSize = $state("medium");
  // Which track failed to load, if one did. Named rather than counted: a film
  // with four tracks and one broken one is a different problem.
  let trackFault = $state(null);
  const CUE_SIZES = [
    ["small", "Small"],
    ["medium", "Medium"],
    ["large", "Large"],
  ];
  // Set once the cheap delivery has been refused, so the retry happens once
  // rather than in a loop.
  let converted = $state(false);
  // How the stream currently playing is being delivered. It starts as what the
  // backend chose and changes when the fallback takes over, which the seek path
  // has to follow: a converted stream is no longer seekable by the element.
  // svelte-ignore state_referenced_locally
  let delivery = $state(source.delivery);
  // Which audio track is playing. Changing it rebuilds the stream: only one
  // track travels, because the element has no working way to choose between
  // several once they have arrived.
  // svelte-ignore state_referenced_locally
  let audioTrack = $state(source.audio_track ?? 0);
  // svelte-ignore state_referenced_locally
  const audioTracks = source.audio ?? [];
  // Tracks the file has and the app cannot show. Read once, like the rest: a
  // new film is a new player.
  // svelte-ignore state_referenced_locally
  const bitmapSubtitles = source.bitmap_subtitles ?? 0;

  const duration = $derived(source.duration ?? 0);
  const position = $derived(scrubbing ? scrubValue : offset + currentTime);
  const percent = (seconds) => (duration ? Math.min(100, (seconds / duration) * 100) : 0);

  // WebKit — the engine of this window on macOS — plays HLS itself. Chromium on
  // Windows and webkit2gtk on Linux do not, and there the playlist has to be
  // fed to the element through Media Source Extensions instead.
  const NATIVE_HLS =
    document.createElement("video").canPlayType("application/vnd.apple.mpegurl") !== "";

  // The same words the backend and the README use for the four deliveries, so
  // the caption under the spinner and the badge in the corner agree.
  const WAITING = {
    direct: "Loading",
    remux: "Repackaging",
    "transcode-audio": "Converting the audio",
    transcode: "Converting",
  };

  // The badge in the corner says how the film is reaching the window, and it
  // used to print the backend's own word for it: "transcode-audio" over a film.
  // Same four deliveries, in the past tense, because by then it has happened.
  const MODE = {
    remux: "Repackaged",
    "transcode-audio": "Audio converted",
    transcode: "Converted",
  };

  // Nine shortcuts nobody could find out about. `?` is where every player of
  // this kind keeps its list, and Escape closes it like any other overlay.
  let shortcuts = $state(false);
  const SHORTCUTS = [
    ["Space  ·  K", "Play or pause"],
    ["← →", "5 seconds"],
    ["J  ·  L", "10 seconds"],
    ["↑ ↓", "Volume"],
    ["M", "Mute"],
    ["C", "Subtitles"],
    ["F", "Full screen"],
    ["Esc", "Back to library"],
  ];

  // Whether the wait has gone on long enough to be worth naming. The delay is
  // the decision — a word that flashes up for 200ms between two frames of a
  // file that was never going to stall is noise — so it lives here rather than
  // as an `animation-delay`, which the reduced-motion rule zeroes.
  let slowWait = $state(false);
  $effect(() => {
    if (!buffering) {
      slowWait = false;
      return;
    }
    const timer = setTimeout(() => (slowWait = true), 1000);
    return () => clearTimeout(timer);
  });

  const isPlaylist = (url) => url.endsWith(".m3u8");
  let hls = null;
  // Numbered for the same reason seeks are: the import below is an await, and
  // whatever happens during it — a second attach, or leaving the player — has
  // already destroyed the instance this call is about to replace. Building one
  // afterwards attaches it to the element with nothing left holding a reference
  // to destroy it, and it goes on pulling segments for a film nobody is watching.
  let attachToken = 0;

  /// Point the element at `url`, whichever of the two ways this engine needs.
  ///
  /// hls.js is imported only when it is actually used: on macOS the browser
  /// does this natively and the library is never fetched at all.
  async function attach(url) {
    const token = ++attachToken;
    hls?.destroy();
    hls = null;
    if (!isPlaylist(url) || NATIVE_HLS) {
      video.src = url;
      video.load();
      return;
    }
    const { default: Hls } = await import("hls.js");
    if (token !== attachToken || !video) return;
    if (!Hls.isSupported()) {
      // Neither native nor MSE. Handing the URL over anyway lets the element
      // report the failure itself, which is what starts the conversion retry.
      video.src = url;
      video.load();
      return;
    }
    hls = new Hls();
    // A stalled segment recovers by itself; only a fatal error means the
    // picture is not coming, and that is the same dead end as a refused file.
    hls.on(Hls.Events.ERROR, (_event, data) => {
      if (data.fatal) onPlaybackError();
    });
    hls.loadSource(url);
    hls.attachMedia(video);
  }

  $effect(() => {
    if (video) attach(source.url);
  });

  // Resuming happens once, on the first stream this player attaches — never
  // again. `seekTo` is reached through the element's own metadata event because
  // a direct file cannot be seeked before it knows how long it is; a converted
  // one is already positioned by its offset and needs nothing. Guarded by a
  // flag rather than by comparing positions: after the seek the film is at
  // `startAt`, and anything that compared the two would resume again on the
  // next `loadedmetadata` — which every stream restart fires.
  // Read once, like the four pieces of state seeded from `source` above and for
  // the same reason: a new film arrives as a new player, so where to resume it
  // is settled at creation and never changes under this component.
  // svelte-ignore state_referenced_locally
  let resumed = startAt <= 0;
  function resumeOnce() {
    if (resumed) return;
    resumed = true;
    // Landing within a second of the end would start a film on its credits and
    // immediately end it, which with autoplay on is an episode skipped.
    if (duration && startAt >= duration - 1) return;
    if (startAt > offset) seekTo(startAt);
  }

  /// The film's position, for whoever is remembering it.
  ///
  /// `position` is `offset + currentTime`: a restarted stream's element clock
  /// runs from where that stream began, not from where the film does, so the
  /// element's own `currentTime` is the wrong number to hand out.
  ///
  /// Sent on pause and on the way out, and otherwise no more than once every
  /// twenty seconds — `timeupdate` fires four times a second, and each of these
  /// is a write to a stick.
  let lastReport = 0;
  const REPORT_EVERY = 20;
  function report(force = false) {
    // A film still waiting for its first frame reports zero, which as a
    // remembered position means "start again from the beginning".
    if (!duration || buffering) return;
    if (!force && Math.abs(position - lastReport) < REPORT_EVERY) return;
    lastReport = position;
    onposition(relativePath, position, duration);
  }

  // The last word on where the film got to, whichever way the player was left.
  //
  // One place rather than one call beside every exit: Escape, the back button,
  // the error screen's own button and the end of the film all finish as this
  // component being unmounted, and a position saved on three of the four routes
  // is a feature that loses your place depending on how you left.
  $effect(() => () => report(true));

  /// Whether the element can reach `target` on its own.
  ///
  /// An untouched file is the whole film with byte ranges behind it, so
  /// anywhere in it counts. A playlist is the film from `offset` onwards and
  /// grows as ffmpeg writes: everything it has already produced is seekable
  /// too, which is what `seekable` reports. Only a target beyond that — or
  /// before this session began — costs a new conversion.
  function seeksItself(target) {
    if (delivery === "direct") return true;
    if (!video || target < offset) return false;
    const ranges = video.seekable;
    if (!ranges?.length) return false;
    return target - offset <= ranges.end(ranges.length - 1);
  }

  function show() {
    visible = true;
    clearTimeout(hideTimer);
    // Controls that vanish over a paused picture — or over a picture that has
    // not arrived yet — leave the user with a black frame and no way back
    // except moving the mouse to find out there is one.
    //
    // Keyboard focus is the fifth reason. The bar fades to opacity 0 with its
    // buttons still in the tab order, so a user who had tabbed to one was left
    // with an invisible focus ring on an invisible control: pressing a key
    // brought the bar back, but they could no longer see where they were.
    if (!menu && !paused && !error && !buffering && !focusOnControl()) {
      hideTimer = setTimeout(() => (visible = false), 2800);
    }
  }

  /// Whether the keyboard is currently on one of the controls. A click leaves
  /// focus on the button it pressed as well, so `:focus-visible` is what
  /// decides: pausing with the mouse must not pin the bar open for the rest of
  /// the film.
  function focusOnControl() {
    const active = document.activeElement;
    return !!active?.matches?.(":focus-visible") && !!stage?.contains(active);
  }

  // The bar starts visible, and every one of the four reasons to hold it up can
  // begin *after* the timer was armed: a converted stream buffers for seconds
  // before its first frame, and the bar used to disappear over it.
  $effect(() => {
    show();
  });

  // Leaving mid-nudge otherwise left a timer to ask for a stream nobody is
  // going to watch.
  $effect(() => () => {
    clearTimeout(hideTimer);
    clearTimeout(nudgeTimer);
    // Armed by every resize, and a resize is exactly what leaving full screen
    // produces: without this the query still went out afterwards, to answer a
    // question about a player that no longer exists.
    clearTimeout(syncTimer);
    // Left running, it keeps pulling segments from a session the backend is
    // about to replace. Bumping the token first disowns an attach still waiting
    // on its import, which would otherwise build an instance after this ran.
    attachToken += 1;
    hls?.destroy();
    hls = null;
  });

  function toggleMenu(name) {
    menu = menu === name ? null : name;
    show();
  }

  /// Move the highlight through the open menu, wrapping at both ends.
  ///
  /// Focus starts on the button that opened the list, which is not in the list,
  /// so the first press has to land on the track that is currently on rather
  /// than always on the first one — otherwise turning subtitles off and back on
  /// walks past the choice already made.
  function stepMenu(direction) {
    const items = [...(stage?.querySelectorAll(".picker ul button") ?? [])];
    if (items.length === 0) return;
    const from = items.indexOf(document.activeElement);
    if (from === -1) {
      const current = items.findIndex((item) => item.classList.contains("current"));
      return items[current === -1 ? 0 : current].focus();
    }
    items[(from + direction + items.length) % items.length].focus();
  }

  // Clicking the picture used to change nothing anyone could see until the frame
  // stopped moving — on a dark or static shot that is no feedback at all. Each
  // toggle stamps the symbol of what was just done in the middle of the picture
  // and lets it fade. Counted rather than flagged, so two taps in a row replay
  // it instead of the second one landing on an animation already running.
  let pulse = $state(0);
  let pulseIcon = $state("play");

  function togglePause() {
    if (!video) return;
    const resuming = video.paused;
    if (resuming) video.play();
    else video.pause();
    pulseIcon = resuming ? "play" : "pause";
    pulse += 1;
    show();
  }

  /// Clicking the picture is how everyone pauses, and it used to do nothing:
  /// the <video> covered the stage and swallowed the click. Deciding here from
  /// the target keeps the control rows from pausing the film when a click lands
  /// between their buttons.
  function onStageClick(event) {
    // A click on a menu, or on the button that opens one, belongs to the menu.
    // Everything else closes it — which is the gesture people already use.
    if (event.target.closest?.(".picker")) return;
    if (menu) return (menu = null);
    if (event.target === video || event.target === stage) togglePause();
  }

  /// Ask what state the window is actually in.
  ///
  /// It can leave full screen without going through the button — the green
  /// traffic light, ⌃⌘F, another application taking the space — and a stale
  /// flag here is not only a wrong icon: Escape reads it to decide whether it
  /// leaves the screen or leaves the film. Resize is the event both routes
  /// produce; the timer is because a manual drag produces hundreds of them and
  /// each answer costs a round trip.
  let syncTimer;
  function syncFullscreen() {
    clearTimeout(syncTimer);
    syncTimer = setTimeout(async () => (fullscreen = await isFullscreen()), 150);
  }

  // And once at the start, because the window can already be full screen when a
  // film opens: nothing brings the desktop back on the way out of the last one,
  // so leaving by the back button puts the library on a full-screen window.
  // Starting the next film at `false` was the same lie the resize handler
  // exists to prevent — a button offering "Full screen" from a full screen, and
  // an Escape that closes the film with the desktop still gone.
  $effect(() => {
    syncFullscreen();
  });

  async function toggleFullscreen() {
    show();
    // Taken from the answer, not assumed: a request can be refused, and a
    // button that flips its own icon on a refusal lies about where you are.
    fullscreen = await setFullscreen(!fullscreen, stage);
  }

  /// Re-cut every track to the offset the next stream begins at.
  ///
  /// The DOM has to carry the new `src` values *before* `load()` runs, or the
  /// element re-fetches the tracks it already had and the cues stay on the
  /// film's clock while the picture is on the stream's — which is a subtitle
  /// that never appears again after the first seek.
  async function retimeSubtitles(at, token) {
    if (source.subtitles.length === 0) return;
    try {
      const fresh = await Promise.all(
        source.subtitles.map((track) => subtitleUrl(track.path, track.track, at)),
      );
      // One IPC round trip per track, and they come back in whatever order the
      // backend answers — not the order they went out. A request already
      // superseded must not write its tracks over the newer one's: cues cut for
      // the second it was abandoned at, under a picture that starts somewhere
      // else, is a subtitle that never lines up again.
      if (token !== seekToken) return;
      trackUrls = fresh;
      await tick();
    } catch {
      // A track that cannot be re-cut is not a reason to lose the picture: the
      // film restarts with the subtitles it had, which is what happened before
      // any of them were re-cut at all.
    }
  }

  /// Point the element at a new stream that begins at `at`.
  ///
  /// `load()` resets the element, and that includes the two things the user
  /// chose by hand: playbackRate goes back to 1 and every text track back to
  /// disabled. Without putting them back, a seek left the bar reading 1.5×
  /// over a film playing at 1×, and turned the subtitles off without saying so.
  /// The tracks themselves are re-cut first, because `load()` is what fetches
  /// them. Both restart paths go through here so none of it can be forgotten.
  /// `token` is the seek this restart belongs to. The check before the call is
  /// not enough: there are two awaits in here, and the newest request may take
  /// over during either of them. Whoever finishes last wins the element, so a
  /// superseded restart has to stop rather than point the element at a stream
  /// the user has already moved away from.
  async function restart(url, at, token) {
    offset = at;
    currentTime = 0;
    buffered = 0;
    await retimeSubtitles(at, token);
    if (token !== seekToken) return;
    await attach(url);
    // The element is gone if the player was left during the await — leaving
    // does not bump the token, so this is the only thing that catches it.
    if (token !== seekToken || !video) return;
    video.playbackRate = speed;
    selectSubtitle(subtitle);
    try {
      await video.play();
    } catch {
      // A refused play() is not a failed film. Autoplay policy refuses one, and
      // a newer seek that reloads the element aborts the one before it; both
      // leave a stream sitting on its first frame, which the play button — and
      // the bar, which stays up because nothing is playing — can start. Turning
      // either into the fatal error box threw away a picture that was fine.
      // Taken from the element rather than assumed: no `pause` event fires for
      // a play that never started, so this is the only place that knows.
      buffering = false;
      paused = video.paused;
    }
  }

  // Every restart is numbered. Two of them can be in flight at once — a tap
  // that lands while ffmpeg is still starting the last one — and they come back
  // in whatever order ffmpeg answers. Only the newest request may touch
  // `offset`, or the film lands wherever the slower of the two started.
  let seekToken = 0;
  let nudgeTimer;

  async function seekTo(seconds) {
    const target = Math.max(0, Math.min(seconds, duration || seconds));
    // Grabbing the slider, or landing on a control, cancels a nudge still
    // waiting to go out: otherwise it fires afterwards and undoes this seek.
    clearTimeout(nudgeTimer);
    show();
    if (!video) return;

    if (seeksItself(target)) {
      // The element's clock runs from where this stream begins, not from where
      // the film does; for a direct file the two are the same.
      video.currentTime = target - offset;
      // Anchoring the reported position too keeps the bar from snapping back to
      // where it was for the frame before the first timeupdate.
      currentTime = target - offset;
      scrubbing = false;
      return;
    }

    // Restarting the stream: the element reloads, so the state it reports has
    // to be re-anchored to the new offset.
    buffering = true;
    error = null;
    const token = ++seekToken;
    try {
      const url = converted
        ? await fallbackUrl(relativePath, target, audioTrack)
        : await seekUrl(relativePath, target, delivery, source.video_codec, audioTrack);
      if (token !== seekToken) return;
      await restart(url, target, token);
    } catch (e) {
      // Only the backend refusing to build the stream reaches here, and only
      // the newest request may say so: a superseded one failing is expected.
      if (token === seekToken) error = String(e);
    } finally {
      // The anchor holds until the stream that replaces it is running, or until
      // a newer request has taken over the job of replacing it. Dropping it any
      // earlier let `position` fall back to where the old stream was, so the bar
      // snapped backwards mid-request.
      if (token === seekToken) scrubbing = false;
    }
  }

  // Each restarted stream costs an ffmpeg launch, so five taps of the arrow key
  // must not cost five of them: the taps accumulate on screen and one request
  // goes out when they stop. A seek the element can do itself has no such cost
  // and answers immediately.
  function nudge(seconds) {
    show();
    // `position` is already the anchor while scrubbing, so the taps accumulate
    // from the last one rather than from where the old stream still is.
    const target = Math.max(0, Math.min(position + seconds, duration || Infinity));
    if (seeksItself(target)) return seekTo(target);

    scrubbing = true;
    scrubValue = target;
    clearTimeout(nudgeTimer);
    nudgeTimer = setTimeout(() => seekTo(target), 400);
  }

  // Where the pointer is over the seek bar, in seconds of film. Without a
  // read-out the only way to find out where a click would land is to make it,
  // and on a converted stream landing wrong costs an ffmpeg restart.
  let hoverAt = $state(null);

  function onScrubMove(event) {
    const box = event.currentTarget.getBoundingClientRect();
    if (!duration || box.width === 0) return (hoverAt = null);
    const ratio = (event.clientX - box.left) / box.width;
    hoverAt = Math.min(duration, Math.max(0, ratio * duration));
  }

  function setVolume(value) {
    show();
    volume = value;
    muted = value === 0;
    if (video) {
      video.volume = value;
      video.muted = muted;
    }
  }

  function toggleMute() {
    show();
    muted = !muted;
    if (video) video.muted = muted;
  }

  function setSpeed(rate) {
    menu = null;
    show();
    speed = rate;
    if (video) video.playbackRate = rate;
  }

  /// Play the same second of the same film in another language.
  ///
  /// Numbered like a seek and for the same reason: this and a seek both replace
  /// the stream, and the one that comes back last must not win over the one
  /// asked for last. The position is read before the request goes out, because
  /// the film keeps playing while ffmpeg starts.
  async function setAudio(index) {
    menu = null;
    show();
    if (index === audioTrack || !video) return;

    const at = position;
    const token = ++seekToken;
    buffering = true;
    error = null;
    try {
      const stream = converted
        ? { url: await fallbackUrl(relativePath, at, index), delivery: "transcode", audio_track: index }
        : await audioUrl(relativePath, at, index);
      if (token !== seekToken) return;
      // Taken from the answer, not from the request: asking for a track the
      // file does not have comes back as the first one, and the bar has to say
      // what is playing rather than what was asked for.
      audioTrack = stream.audio_track ?? index;
      delivery = stream.delivery ?? delivery;
      await restart(stream.url, at, token);
    } catch (e) {
      if (token === seekToken) error = String(e);
    } finally {
      if (token === seekToken) scrubbing = false;
    }
  }

  function selectSubtitle(index) {
    // Choosing closes the list. Leaving it open cost the next click — it went to
    // dismissing the menu instead of the picture — and left the keyboard on a
    // button inside it, which is worse: see `claimed` below.
    menu = null;
    show();
    subtitle = index;
    if (index !== null) lastSubtitle = index;
    if (!video) return;
    for (let i = 0; i < video.textTracks.length; i += 1) {
      video.textTracks[i].mode = i === index ? "showing" : "disabled";
    }
  }

  // Subtitles are the control a personal library needs most: the files sit next
  // to the video and half the point of them is turning them on mid-scene.
  function toggleSubtitles() {
    if (source.subtitles.length === 0) return;
    selectSubtitle(subtitle === null ? Math.min(lastSubtitle, source.subtitles.length - 1) : null);
  }

  function onKey(event) {
    if (event.key === "Escape") {
      // The list is the innermost thing on screen, so it goes first — before
      // the menu, and long before leaving the film.
      if (shortcuts) {
        shortcuts = false;
        return show();
      }
      // Escape used to leave the film from inside an open menu, which is an
      // expensive way to close a list of subtitle tracks. Focus goes back to
      // the button that opened it: the list it was standing on is about to be
      // removed, and focus would otherwise fall to the document with nothing
      // to show for it.
      if (menu) {
        const opener = stage?.querySelector(".picker .label.open");
        menu = null;
        opener?.focus();
        return show();
      }
      // One layer at a time, and full screen is a layer: Escape brings the
      // window back before it leaves the film. Leaving a film by putting the
      // whole desktop back at the same moment is two things at once, and the
      // second one is not undoable.
      if (fullscreen) {
        toggleFullscreen();
        return;
      }
      return onexit();
    }
    show();

    // An open menu owns the arrows. Without this the window handler took them
    // and the film's volume moved instead of the highlight: three presses in an
    // open subtitle list dropped the sound to 85% and left the list untouched,
    // with no way to reach a track except Tab.
    if (menu && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
      event.preventDefault();
      return stepMenu(event.key === "ArrowDown" ? 1 : -1);
    }

    // A focused control owns the keys that operate it: Space presses the button
    // under it, the arrows step the slider under it. Without this the window
    // handler ate both — every button in the bar was dead to Space, and the
    // arrows on the volume slider seeked the film instead of moving the volume.
    // The seek slider is the exception: its native step is 0.1s, which is not a
    // useful distance in a film, so the arrows keep meaning the 5s nudge there.
    //
    // Only a button the *keyboard* is on, though — `:focus-visible`, the same
    // test the bar uses to decide whether to stay up. A click leaves focus on
    // the button it pressed, and without this that button went on eating Space
    // afterwards: pick a speed and Space no longer resumed the film, it pressed
    // that speed again, for ever, because the user never asked to be there.
    const target = event.target;
    const claimed =
      (target?.closest?.("button")?.matches?.(":focus-visible") &&
        (event.key === " " || event.key === "Enter")) ||
      (target?.matches?.('input[type="range"]:not(.seek)') && event.key.startsWith("Arrow"));
    if (claimed) return;

    const keys = {
      " ": togglePause,
      k: togglePause,
      ArrowRight: () => nudge(5),
      ArrowLeft: () => nudge(-5),
      ArrowUp: () => setVolume(Math.min(volume + 0.05, 1)),
      ArrowDown: () => setVolume(Math.max(volume - 0.05, 0)),
      m: toggleMute,
      j: () => nudge(-10),
      l: () => nudge(10),
      c: toggleSubtitles,
      f: toggleFullscreen,
      "?": () => (shortcuts = !shortcuts),
    };
    const handler = keys[event.key];
    if (handler) {
      event.preventDefault();
      handler();
    }
  }

  /// The picture never arrived. If the file was only rewrapped, the platform
  /// may simply not decode this codec after all — convert it and try once more
  /// from where we were, rather than showing a dead end.
  async function onPlaybackError() {
    if (converted || delivery === "transcode") {
      // The element knows only that it failed; it carries no reason and there
      // is none to read off it. ffmpeg's own complaint is the reason, and the
      // backend kept it — "Invalid data found when processing input" is
      // something a user can act on, "This file could not be played" is not.
      error = (await playbackFailure().catch(() => null)) || "This file could not be played.";
      show();
      return;
    }
    // A nudge still waiting to go out would fire after the fallback and
    // supersede it, abandoning the restart while leaving `converted` and the
    // new delivery behind: an ffmpeg launched for nothing and a second reload.
    // Same reason `seekTo` clears it.
    clearTimeout(nudgeTimer);
    converted = true;
    delivery = "transcode";
    buffering = true;
    error = null;
    // Numbered like a seek, and for the same reason: this restart must not be
    // overtaken by a seek that was asked for before the picture failed.
    const token = ++seekToken;
    try {
      const at = offset + currentTime;
      const url = await fallbackUrl(relativePath, at, audioTrack);
      if (token !== seekToken) return;
      await restart(url, at, token);
    } catch (e) {
      if (token === seekToken) error = String(e);
    } finally {
      // The anchor outlives the request that set it, exactly as in `seekTo`:
      // dropping it early lets the bar snap back to the old stream.
      if (token === seekToken) scrubbing = false;
    }
  }

  function onProgress() {
    const ranges = video?.buffered;
    if (!ranges?.length) return;
    buffered = offset + ranges.end(ranges.length - 1);
  }

  function onEnded() {
    // With a restarted stream the element ends at the end of *its* piece, which
    // is the end of the film only if it started at the beginning.
    if (delivery !== "direct" && offset + currentTime < duration - 1) return;
    // Told apart from every other way out, because this is the only one that
    // may lead into the next episode. Without a handler it is still an exit.
    (onfinished ?? onexit)();
  }
</script>

<svelte:window onkeydown={onKey} onresize={syncFullscreen} />
<svelte:document onfullscreenchange={syncFullscreen} />

<!-- Space and k do this from the keyboard; the click handler is the pointer's
     copy of that, not the only way in. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
  class="stage"
  class:idle={!visible}
  class:cue-small={cueSize === "small"}
  class:cue-large={cueSize === "large"}
  bind:this={stage}
  onmousemove={show}
  onclick={onStageClick}
  onfocusin={show}
  onfocusout={() => queueMicrotask(show)}
>
  <!-- svelte-ignore a11y_media_has_caption -->
  <!-- No `src` here: the source is attached from script, because a playlist
       needs hls.js on the engines that do not play HLS themselves. -->
  <video
    bind:this={video}
    autoplay
    crossorigin="anonymous"
    ontimeupdate={() => ((currentTime = video?.currentTime ?? 0), report())}
    onprogress={onProgress}
    onloadedmetadata={resumeOnce}
    onplay={() => ((paused = false), (buffering = false), show())}
    onpause={() => ((paused = true), show(), report(true))}
    onwaiting={() => (buffering = true)}
    onplaying={() => ((buffering = false), onProgress())}
    onended={onEnded}
    onerror={onPlaybackError}
  >
    {#each source.subtitles as track, index (index)}
      <!-- A track the element cannot fetch or parse fails in complete silence:
           it switches on in the menu and not one line ever appears, which is
           indistinguishable from a player whose subtitles are broken. -->
      <track
        kind="subtitles"
        src={trackUrls[index]}
        srclang={track.language ?? "und"}
        label={track.label}
        onerror={() => (trackFault = track.label)}
      />
    {/each}
  </video>

  <div class="scrim" class:hidden={!visible} aria-hidden="true"></div>

  <!-- The symbol of what the last click did, stamped over the picture and gone
       again. `pulse` is 0 until the first one, so nothing is drawn at load. -->
  {#key pulse}
    {#if pulse}
      <div class="pulse" aria-hidden="true"><Icon name={pulseIcon} size={34} /></div>
    {/if}
  {/key}

  <!-- A paused film is a still frame and nothing else: on a dark shot it is
       indistinguishable from one that failed to start. The button is the state,
       and it is also the way out of it. -->
  {#if paused && !buffering && !error}
    <button class="resume" onclick={togglePause} aria-label="Play">
      <Icon name="play" size={34} />
    </button>
  {/if}

  <!-- The element stays in the DOM whatever happens: a live region that appears
       at the same moment as its text announces nothing. Only the backing comes
       and goes with the wait — left on, it was a dark blob in the middle of
       every frame of every film, for the whole running time. -->
  <div class="status" class:waiting={buffering && !error} aria-live="polite">
    {#if buffering && !error}
      <div class="spinner"></div>
      <!-- A bare spinner over a black frame says only "wait", and the four
           deliveries do not cost the same wait at all: a file served untouched
           is instant, a full conversion has ffmpeg encoding ahead of the
           picture and can hold the frame for tens of seconds. Naming which one
           is the difference between a slow app and a working one. -->
      {#if slowWait}
        <p class="waiting">{WAITING[delivery] ?? "Loading"}</p>
      {/if}
    {/if}
  </div>

  {#if shortcuts}
    <!-- Not a dialog: nothing in it is interactive, it takes no focus, and any
         key that dismisses it is already handled above. -->
    <div class="keys" role="note" aria-label="Keyboard shortcuts list">
      <dl>
        {#each SHORTCUTS as [key, meaning] (key)}
          <div>
            <dt>{key}</dt>
            <dd>{meaning}</dd>
          </div>
        {/each}
      </dl>
    </div>
  {/if}

  {#if error}
    <!-- `role="alert"`: the film dies, a sentence appears, and without this a
         screen reader is told nothing at all. The toast in App already has it. -->
    <div class="fault" role="alert">
      <p>{error}</p>
      <button onclick={onexit}>Back to library</button>
    </div>
  {/if}

  <!-- Where you are and how to leave, in the corner every other player puts
       them. The close button alone, at the far end of the bottom row, made
       leaving a thing you had to go looking for. -->
  <div class="top" class:hidden={!visible}>
    <button class="icon" onclick={onexit} aria-label="Back to library">
      <Icon name="back" size={22} />
    </button>
    <span class="title">{source.title}</span>
    <!-- The only sign the shortcuts exist. In the top bar, which is already
         the place that says where you are and how to leave. -->
    <button
      class="hint"
      onclick={() => (shortcuts = !shortcuts)}
      aria-label="Keyboard shortcuts"
      aria-expanded={shortcuts}>?</button
    >
    {#if delivery !== "direct"}<span class="mode">{MODE[delivery] ?? delivery}</span>{/if}
  </div>

  {#if trackFault}
    <p class="track-fault" role="status">
      “{trackFault}” could not be read.
      <button onclick={() => (trackFault = null)}>Dismiss</button>
    </p>
  {/if}

  <div class="bar" class:hidden={!visible}>
    <div class="scrub">
      <span class="time">{clock(position)}</span>
      <!-- The wrapper is what the read-out is positioned against, so it has to
           be exactly as wide as the track the pointer is moving over. -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="track" onmousemove={onScrubMove} onmouseleave={() => (hoverAt = null)}>
        {#if hoverAt !== null}
          <span class="preview" style="left: {percent(hoverAt)}%">{clock(hoverAt)}</span>
        {/if}
        <input
          class="seek"
          type="range"
          min="0"
          max={duration || 1}
          step="0.1"
          value={position}
          disabled={!duration}
          aria-label="Position"
          aria-valuetext={clock(position)}
          style="--played: {percent(position)}%; --buffered: {percent(
            Math.max(buffered, position),
          )}%"
          oninput={(e) => {
            scrubbing = true;
            scrubValue = Number(e.currentTarget.value);
            show();
          }}
          onchange={() => seekTo(scrubValue)}
        />
      </div>
      <span class="time right">{clock(duration)}</span>
    </div>

    <div class="row">
      <!-- The one control the row is actually for. It was the same square as the
           five beside it, so the eye had to read the glyphs to find it. -->
      <button class="icon primary" onclick={togglePause} aria-label={paused ? "Play" : "Pause"}>
        <Icon name={paused ? "play" : "pause"} size={22} />
      </button>
      <button class="icon" onclick={() => nudge(-10)} aria-label="Back 10 seconds">
        <Icon name="back10" />
      </button>
      <button class="icon" onclick={() => nudge(10)} aria-label="Forward 10 seconds">
        <Icon name="forward10" />
      </button>

      <div class="volume">
        <button class="icon" onclick={toggleMute} aria-label={muted ? "Unmute" : "Mute"}>
          <Icon name={muted || volume === 0 ? "mute" : "volume"} />
        </button>
        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          value={muted ? 0 : volume}
          aria-label="Volume"
          style="--played: {(muted ? 0 : volume) * 100}%; --buffered: {(muted ? 0 : volume) *
            100}%"
          oninput={(e) => setVolume(Number(e.currentTarget.value))}
        />
      </div>

      <span class="spacer"></span>

      {#if source.subtitles.length > 0 || bitmapSubtitles > 0}
        <div class="picker">
          <button
            class="label"
            class:open={menu === "subs"}
            class:on={subtitle !== null}
            onclick={() => toggleMenu("subs")}
            aria-haspopup="true"
            aria-expanded={menu === "subs"}
          >
            Subtitles
          </button>
          {#if menu === "subs"}
            <ul>
              <!-- A film whose only tracks are images used to offer no menu at
                   all, which reads as "this film has no subtitles" — and the
                   detail page had already said it did. Saying what they are is
                   the difference between a missing feature and a broken one. -->
              {#if bitmapSubtitles > 0}
                <li class="aside">
                  {bitmapSubtitles === 1 ? "1 track is" : `${bitmapSubtitles} tracks are`} pictures,
                  not text (PGS or VobSub), and cannot be shown. A .srt beside the file can.
                </li>
              {/if}
              <li>
                <button class:current={subtitle === null} onclick={() => selectSubtitle(null)}>
                  <span class="tick"><Icon name="check" size={13} /></span>Off
                </button>
              </li>
              {#each source.subtitles as track, index (index)}
                <li>
                  <button class:current={subtitle === index} onclick={() => selectSubtitle(index)}>
                    <span class="tick"><Icon name="check" size={13} /></span>{track.label}
                  </button>
                </li>
              {/each}
              <!-- Where the tracks are, because it is the same decision: a
                   caption you cannot read is not a caption that is on. -->
              {#if source.subtitles.length > 0}
                <li class="sizes" role="group" aria-label="Subtitle size">
                  {#each CUE_SIZES as [value, label] (value)}
                    <button
                      class="size"
                      class:current={cueSize === value}
                      aria-pressed={cueSize === value}
                      onclick={() => (cueSize = value)}>{label}</button
                    >
                  {/each}
                </li>
              {/if}
            </ul>
          {/if}
        </div>
      {/if}

      {#if audioTracks.length > 1}
        <div class="picker">
          <button
            class="label"
            class:open={menu === "audio"}
            onclick={() => toggleMenu("audio")}
            aria-haspopup="true"
            aria-expanded={menu === "audio"}
          >
            Audio
          </button>
          {#if menu === "audio"}
            <ul>
              {#each audioTracks as track (track.index)}
                <li>
                  <button
                    class:current={audioTrack === track.index}
                    onclick={() => setAudio(track.index)}
                  >
                    <span class="tick"><Icon name="check" size={13} /></span>{track.label}
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      {/if}

      <div class="picker">
        <button
          class="label"
          class:open={menu === "speed"}
          class:on={speed !== 1}
          onclick={() => toggleMenu("speed")}
          aria-haspopup="true"
          aria-expanded={menu === "speed"}
        >
          {speed === 1 ? "Speed" : `${speed}×`}
        </button>
        {#if menu === "speed"}
          <ul>
            {#each [0.5, 0.75, 1, 1.25, 1.5, 2] as rate (rate)}
              <li>
                <button class:current={speed === rate} onclick={() => setSpeed(rate)}>
                  <span class="tick"><Icon name="check" size={13} /></span>{rate}×
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </div>

      <button
        class="icon"
        onclick={toggleFullscreen}
        aria-label={fullscreen ? "Leave full screen" : "Full screen"}
      >
        <Icon name={fullscreen ? "exitFullscreen" : "fullscreen"} />
      </button>
    </div>
  </div>
</div>

<style>
  .stage {
    position: fixed;
    inset: 0;
    background: #000;
    display: flex;
    align-items: flex-end;
  }

  .stage.idle {
    cursor: none;
  }

  video {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    background: #000;
  }

  /* Both of these sit dead centre, on top of the picture and under nothing. */
  .pulse,
  .resume {
    position: absolute;
    top: 50%;
    left: 50%;
    display: grid;
    place-items: center;
    width: 5rem;
    height: 5rem;
    border-radius: 50%;
    color: var(--text);
    background: #0b0b0fa6;
    backdrop-filter: blur(6px);
    box-shadow: inset 0 0 0 1px #ffffff1f;
  }

  .pulse {
    /* Nobody clicks a symbol that is already leaving. */
    pointer-events: none;
    animation: pulse-out 0.5s var(--ease-out) forwards;
  }

  @keyframes pulse-out {
    from {
      transform: translate(-50%, -50%) scale(0.8);
      opacity: 1;
    }
    to {
      transform: translate(-50%, -50%) scale(1.35);
      opacity: 0;
    }
  }

  .resume {
    transform: translate(-50%, -50%);
    cursor: pointer;
    transition:
      transform var(--dur-fast) var(--ease-out),
      background-color var(--dur-fast) var(--ease-out);
    /* No `both`: the fill would hold the animated `transform` after the
       animation ends, and this element also has a hover that changes exactly
       that property. */
    animation: resume-in var(--dur) var(--ease-out);
  }

  .resume:hover {
    background: #14141ae6;
    transform: translate(-50%, -50%) scale(1.06);
  }

  @keyframes resume-in {
    from {
      opacity: 0;
      transform: translate(-50%, -50%) scale(0.85);
    }
  }

  .status {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    display: grid;
    justify-items: center;
    gap: 0.9rem;
    /* Nothing to wait for: no box, no padding, nothing to catch a click. The
       element is still here only so the live region below has somewhere to
       live between waits. */
    pointer-events: none;
  }

  /* The top and bottom rows sit on gradient scrims; the middle of the picture
     has none, so a spinner drawn in #ffffff26 and a grey caption disappear the
     moment the frame under them is bright — a stall over a daylight scene looks
     like nothing is happening at all. Radial rather than a panel: it darkens
     the frame without putting a box on top of it. Only while something is
     actually being waited for: painted unconditionally it was a dark halo in
     the middle of every film, from the first frame to the last. */
  .status.waiting {
    padding: var(--space-5xl) 4.5rem;
    background: radial-gradient(farthest-side at 50% 50%, #000000cc, #000000ad 45%, #00000000 100%);
  }

  /* The second of delay is in the script, not here: it is logic — the wait
     that needs explaining is the long one — and reduced-motion zeroes every
     delay in the stylesheet, which made the word flash up immediately for
     exactly the users who had asked for less movement. */
  .waiting {
    margin: 0;
    font-size: var(--t-meta);
    letter-spacing: 0.06em;
    color: var(--text-2);
    animation: waiting-in var(--dur-slow) both var(--ease-out);
  }

  @keyframes waiting-in {
    from {
      opacity: 0;
    }
  }

  .spinner {
    width: 2.5rem;
    height: 2.5rem;
    border: 2px solid #ffffff40;
    border-top-color: var(--on);
    border-radius: 50%;
    /* Keyframes live in app.css: a component renames its own, and the
       reduced-motion rule that keeps this turning has to name it. */
    animation: reel-spin 0.8s linear infinite;
  }

  /* The engine positions captions for its own controls, and these are not its
     controls: it has no idea the bar is there, so every time the bar comes up —
     which is every mouse move, every pause, every seek — the line of dialogue
     went behind it. Moved on the same duration as the bar's own fade, or the
     text jumps while the bar is still fading. */
  video::-webkit-media-text-track-container {
    transition: transform var(--dur-slow) var(--ease-out);
  }

  .stage:not(.idle) video::-webkit-media-text-track-container {
    /* Clears the control panel — 5.6rem of bar plus its 1.15rem inset — with a
       gap left over, rather than resting on it. */
    transform: translateY(-7.75rem);
  }

  /* Cue size has to follow the picture, not the stylesheet. `1rem` is sixteen
     pixels whatever the window does: on a full-screen film that is a caption
     about a third the size every other player draws, and small enough on a
     busy frame to be missed altogether. `vh` is the video's own height here —
     the stage is `position: fixed; inset: 0` — so this holds in a window and
     in full screen, which is the one place it matters most.
     A solid box rather than a shadow: it holds on any frame. `::cue` takes no
     padding — WebVTT allows a short list of properties and that is not on it —
     so the backing hugs the text, and the weight is what carries it. */
  video::cue {
    font-family: var(--ui);
    font-size: clamp(1.15rem, 3.6vh, 2.6rem);
    font-weight: 500;
    /* This is the only vertical padding a cue can have. `::cue` takes a short
       list of properties and `padding` is not on it, so the backing box is
       exactly the line box: at 1.3 the text touched the top and bottom edges of
       its own black box. The extra half-line is what turns it into a caption
       with room around it rather than text with a rectangle drawn on it. */
    line-height: 1.75;
    color: #fff;
    /* No background here. The engine stacks two boxes to draw a caption — the
       line box, and the cue box inside it — and painting a see-through colour
       on the inner one gave a caption in two tones that changed colour with the
       shot behind it: olive over a yellow frame, navy over a blue one, one
       layer at the edges and two in the middle. Probed one pseudo-element at a
       time in the browser rather than guessed: this is the inner box, and it
       has to be clear so there is only ever one layer. */
    background: transparent;
    text-shadow: none;
  }

  /* And this is the line box, which carries the whole thing: one flat opaque
     band, the same on every frame. It runs the width of the picture because
     that is what this element is — it cannot be shrunk to the text, `width` on
     it is ignored — and a broadcast caption bar is a fair thing for it to look
     like. */
  video::-webkit-media-text-track-display {
    background-color: #000000 !important;
  }

  /* Written out rather than through a custom property: `::cue` is styled
     inside the engine's own shadow tree, and whether a variable declared out
     here reaches in there is not something to bet the caption size on. */
  .stage.cue-small video::cue {
    font-size: clamp(1rem, 2.7vh, 1.9rem);
  }

  .stage.cue-large video::cue {
    font-size: clamp(1.4rem, 4.8vh, 3.4rem);
  }

  .keys {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    padding: var(--space-xl) 1.75rem;
    border-radius: var(--radius-lg);
    background: var(--glass);
    border: 1px solid var(--glass-line);
    backdrop-filter: blur(22px) saturate(150%);
    box-shadow: var(--lift);
    animation: menu-in var(--dur) var(--ease-out) both;
  }

  .keys dl {
    margin: 0;
    display: grid;
    gap: var(--space-sm) var(--space-xl);
  }

  .keys div {
    display: flex;
    align-items: baseline;
    gap: var(--space-xl);
    justify-content: space-between;
  }

  .keys dt {
    font-size: var(--t-meta);
    font-weight: 700;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }

  .keys dd {
    margin: 0;
    font-size: var(--t-meta);
    color: var(--text-2);
  }

  .hint {
    flex: none;
    width: 1.6rem;
    height: 1.6rem;
    border-radius: 50%;
    border: 1px solid var(--glass-line);
    color: var(--text-3);
    font-size: var(--t-meta);
    font-weight: 700;
    line-height: 1;
    transition:
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }

  .hint:hover {
    background: #ffffff1f;
    color: var(--text);
  }

  /* A failure needs a way out of it, not only a sentence about it. */
  .fault {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    display: grid;
    justify-items: center;
    gap: var(--space-lg);
    text-align: center;
    padding: 1.75rem 2.25rem;
    border-radius: var(--radius-lg);
    background: var(--glass);
    border: 1px solid var(--glass-line);
    backdrop-filter: blur(22px) saturate(150%);
    box-shadow: var(--lift);
  }

  .fault p {
    margin: 0;
    font-size: var(--t-body);
    color: #ff9a9a;
  }

  .fault button {
    background: var(--text);
    color: var(--ink);
    padding: var(--space-sm) 1.25rem;
    border-radius: var(--radius);
    font-size: var(--t-body);
    font-weight: 700;
  }

  /* The two rows used to be gradients painted straight onto the frame, which
     is what a player looked like in 2013. They are surfaces now: a panel that
     sits *over* the picture with its own edge, its own blur and its own
     shadow, inset from the window rather than welded to it. The scrim stays
     underneath, faint, because a white glyph over a snow scene needs it. */
  .top {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: 0.55rem 0.7rem 0.55rem 0.5rem;
    margin: var(--space-md) var(--gutter) 0;
    border-radius: var(--radius-pill);
    background: var(--glass);
    border: 1px solid var(--glass-line);
    box-shadow: var(--lift);
    backdrop-filter: blur(22px) saturate(150%);
    transition: opacity var(--dur-slow) var(--ease-out);
  }

  .top .title {
    /* A flex item will not shrink below its content by default, so a long title
       pushed the badge beside it off the right edge instead of ellipsing. */
    min-width: 0;
    flex: 1;
    font-size: var(--t-body);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .bar {
    position: relative;
    width: auto;
    /* As a flex item of the stage its automatic minimum is its content width,
       so a narrow window pushed the whole bar past the right edge instead of
       letting the row below wrap. */
    min-width: 0;
    flex: 1;
    margin: 0 var(--gutter) 1.15rem;
    padding: 0.7rem 0.85rem 0.5rem;
    border-radius: var(--radius-lg);
    background: var(--glass);
    border: 1px solid var(--glass-line);
    box-shadow: var(--lift);
    backdrop-filter: blur(22px) saturate(150%);
    transition:
      opacity var(--dur-slow) var(--ease-out),
      transform var(--dur-slow) var(--ease-out);
    display: grid;
    gap: 0.15rem;
  }

  /* The panels float, so they need their own footing on a bright frame: two
     scrims, faint enough not to read as further panels and dark enough that a
     white glyph over a snow scene holds at 3:1. A real element rather than a
     pseudo one, so it can sit above the picture and below the controls. */
  .scrim {
    position: absolute;
    inset: 0;
    pointer-events: none;
    background:
      linear-gradient(#00000073, transparent 22%),
      linear-gradient(transparent 62%, #00000073);
    transition: opacity var(--dur-slow) var(--ease-out);
  }

  .scrim.hidden {
    opacity: 0;
  }

  .top.hidden,
  .bar.hidden {
    opacity: 0;
    pointer-events: none;
  }

  /* Grid items get the same automatic minimum as flex items, so both rows need
     it cleared before either can shrink inside the bar. */
  .scrub,
  .row {
    min-width: 0;
  }

  .scrub {
    display: flex;
    align-items: center;
    gap: 0.7rem;
    padding: 0 0.2rem;
  }

  .track {
    position: relative;
    display: flex;
    flex: 1;
    /* Same reason: a range input's intrinsic width is about 130px and would
       otherwise hold the row open. */
    min-width: 0;
  }

  .scrub .seek {
    flex: 1;
    min-width: 0;
  }

  /* The second the pointer is over, above the point it is over and centred on
     it. At the two ends it overhangs the track and sits over the clocks either
     side, which is where the room is. */
  .preview {
    position: absolute;
    bottom: calc(100% + 0.15rem);
    transform: translateX(-50%);
    padding: var(--space-3xs) var(--space-xs);
    border-radius: var(--radius-sm);
    background: #101015f2;
    border: 1px solid var(--line);
    font-size: var(--t-eyebrow);
    font-variant-numeric: tabular-nums;
    color: var(--text);
    pointer-events: none;
    white-space: nowrap;
  }

  .time {
    font-size: var(--t-meta);
    font-variant-numeric: tabular-nums;
    color: var(--text-2);
    min-width: 5ch;
  }

  .time.right {
    text-align: right;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    /* Below roughly 330px the row is wider than the window; wrapping keeps the
       last control reachable instead of clipped by body { overflow-x: hidden }. */
    flex-wrap: wrap;
  }

  .spacer {
    flex: 1;
  }

  /* Round, not square. A row of rounded squares reads as a toolbar; a row of
     circles reads as a transport, which is what it is. */
  .icon {
    width: 2.3rem;
    height: 2.3rem;
    border-radius: 50%;
    display: grid;
    place-items: center;
    color: var(--text-2);
    transition:
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out),
      transform var(--dur-fast) var(--ease-out);
  }

  .icon:hover {
    background: #ffffff1f;
    color: var(--text);
  }

  .icon:active {
    transform: scale(0.94);
  }

  /* Play/pause reads first in the row by being wider and filled, rather than by
     a hue. There are no hues left in here: white on black is the one control
     the eye should find without looking. */
  .icon.primary {
    width: 2.85rem;
    height: 2.85rem;
    margin-right: 0.35rem;
    background: var(--text);
    color: var(--ink);
  }

  .icon.primary:hover {
    background: #ffffff;
    color: var(--ink);
  }

  .volume {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
  }

  /* Out of the way until it is wanted: the row is for the picture, not for a
     slider that is set once and left alone.
     The width is fixed and only the paint changes. Animating `width` relaid the
     whole control row on every frame of the reveal, dragging the spacer and
     everything right of it along with it. The row is permanently 6rem wider,
     which it has the space for — it already wraps. */
  .volume input {
    width: 6rem;
    opacity: 0;
    transform: translateX(-6px);
    pointer-events: none;
    transition:
      opacity var(--dur) var(--ease-out),
      transform var(--dur) var(--ease-out);
  }

  .volume:hover input,
  .volume input:focus-visible {
    opacity: 1;
    transform: none;
    pointer-events: auto;
  }

  /* It sits halfway down the top scrim, where the frame shows through: on a
     daylit shot the third text tier and a hairline both fall under 3:1. Its own
     backing carries it wherever the picture goes. */
  /* Inside the panel now, so it no longer needs a backing of its own to
     survive a daylit frame — the panel is that backing. */
  .mode {
    flex: none;
    font-size: var(--t-eyebrow);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-3);
    border: 1px solid var(--glass-line);
    border-radius: var(--radius-pill);
    padding: var(--space-3xs) 0.5rem;
  }

  .picker {
    position: relative;
  }

  .label {
    padding: 0.45rem 0.85rem;
    border-radius: var(--radius-pill);
    font-size: var(--t-meta);
    font-weight: 600;
    color: var(--text-2);
    transition:
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }

  .label:hover,
  .label.open {
    background: #ffffff1f;
    color: var(--text);
  }

  /* A track that is on has to say so from the bar: opening the menu to find out
     is one action too many. */
  .label.on {
    color: var(--on);
  }

  .picker ul {
    position: absolute;
    bottom: calc(100% + 0.4rem);
    right: 0;
    margin: 0;
    padding: 0.3rem;
    list-style: none;
    min-width: 12rem;
    max-height: 15rem;
    overflow-y: auto;
    /* The same glass as the bar it grows out of. It used to be a flat opaque
       panel four pixels above a translucent one: two materials, one gesture,
       and the menu read as a window from another application. */
    background: var(--glass);
    border: 1px solid var(--glass-line);
    backdrop-filter: blur(22px) saturate(150%);
    border-radius: var(--radius-lg);
    box-shadow: 0 8px 24px #000000a6;
    /* It comes from the button that opened it rather than appearing on top of
       the film, which is the difference between a menu and a flash. */
    transform-origin: bottom right;
    animation: menu-in var(--dur-fast) var(--ease-out) both;
  }

  @keyframes menu-in {
    from {
      opacity: 0;
      transform: translateY(4px) scale(0.97);
    }
  }

  .picker li button {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    width: 100%;
    text-align: left;
    padding: var(--space-sm) 0.6rem;
    border-radius: var(--radius-sm);
    font-size: var(--t-meta);
    color: var(--text-2);
  }

  .picker li button:hover {
    background: #ffffff14;
    color: var(--text);
  }

  .picker li button.current {
    color: var(--on);
  }

  /* Above the panel, not inside it: it belongs to the film, it is rare, and it
     must not move the controls when it appears. */
  .track-fault {
    position: absolute;
    left: 50%;
    bottom: 7.5rem;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: var(--space-md);
    margin: 0;
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-pill);
    background: var(--glass);
    border: 1px solid var(--glass-line);
    backdrop-filter: blur(22px);
    font-size: var(--t-meta);
    color: #ff9a9a;
    white-space: nowrap;
  }

  .track-fault button {
    font-size: var(--t-eyebrow);
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-3);
  }

  .track-fault button:hover {
    color: var(--text);
  }

  /* Three buttons on one line rather than three more rows: it is one setting,
     and it must not push the tracks off the top of the list. */
  .sizes {
    display: flex;
    gap: 0.2rem;
    margin-top: 0.25rem;
    padding-top: 0.3rem;
    border-top: 1px solid var(--line-soft);
  }

  .picker li button.size {
    flex: 1;
    justify-content: center;
    padding: var(--space-xs) 0.3rem;
    font-size: var(--t-eyebrow);
  }

  .picker li button.size.current {
    background: #ffffff1f;
    color: var(--on);
  }

  /* Not a choice — a sentence explaining why the choices below are short. */
  .aside {
    padding: var(--space-sm) 0.6rem;
    max-width: 20rem;
    font-size: var(--t-eyebrow);
    line-height: 1.45;
    color: var(--text-3);
    border-bottom: 1px solid var(--line-soft);
    margin-bottom: 0.2rem;
  }

  /* The selection is a tick, not only a colour: an amber word among grey ones
     is invisible to anyone who cannot separate them. */
  .tick {
    visibility: hidden;
  }

  .picker li button.current .tick {
    visibility: visible;
  }

  input[type="range"] {
    appearance: none;
    background: transparent;
    height: 1rem;
    cursor: pointer;
  }

  /* Three zones in one track: played, loaded, and the rest. Without the first
     of them the bar was a grey line with a dot on it and the only way to read
     your position was the clock. */
  /* The played part is white, not amber. Amber means "you chose this" —
     everywhere in the app, from the season tabs to the ticked track — and the
     progress bar is not a choice, it is a read-out. It is also the largest
     thing on screen, so painting it in the accent made the player the one
     screen where the accent shouted instead of pointing. */
  input[type="range"]::-webkit-slider-runnable-track {
    height: 5px;
    border-radius: var(--radius-pill);
    background: linear-gradient(
      to right,
      #ffffff 0 var(--played),
      #ffffff59 var(--played) var(--buffered),
      #ffffff2e var(--buffered) 100%
    );
    transition: height var(--dur-fast) var(--ease-out);
  }

  input[type="range"]::-webkit-slider-thumb {
    appearance: none;
    width: 12px;
    height: 12px;
    margin-top: -4px;
    border-radius: 50%;
    background: #ffffff;
    transform: scale(0);
    transition: transform var(--dur-fast) var(--ease-out);
  }

  /* The thumb is the grab handle, so it appears when the pointer is near enough
     to grab it and stays out of the picture the rest of the time. */
  .scrub:hover .seek::-webkit-slider-thumb,
  input[type="range"]:focus-visible::-webkit-slider-thumb,
  .volume input::-webkit-slider-thumb {
    transform: scale(1);
  }

  .scrub:hover .seek::-webkit-slider-runnable-track,
  .seek:focus-visible::-webkit-slider-runnable-track {
    height: 9px;
  }

  .scrub:hover .seek::-webkit-slider-thumb,
  .seek:focus-visible::-webkit-slider-thumb {
    width: 17px;
    height: 17px;
    margin-top: -4px;
    box-shadow: 0 0 0 4px #ffffff26;
  }

  input[type="range"]:disabled {
    cursor: default;
    opacity: 0.4;
  }

  @media (max-width: 640px) {
    .time {
      min-width: 4ch;
    }

    .volume input {
      display: none;
    }
  }
</style>
