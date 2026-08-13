<script>
  import "./app.css";
  import Home from "./lib/Home.svelte";
  import Detail from "./lib/Detail.svelte";
  import Player from "./lib/Player.svelte";
  import Icon from "./lib/Icon.svelte";
  import {
    getContent,
    getLibrary,
    getPlaybackSource,
    getProgress,
    getUpNext,
    openAuthorSite,
    recordProgress,
    stopStream,
    takeUp,
  } from "./lib/api.js";

  let library = $state(null);
  let detail = $state(null);
  let error = $state(null);
  let loading = $state(true);
  let playing = $state(null);
  // Reading a folder and probing a file both take a process launch, so the two
  // of them are the moments the app can look asleep. `busy` drives the one
  // progress line at the top of the window.
  let busy = $state(false);
  // Which tile the library should hand focus back to after a detail closes.
  let cameFrom = $state(null);
  let cameFromRow = $state("");
  // Which tile is being opened right now, so the answer to the click appears
  // under the pointer instead of only as a line at the top of the window.
  let pending = $state(null);
  // Outlives Home, which is unmounted whenever a title is open.
  let query = $state("");
  // Outlives Home for the same reason the query does.
  let kind = $state("all");
  // A press that opens nothing is a dead button, and this app is built for
  // machines where there may be no browser to open at all. Not the error toast:
  // that one is only rendered once a library exists, and on the screen that
  // reports a failed scan `error` is the page's own sentence — a browser that
  // did not open would rewrite it.
  let noBrowser = $state(false);
  // What has been watched, keyed by the file's path under the media folder.
  // Held here because both screens read it and the player writes it, and
  // because it outlives all three: `Home` is unmounted whenever a title is
  // open, and the player is unmounted between two episodes.
  let progress = $state({});
  // The episode the open title would carry on with, worked out by the backend
  // from the seasons and the history together. Refetched rather than derived
  // here so the rule for it exists once, in one language.
  let upNext = $state(null);
  // Which visit to the player is in progress. Bumped by every exit, so work
  // still on its way from one episode to the next can tell that the viewer has
  // left in the meantime. Not `$state`: nothing on screen reads it.
  let watching = 0;

  /// Keep the local copy in step with what the backend actually stored.
  ///
  /// The backend is the one that decides — a few seconds in is not a position —
  /// so a mark it refused must not be written here either, or the tiles would
  /// show progress the next start does not have.
  function remember(path, mark) {
    if (mark) progress = { ...progress, [path]: mark };
  }

  /// Report a position, letting a failure pass in silence.
  ///
  /// This runs every twenty seconds of every film. A stick that cannot be
  /// written to is a thing to keep watching through, not an error box on
  /// repeat over the picture.
  async function keepPosition(path, seconds, duration) {
    try {
      remember(path, await recordProgress(path, seconds, duration));
    } catch (e) {
      console.warn("position not saved", e);
    }
  }

  async function openCredit() {
    try {
      await openAuthorSite();
      noBrowser = false;
    } catch {
      noBrowser = true;
    }
  }

  async function startPlayback(
    relativePath,
    external = [],
    startAt = 0,
    // Carried explicitly so the second episode of an evening knows the title it
    // belongs to as surely as the first did.
    contentId = detail?.summary?.id ?? null,
  ) {
    if (busy) return null;
    error = null;
    busy = true;
    try {
      // Probing costs a process launch, so it happens here rather than during
      // the scan: the delay lands on one file, not on opening the app.
      const source = await getPlaybackSource(relativePath, external);
      // The title the file belongs to, kept so the end of an episode can ask
      // what follows it. Playback always begins on a detail page, so it is
      // always at hand — and it has to be caught here, because `detail` may be
      // gone by the time the film ends.
      playing = { source, relativePath, startAt, contentId };
      return source;
    } catch (e) {
      error = String(e);
      return null;
    } finally {
      busy = false;
    }
  }

  /// The film ended by itself. Mark it watched and, if there is one, play the
  /// next episode — across the end of a season, which is where a series is
  /// most likely to be abandoned for want of one press.
  async function finished() {
    const { relativePath, contentId, source } = playing;
    // Which player this is about. Working out what comes next is a deep scan of
    // a folder on a stick, and the probe after it is a process launch: there
    // are seconds in here, and Escape during them has to keep meaning Escape
    // rather than being answered by a player that reopens itself.
    const from = watching;
    const left = () => from !== watching;

    const length = source.duration ?? 0;
    await keepPosition(relativePath, length, length);

    let next = null;
    try {
      if (contentId) next = await getUpNext(contentId);
    } catch (e) {
      // Nothing to carry on with is not an error worth a box over the library:
      // the film that just ended, ended.
      console.warn("next episode not found", e);
    }

    // Already gone, and the exit they made did the stopping.
    if (left()) return;

    // The same file again means the backend does not consider it finished —
    // a film whose length could not be probed, so nothing could be recorded.
    // Starting it over would be an endless loop, which is worse than stopping.
    if (!next || next.file === relativePath) return stopPlayback();

    const started = await startPlayback(next.file, next.subtitles, next.seconds, contentId);
    // The same check again, because `startPlayback` is a probe long enough to
    // leave during — and this side of it there is a player on screen to take
    // back down.
    if (!started || left()) return stopPlayback();
    // Dated now, so the series does not drop out of "continue watching" for the
    // first fifteen seconds of the episode — the window in which nothing about
    // it has yet been worth recording.
    try {
      remember(next.file, await takeUp(next.file, started.duration ?? 0));
    } catch (e) {
      console.warn("next episode not noted", e);
    }
  }

  // Every way out of the player comes through here. Unmounting it stops the
  // hls.js side, but the ffmpeg feeding it belongs to the backend and keeps
  // converting the rest of the film into temp until something says otherwise.
  // Failing to stop it is not worth an error box: the next film, or closing the
  // window, ends it anyway.
  function stopPlayback() {
    playing = null;
    // Counted, not merely cleared: the end of an episode leads into the start
    // of the next one through several awaits, and the only way that chain can
    // tell it has been abandoned is that this happened while it was waiting.
    watching += 1;
    // Before the stop, not after it. The title behind the player is still open
    // and what it should offer has just changed — the episode that was playing
    // may now be finished, and the button has to point at the one after it.
    // Ordered first so that nothing thrown on the way to the backend can leave
    // the page offering to resume an episode that is over.
    refreshUpNext();
    stopStream().catch(() => {});
  }

  /// What the open title would play next. Silent on failure: the detail page
  /// falls back to the button it has always had.
  async function refreshUpNext() {
    const id = detail?.summary?.id;
    if (!id) return (upNext = null);
    try {
      upNext = await getUpNext(id);
    } catch (e) {
      console.warn("next episode not found", e);
      upNext = null;
    }
  }

  async function loadLibrary() {
    loading = true;
    busy = true;
    error = null;
    // A reload remounts Home; without this it would pull focus back to whatever
    // title was last opened, which is not where a reload should land you.
    cameFrom = null;
    try {
      library = await getLibrary();
      // Beside the library rather than per tile: it is one small map, and the
      // grid cannot draw a single bar until it has all of it. A stick that
      // cannot be read for history is still a stick full of films.
      progress = await getProgress().catch((e) => (console.warn("history not read", e), {}));
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
      busy = false;
    }
  }

  async function open(content, row = "") {
    if (busy) return;
    error = null;
    busy = true;
    pending = content.id;
    try {
      detail = await getContent(content.id);
      cameFrom = content.id;
      // Which section it was opened from: a title can be on screen twice, once
      // under "continue watching" and once in its own group, and coming back
      // has to land on the tile that was clicked.
      cameFromRow = row;
      await refreshUpNext();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
      pending = null;
    }
  }

  function back() {
    detail = null;
    upNext = null;
  }

  loadLibrary();
</script>

<svelte:window
  onkeydown={(event) => {
    // The player owns Escape while it is up. Otherwise Escape backs out of one
    // thing at a time: first the error, then the title.
    if (event.key !== "Escape" || playing) return;
    if (error) error = null;
    else if (detail) back();
  }}
/>

{#if busy}
  <!-- One line, always in the same place: a click that costs a process launch
       has to answer immediately, even if the answer is only "working".
       The word is inside the region rather than on it: a live region with a
       label and no content announces nothing, so the only feedback for an
       operation that takes seconds was silent to a screen reader. -->
  <div class="progress" role="status">
    <span class="sr-only">Working</span>
  </div>
{/if}

{#if playing}
  <!-- Keyed on the file, not just on `playing`: the player seeds four pieces of
       its own state from `source` at creation — the subtitle tracks, the
       delivery, the audio track and the track list — because the user then
       changes them by hand. Handing it a different film without a remount
       would attach the new stream while those four still described the last
       one. Today `stopPlayback` always unmounts it, so the key costs nothing;
       the day something plays the next episode straight through, it is the
       difference between a new film and a broken one. -->
  {#key playing.relativePath}
    <Player
      source={playing.source}
      relativePath={playing.relativePath}
      startAt={playing.startAt}
      onexit={stopPlayback}
      onfinished={finished}
      onposition={keepPosition}
    />
  {/key}
{:else if loading}
  <!-- A line of text in the top left of a black page was the whole screen for
       as long as the scan took, which on a slow stick with two hundred folders
       is several seconds. The shapes say what is coming; the sentence still
       says what is happening, and it is what a screen reader gets. -->
  <p class="notice" role="status">Reading the library…</p>
  <div class="skeleton" aria-hidden="true">
    {#each Array(12) as _, i}
      <span class="ghost" style="--ghost-delay: {i * 60}ms"></span>
    {/each}
  </div>
{:else if error && !library}
  <!-- A stick that was unplugged mid-scan, or a folder that moved: the one
       thing to do about it is try again, so the screen has to offer that. -->
  <div class="welcome">
    <h1>Could not read the library</h1>
    <p class="path">{error}</p>
    <button onclick={loadLibrary}>Retry</button>
  </div>
{:else if library && !library.media_root_exists}
  <div class="welcome">
    <h1>Content folder missing</h1>
    <p>
      Create a folder named <code>media</code> next to the executable and put
      one folder inside it for each movie or series.
    </p>
    <p class="path">Expected path: <code>{library.media_root}</code></p>
    <button onclick={loadLibrary}>Retry</button>
  </div>
{:else if detail}
  <Detail
    {detail}
    {progress}
    {upNext}
    mediaRoot={library.media_root}
    onback={back}
    onplay={startPlayback}
  />
{:else if library}
  <Home
    {library}
    {progress}
    onopen={open}
    onreload={loadLibrary}
    focusId={cameFrom}
    focusRow={cameFromRow}
    {pending}
    bind:query
    bind:kind
  />
{/if}

{#if !playing}
  <!-- Every screen but the film. It belongs on the empty ones as much as on a
       full library — a stick that has not been set up yet is exactly when
       somebody wonders what this is and who wrote it — and it has no business
       over a picture. -->
  <footer>
    Made by
    <button onclick={openCredit}>Luis Enriquez</button>
    <span aria-hidden="true">·</span>
    <span class="site">luise.ac</span>
    {#if noBrowser}
      <span class="failed" role="alert">— no browser opened; the address is there to type</span>
    {/if}
  </footer>
{/if}

{#if error && library && !playing}
  <!-- Over the page rather than above it: an error that reflows the grid moves
       the tile the user was about to click. -->
  <div class="toast" role="alert">
    <p>{error}</p>
    <button onclick={() => (error = null)} aria-label="Dismiss">
      <Icon name="close" size={16} />
    </button>
  </div>
{/if}

<style>
  .notice {
    padding: var(--space-3xl) var(--gutter);
    color: var(--text-2);
    font-size: var(--t-body);
  }

  .progress {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    height: 2px;
    z-index: 10;
    background: linear-gradient(90deg, transparent, var(--on), transparent);
    background-size: 40% 100%;
    background-repeat: no-repeat;
    /* Keyframes in app.css, not here: a Svelte component renames its own, and
       the reduced-motion rule that keeps this one alive has to name it. */
    animation: reel-sweep 1.1s var(--ease-in-out) infinite;
  }

  /* Same geometry as the real grid, so nothing moves when the titles land. */
  .skeleton {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(clamp(148px, 11vw, 200px), 1fr));
    gap: var(--space-2xl) var(--space-lg);
    padding: 0 var(--gutter);
  }

  .ghost {
    aspect-ratio: 2 / 3;
    border-radius: var(--radius-md);
    background: var(--ink-raised);
    box-shadow: inset 0 0 0 1px #ffffff14;
    animation: ghost-in var(--dur-slow) var(--ghost-delay, 0ms) both var(--ease-out);
  }

  @keyframes ghost-in {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  .toast {
    position: fixed;
    left: 50%;
    bottom: 1.5rem;
    transform: translateX(-50%);
    z-index: 10;
    display: flex;
    align-items: center;
    gap: var(--space-md);
    max-width: min(60ch, calc(100vw - 2rem));
    padding: 0.6rem 0.6rem 0.6rem var(--space-lg);
    border-radius: var(--radius);
    background: var(--ink-raised);
    border: 1px solid var(--line-strong);
    box-shadow: 0 8px 24px #000000a6;
    /* The one element in the app that appears where the pointer is not: with
       no entrance it reads as a glitch rather than as a message. Rare enough
       (errors only) that the "no motion on frequent actions" rule does not
       apply. */
    animation: toast-in var(--dur) var(--ease-out) both;
  }

  @keyframes toast-in {
    from {
      opacity: 0;
      transform: translate(-50%, 8px);
    }
    to {
      opacity: 1;
      transform: translate(-50%, 0);
    }
  }

  .toast p {
    margin: 0;
    font-size: var(--t-body);
    color: #ff9a9a;
  }

  .toast button {
    display: grid;
    place-items: center;
    width: 1.75rem;
    height: 1.75rem;
    border-radius: var(--radius-sm);
    color: var(--text-3);
  }

  .toast button:hover {
    background: var(--ink-hi);
    color: var(--text);
  }

  /* In the flow, not fixed: pinned to the window it would sit over the last row
     of posters for the whole length of the library. */
  footer {
    display: flex;
    align-items: baseline;
    justify-content: center;
    gap: var(--space-sm);
    padding: var(--space-3xl) var(--gutter) var(--space-2xl);
    font-size: var(--t-meta);
    color: var(--text-3);
  }

  footer button {
    padding: 0;
    font-weight: 600;
    color: var(--text-2);
    text-decoration: underline;
    text-underline-offset: 3px;
    text-decoration-color: var(--line-strong);
    transition: color var(--dur-fast) var(--ease-out);
  }

  footer button:hover {
    color: var(--on);
    text-decoration-color: currentColor;
  }

  /* Shown as well as linked. The app is built for machines with no network, so
     the browser may not open — and an address you can read is one you can type
     somewhere else. */
  .site {
    font-variant-numeric: tabular-nums;
  }

  /* Dimmer than the address it points at: the sentence is the explanation, the
     address is the thing to act on. */
  .failed {
    color: var(--text-3);
  }

  .welcome {
    max-width: 52ch;
    margin: 0 auto;
    padding: clamp(var(--space-4xl), 12vh, 7rem) var(--gutter);
    line-height: 1.65;
    font-size: var(--t-body);
    color: var(--text-2);
  }

  .welcome h1 {
    margin: 0 0 var(--space-lg);
    font-size: var(--t-display);
    font-weight: 800;
    letter-spacing: -0.03em;
    line-height: 1.05;
    color: var(--text);
  }

  .path {
    color: var(--text-3);
    font-size: var(--t-meta);
    word-break: break-all;
  }

  code {
    color: var(--text);
    background: var(--ink-raised);
    border: 1px solid var(--line);
    padding: var(--space-3xs) var(--space-xs);
    border-radius: var(--radius-sm);
    font-size: 0.9em;
  }

  .welcome button {
    margin-top: 1.25rem;
    background: var(--text);
    color: var(--ink);
    padding: 0.65rem var(--space-xl);
    border-radius: var(--radius);
    font-weight: 700;
    font-size: var(--t-body);
  }

</style>
