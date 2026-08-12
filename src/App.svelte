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
    openAuthorSite,
    stopStream,
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
  // Which tile is being opened right now, so the answer to the click appears
  // under the pointer instead of only as a line at the top of the window.
  let pending = $state(null);
  // Outlives Home, which is unmounted whenever a title is open.
  let query = $state("");

  async function startPlayback(relativePath, external = []) {
    if (busy) return;
    error = null;
    busy = true;
    try {
      // Probing costs a process launch, so it happens here rather than during
      // the scan: the delay lands on one file, not on opening the app.
      const source = await getPlaybackSource(relativePath, external);
      playing = { source, relativePath };
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  // Every way out of the player comes through here. Unmounting it stops the
  // hls.js side, but the ffmpeg feeding it belongs to the backend and keeps
  // converting the rest of the film into temp until something says otherwise.
  // Failing to stop it is not worth an error box: the next film, or closing the
  // window, ends it anyway.
  function stopPlayback() {
    playing = null;
    stopStream().catch(() => {});
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
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
      busy = false;
    }
  }

  async function open(content) {
    if (busy) return;
    error = null;
    busy = true;
    pending = content.id;
    try {
      detail = await getContent(content.id);
      cameFrom = content.id;
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
      pending = null;
    }
  }

  function back() {
    detail = null;
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
    <Player source={playing.source} relativePath={playing.relativePath} onexit={stopPlayback} />
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
    mediaRoot={library.media_root}
    onback={back}
    onplay={startPlayback}
  />
{:else if library}
  <Home {library} onopen={open} onreload={loadLibrary} focusId={cameFrom} {pending} bind:query />
{/if}

{#if !playing}
  <!-- Every screen but the film. It belongs on the empty ones as much as on a
       full library — a stick that has not been set up yet is exactly when
       somebody wonders what this is and who wrote it — and it has no business
       over a picture. -->
  <footer>
    Made by
    <button onclick={() => openAuthorSite().catch(() => {})}>Luis Enriquez</button>
    <span aria-hidden="true">·</span>
    <span class="site">luise.ac</span>
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
