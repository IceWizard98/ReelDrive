<script module>
  // The staggered entrance belongs to opening the app, not to every return from
  // a title. App unmounts Home while a title is open, so each press of Library
  // rebuilt the grid and replayed the whole thing: measured at 1280px, the last
  // tile sat at opacity 0 for 200ms and did not finish arriving until 700ms
  // after the click — an empty shelf where the user had just been looking, and
  // a moving target for the focus ring Home puts back on the tile they left.
  //
  // Module scope survives the unmount, which is exactly the lifetime wanted:
  // once per run of the app. Every tile of the first grid reads it during init,
  // before any effect has run, so they all still animate together.
  let entered = false;
</script>

<script>
  import { fileUrl, initials, placeholderStyle } from "./api.js";

  let {
    content,
    mediaRoot,
    onopen,
    index = 0,
    pending = false,
    // How much of what is being watched here has been watched, 0 to 1, or
    // `null` for a title nobody has opened. For a series this is the progress
    // of the episode that would be resumed, not of the series — see `Home`.
    watched = null,
    // What that progress refers to, in words, for anyone who cannot see a bar.
    watchedLabel = "",
    // Which section of the grid this tile is in. A title can appear twice —
    // once under "Continue watching" and once in its own group — and the two
    // copies have to be told apart by the focus that is put back after a title
    // closes, or coming out of a series always lands on the top row.
    row = "",
  } = $props();

  const animate = !entered;
  $effect(() => {
    entered = true;
  });

  // A missing cover is normal — fall back to a tile whose colour is derived
  // from the title, so the same content always looks the same.
  let src = $derived(fileUrl(mediaRoot, content.cover));
  let failed = $state(false);
</script>

<!-- `aria-busy` rather than `disabled`: the tile keeps its place in the tab
     order and the focus the arrow keys put on it, and the click is already
     dropped upstream. -->
<button
  class="tile"
  class:enter={animate}
  class:pending
  aria-busy={pending}
  data-id={content.id}
  data-row={row}
  style="--enter-delay: {Math.min(index, 8) * 28}ms"
  onclick={() => onopen(content, row)}
>
  <span class="frame">
    {#if src && !failed}
      <img class="art" {src} alt="" loading="lazy" decoding="async" onerror={() => (failed = true)} />
    {:else}
      <span class="art art-generated" style={placeholderStyle(content.title)}>
        <span>{initials(content.title)}</span>
      </span>
    {/if}
    <!-- On the poster rather than under it: the caption rows are shared tracks
         across the whole grid row, so a bar down there would open a fourth
         track on every tile in the row to serve the one that had it. -->
    {#if watched !== null}
      <span class="watched" style="--watched: {Math.max(watched, 0.02) * 100}%"></span>
    {/if}
  </span>
  {#if watchedLabel}
    <!-- The bar is the whole of the information for anyone looking at it, and
         none of it for anyone who is not. -->
    <span class="sr-only">{watchedLabel}</span>
  {/if}

  <span class="title">{content.title}</span>
  <span class="year">{content.year ?? ""}</span>
</button>

<style>
  /* Three rows of the parent grid — poster, title, year — so every poster in a
     row is exactly as tall as its neighbours, every title starts on the same
     line and every year sits on the same baseline. Without this the sub-pixel
     width of `1fr` columns leaks into the height and the captions drift apart
     by a few pixels.
     The year used to align by having the title reserve two lines whether it
     needed them or not, which parked it 20px below every one-line title in the
     library. Giving it a track of its own means the reserve is the tallest
     title *in that row*, and in most rows that is one line. */
  .tile {
    display: grid;
    grid-row: span 3;
    grid-template-rows: subgrid;
    gap: var(--space-sm);
    padding: 0;
    text-align: left;
    /* Cleared past the sticky header, so a tile focused by the arrow keys does
       not scroll to a position underneath it. */
    scroll-margin: 6rem 0 var(--space-2xl);
    transition: opacity var(--dur) var(--ease-out);
  }

  /* 450ms plus up to 420ms of stagger left the grid still moving almost a
     second after the app opened. 260ms and eight steps of 28ms settle it in
     under half of that, with the stagger still wide enough to read as a
     sequence rather than as one block. */
  .tile.enter {
    animation: tile-in var(--dur-slow) var(--enter-delay, 0ms) both var(--ease-out);
  }

  /* The tile that was clicked stays lit and the others step back: one property
     says both "the click landed" and "the rest are not answering just now".
     Opening a title costs a process launch, and the only sign of it used to be
     a 2px line at the top of the window, hundreds of pixels from the pointer. */
  .tile.pending .title {
    color: var(--on);
  }

  /* The grid is Home's element, so the ancestor has to be global or Svelte
     prunes the rule as unused. */
  :global(.grid-pending) .tile:not(.pending) {
    opacity: 0.45;
  }

  @supports not (grid-template-rows: subgrid) {
    .tile {
      grid-row: auto;
      grid-template-rows: auto auto auto;
    }

    /* Without shared tracks the year can only be aligned the old way: the title
       reserves both lines whether it uses them or not. */
    .title {
      min-height: 2.5em;
    }
  }

  .frame {
    /* The anchor for the watched bar, which is laid over the bottom of the
       poster. */
    position: relative;
    display: block;
    transition: transform var(--dur) var(--ease-out);
  }

  /* Across the foot of the poster, over the picture. A minimum of 2% so that a
     title just taken up still shows a mark: a bar of zero width is
     indistinguishable from a title never opened, which is the opposite of what
     it means. */
  .watched {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 3px;
    border-radius: 0 0 var(--radius-md) var(--radius-md);
    /* The unwatched part is dark rather than transparent: over a bright poster
       a half-full bar with a see-through tail reads as a full one. */
    background: linear-gradient(
      to right,
      var(--on) 0,
      var(--on) var(--watched),
      #000000a6 var(--watched)
    );
  }

  .tile:hover .frame,
  .tile:focus .frame {
    transform: translateY(-4px) scale(1.02);
  }

  .tile:focus {
    outline: none;
  }

  /* An outline, not an inset shadow: an inset shadow on an <img> paints behind
     the picture, so every tile that actually had a cover — most of them — was
     showing no focus ring at all. The ring also sits on `:focus` rather than
     `:focus-visible`, because the arrow keys move focus programmatically and
     the browser does not always count that as a keyboard interaction. */
  .tile:focus .frame {
    outline: 2px solid var(--on);
    outline-offset: 3px;
    border-radius: calc(var(--radius-md) + 3px);
  }

  .title {
    align-self: start;
    font-size: var(--t-tile);
    font-weight: 600;
    line-height: 1.25;
    /* The title is what the eye scans down a grid of near-black posters, so it
       is legible at rest; the hover state is carried by the lift, not by
       finally turning the words on. */
    color: var(--text);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    transition: color var(--dur-fast) var(--ease-out);
  }

  .tile:hover .title,
  .tile:focus .title {
    color: var(--on);
  }

  .year {
    align-self: start;
    font-size: var(--t-meta);
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
    /* A title with no year must still occupy the row, or the tiles beside it in
       the same grid row would be the only ones holding the track open. */
    min-height: 1em;
  }

  @keyframes tile-in {
    from {
      opacity: 0;
      transform: translateY(10px);
    }
  }
</style>
