<script>
  import Logo from "./Logo.svelte";
  import Tile from "./Tile.svelte";

  // The query lives in App: opening a title unmounts this component, and a
  // filter that clears itself every time you look at one of its own results is
  // a filter you have to retype.
  let {
    library,
    onopen,
    onreload,
    focusId = null,
    pending = null,
    query = $bindable(""),
    kind = $bindable("all"),
  } = $props();

  // Same reasoning as the query, and the same place to live: opening a title
  // unmounts this component, and a filter that forgets itself every time you
  // look at one of its own results is a filter you set twice.
  const KINDS = [
    ["all", "All"],
    ["series", "Series"],
    ["movie", "Movies"],
  ];

  let search = $state(null);
  let grids = $state(null);

  let matches = $derived(
    library.contents
      .filter((content) => kind === "all" || content.kind === kind)
      .filter((content) => content.title.toLowerCase().includes(query.trim().toLowerCase())),
  );

  let series = $derived(matches.filter((c) => c.kind === "series"));
  let movies = $derived(matches.filter((c) => c.kind === "movie"));

  // Two headed groups are only worth it when both are there to be told apart.
  // Searching collapses them, and so does a type filter — a "Series" heading
  // over the only kind on screen is a word that says nothing.
  let sections = $derived(
    query.trim()
      ? [{ label: "Results", items: matches }]
      : kind !== "all"
        ? [{ label: null, items: matches }]
        : [
            { label: "Series", items: series },
            { label: "Movies", items: movies },
          ].filter((section) => section.items.length > 0),
  );

  // What the empty state has to say depends on which of the two narrowed it,
  // and saying the wrong one sends the user looking in the wrong place.
  let narrowedBy = $derived(query.trim() ? "query" : kind !== "all" ? "kind" : null);

  // The stick's own name is the last path segment above the media folder.
  let volume = $derived(
    library.media_root.replace(/[\\/]+$/, "").split(/[\\/]/).slice(-2, -1)[0] || "Disk",
  );

  const tiles = () => (grids ? [...grids.querySelectorAll("button.tile")] : []);

  /// The tile one row up or down, matched by horizontal position rather than by
  /// counting columns: the count changes with the window, and the two sections
  /// are separate grids that a count would not cross.
  function acrossRows(list, from, direction) {
    const start = from.getBoundingClientRect();
    const rows = list
      .map((tile) => ({ tile, box: tile.getBoundingClientRect() }))
      .filter(({ box }) => (direction > 0 ? box.top > start.top + 1 : box.top < start.top - 1));
    if (rows.length === 0) return null;

    const edge = rows.reduce(
      (best, { box }) => (direction > 0 ? Math.min(best, box.top) : Math.max(best, box.top)),
      direction > 0 ? Infinity : -Infinity,
    );
    const centre = start.left + start.width / 2;
    return rows
      .filter(({ box }) => Math.abs(box.top - edge) < 2)
      .reduce((best, candidate) =>
        Math.abs(candidate.box.left + candidate.box.width / 2 - centre) <
        Math.abs(best.box.left + best.box.width / 2 - centre)
          ? candidate
          : best,
      ).tile;
  }

  /// The first tile actually on screen. Entering the grid from the top of the
  /// library while the user is looking at the middle of it is a jump, not a
  /// navigation. The offset clears the sticky header, so the tile it picks is
  /// one that is really visible.
  function firstVisible(list) {
    return list.find((tile) => tile.getBoundingClientRect().bottom > 96) ?? list[0];
  }

  function focusTile(tile) {
    if (!tile) return;
    tile.focus();
    tile.scrollIntoView({ block: "nearest" });
  }

  /// Arrow keys walk the grid the way a remote control would, "/" jumps to the
  /// search field, and Escape empties it. Without these, reaching the last
  /// title on a full stick is fourteen presses of Tab.
  function onKeydown(event) {
    if (event.metaKey || event.ctrlKey || event.altKey) return;

    if (event.target === search) {
      if (event.key === "Escape" && query) {
        event.preventDefault();
        query = "";
      } else if (event.key === "ArrowDown" || event.key === "Enter") {
        event.preventDefault();
        focusTile(tiles()[0]);
      }
      return;
    }

    if (event.key === "/") {
      event.preventDefault();
      search?.focus();
      search?.select();
      return;
    }

    const list = tiles();
    if (list.length === 0) return;
    const index = list.indexOf(document.activeElement);

    // Nothing in the grid has focus — the ordinary state after scrolling with
    // the wheel, or a click on the background. Entering the grid from its very
    // first tile threw the page back to the top of the library, and Home/End
    // stopped scrolling the document at all: the keys were taken from the
    // browser and given to a grid the user was not in.
    if (index === -1) {
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
      event.preventDefault();
      focusTile(firstVisible(list));
      return;
    }

    const move = {
      ArrowRight: () => list[index + 1],
      ArrowLeft: () => list[index - 1],
      ArrowDown: () => acrossRows(list, list[index], 1),
      ArrowUp: () => acrossRows(list, list[index], -1),
      Home: () => list[0],
      End: () => list[list.length - 1],
    }[event.key];
    if (!move) return;

    event.preventDefault();
    focusTile(move());
  }

  /// Coming back from a title, the eye is already where that title was; the
  /// focus ring has to be there too, or a keyboard user restarts from the top
  /// of the library every time.
  $effect(() => {
    if (!focusId || !grids) return;
    focusTile(grids.querySelector(`[data-id="${CSS.escape(focusId)}"]`));
  });
</script>

<svelte:window onkeydown={onKeydown} />

<div class="topcover" aria-hidden="true"></div>

<header>
  <!-- The one place the application says its own name. The header carried the
       volume's name and the stick's counts, which are the *content's*
       identity, and nothing at all said what was reading them — on a wide
       window the whole middle of the bar was empty besides. -->
  <span class="brand"><Logo size={46} /></span>

  <p class="volume">
    <span class="eyebrow">Volume</span>
    <span class="name">{volume}</span>
    <span class="counts">
      {library.contents.length} titles · {library.contents.filter((c) => c.kind === "series").length}
      series · {library.contents.filter((c) => c.kind === "movie").length} movies
    </span>
  </p>

  <!-- In the middle of the bar, which had nothing in it, and between the two
       things it sits between by right: what is on the stick, and how to narrow
       it down. `aria-pressed` rather than tabs — these filter one list, they do
       not switch between panels. -->
  <div class="kinds" role="group" aria-label="Filter by type">
    {#each KINDS as [value, label] (value)}
      <button
        class="kind"
        class:active={kind === value}
        aria-pressed={kind === value}
        onclick={() => (kind = value)}>{label}</button
      >
    {/each}
  </div>

  <!-- The shortcut used to be two spaces and a slash inside the placeholder
       string, which is not a hint: it reads as a character somebody left
       behind. A key looks like a key, and it goes away the moment the field is
       in use, because by then it has already been taken. -->
  <div class="find">
    <input
      type="search"
      placeholder="Search"
      bind:value={query}
      bind:this={search}
      aria-label="Search the library"
      aria-describedby="find-key"
    />
    {#if query === ""}
      <kbd id="find-key">/</kbd>
    {/if}
  </div>
</header>

<div bind:this={grids}>
  {#if library.contents.length === 0}
    <!-- An empty state is the whole screen when it happens, so it sits where
         the eye lands rather than as one line in the top-left corner of a black
         page: the sentence had the same weight as a caption under a poster. -->
    <div class="empty">
      <p class="lead">Nothing in <code>{library.media_root}</code></p>
      <p>Create one folder per movie or series, then reload.</p>
      <button class="action" onclick={onreload}>Reload</button>
    </div>
  {:else if matches.length === 0}
    <!-- Naming the wrong one of the two sends the user looking in the wrong
         place: checking the spelling of a query they never typed, or hunting
         for films on a stick that only holds series. -->
    <div class="empty">
      {#if narrowedBy === "query"}
        <p class="lead">No {kind === "all" ? "titles" : kind === "series" ? "series" : "movies"} match “{query}”</p>
        <p>
          Check the spelling, or
          <button class="link" onclick={() => ((query = ""), (kind = "all"))}
            >show all {library.contents.length}</button
          >.
        </p>
      {:else}
        <p class="lead">No {kind === "series" ? "series" : "movies"} on this stick</p>
        <p>
          There {library.contents.length === 1 ? "is" : "are"}
          <button class="link" onclick={() => (kind = "all")}
            >{library.contents.length}
            {library.contents.length === 1 ? "title" : "titles"}</button
          > here of the other kind.
        </p>
      {/if}
    </div>
  {:else}
    {#each sections as section (section.label ?? kind)}
      <section>
        <!-- No heading under a lit chip: it would repeat the chip word for
             word, and the count it carries is already in the volume line. -->
        {#if section.label}
          <h2>
            <span class="eyebrow">{section.label}</span>
            <span class="count">{section.items.length}</span>
            <span class="rule"></span>
          </h2>
        {/if}
        <div class="grid" class:grid-pending={pending !== null}>
          {#each section.items as content, index (content.id)}
            <Tile
              {content}
              mediaRoot={library.media_root}
              {onopen}
              {index}
              pending={content.id === pending}
            />
          {/each}
        </div>
      </section>
    {/each}
  {/if}
</div>

{#if library.warnings.length > 0}
  <details class="warnings">
    <summary>{library.warnings.length} folders skipped</summary>
    <ul>
      {#each library.warnings as warning}<li>{warning}</li>{/each}
    </ul>
  </details>
{/if}

<style>
  /* A floating panel, not a bar welded to the window. The player already
     established the shape — a surface inset from the edges, with its own line
     and its own shadow, that content passes behind — and Home was the screen
     that made the app look like two different products. The blur is real work
     here: posters scroll under it. */
  header {
    display: flex;
    align-items: center;
    /* A belt to go with the window minimum: below ~400px the volume line and
       the search field used to compress into each other until the counts broke
       over three lines. `margin-left: auto` on the input keeps it right where
       it is as long as they share a row. */
    flex-wrap: wrap;
    gap: var(--space-lg) var(--space-xl);
    padding: var(--space-md) var(--space-md) var(--space-md) 1.1rem;
    margin: var(--space-md) var(--gutter) var(--space-xl);
    position: sticky;
    top: var(--space-md);
    z-index: 2;
    border-radius: var(--radius-lg);
    background: var(--glass);
    border: 1px solid var(--glass-line);
    /* Not `--lift`. That shadow has a 30px blur against a 10px offset, so a
       third of it spreads *upward* — past the strip below, and out to the full
       width of the window. The strip is opaque and the bleed beside it is not,
       and the boundary between them was a hairline across the top of every
       screenshot. The negative spread pulls the whole thing back under the
       panel, which is the only place a panel casts a shadow anyway. */
    box-shadow: 0 12px 26px -10px #00000099;
    backdrop-filter: blur(22px) saturate(150%);
  }

  /* The strip of window above the panel, which a poster would otherwise cross
     on its way out of view.
     A sibling, not `header::before`. As a child of the panel it was governed by
     the panel's own `backdrop-filter`: an element with a filter is the
     containing block for its fixed descendants, so `top: 0; left: 0; right: 0`
     resolved against the panel instead of the window. It covered nothing it was
     meant to cover, and it painted an opaque band over the top of the glass —
     which is the hairline that ran across the top of every screenshot. */
  .topcover {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    height: var(--space-md);
    background: var(--ink);
    pointer-events: none;
    z-index: 3;
  }

  /* The signature line: a rented catalogue cannot tell you what is on the
     shelf, this one can. */
  /* Dimmer than the volume name beside it: this is the thing that never
     changes, and the name of what you plugged in is the thing that does. */
  .brand {
    display: flex;
    /* Big enough for the sprocket holes to read as holes. Below about forty
       pixels the filmstrip turns to grain and the mark stops being a picture of
       anything. */
    color: var(--text-2);
    margin-right: var(--space-3xs);
  }

  .volume {
    margin: 0;
    display: grid;
    gap: var(--space-3xs);
  }

  .name {
    font-size: var(--t-section);
    font-weight: 700;
    letter-spacing: -0.01em;
  }

  .counts {
    font-size: var(--t-meta);
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
  }

  /* A pill, like every other control in the app now. The player's row is round
     buttons; a 6px rectangle beside them read as a form field from another
     application. */
  /* The same pill the season tabs are, and lit the same way: a fill as well as
     brighter text, because one shade of grey against another is no signal to
     anyone who cannot separate them. */
  .kinds {
    display: flex;
    gap: var(--space-3xs);
  }

  .kind {
    padding: var(--space-xs) var(--space-md);
    border-radius: var(--radius-pill);
    font-size: var(--t-meta);
    font-weight: 600;
    color: var(--text-3);
    transition:
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }

  .kind:hover {
    color: var(--text-2);
  }

  .kind.active {
    background: var(--ink-hi);
    color: var(--on);
  }

  .find {
    position: relative;
    margin-left: auto;
    display: flex;
    align-items: center;
  }

  /* Inside the field, not after it: the key belongs to the control it
     operates. `right` clears the pill's own curve. */
  kbd {
    position: absolute;
    right: 0.65rem;
    pointer-events: none;
    padding: 0.05rem 0.4rem;
    border-radius: var(--radius-sm);
    border: 1px solid var(--line);
    font-family: var(--ui);
    font-size: var(--t-eyebrow);
    line-height: 1.4;
    color: var(--text-3);
  }

  input {
    width: min(280px, 40vw);
    padding: var(--space-sm) 0.95rem;
    border-radius: var(--radius-pill);
    /* The only input in the app, on a fill that sits 1.05:1 from the page: the
       border is the only thing that says it is a field, so it is the border
       that has to clear 3:1. */
    border: 1px solid var(--line-strong);
    /* Opaque, not translucent like the panel around it: the border is the only
       thing identifying the field, and a poster passing behind a see-through
       fill would take that border under the 3:1 of WCAG 1.4.11 exactly when a
       bright cover scrolls past. */
    background: var(--ink);
    font-size: var(--t-body);
    transition: border-color var(--dur-fast) var(--ease-out);
  }

  input:hover {
    border-color: #6e6e7c;
  }

  input::placeholder {
    color: var(--text-3);
  }

  section {
    padding: 0 var(--gutter);
    margin-bottom: var(--space-3xl);
  }

  h2 {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    margin: 0 0 var(--space-lg);
    font-weight: inherit;
  }

  .rule {
    flex: 1;
    height: 1px;
    background: var(--line-soft);
  }

  /* Beside the label, not at the far end of the rule. At 1440px the count sat
     1300px from the word it counted, where it reads as a stray number rather
     than as part of the heading. */
  .count {
    font-size: var(--t-eyebrow);
    font-weight: 700;
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
    border: 1px solid var(--line);
    border-radius: var(--radius-pill);
    padding: 0 0.45rem;
    line-height: 1.5;
  }

  .grid {
    display: grid;
    /* A maximised desktop window should spend the extra width on bigger
       posters, not on more columns: at 2560px the fixed minimum gave fifteen
       columns of stamps with half the screen empty underneath. */
    grid-template-columns: repeat(auto-fill, minmax(clamp(148px, 11vw, 200px), 1fr));
    grid-auto-rows: auto;
    /* The caption no longer reserves a second line it usually does not need, so
       the rows sit closer than the 1.75rem that was compensating for it; the
       column gap grows to keep posters from touching at the widest columns. */
    gap: var(--space-2xl) var(--space-lg);
    align-items: start;
  }

  .empty {
    margin: clamp(var(--space-3xl), 12vh, 6rem) auto;
    padding: 0 var(--gutter);
    max-width: 46ch;
    text-align: center;
    color: var(--text-2);
    line-height: 1.65;
    font-size: var(--t-body);
  }

  .empty p {
    margin: 0 0 var(--space-sm);
  }

  .lead {
    font-size: var(--t-section);
    font-weight: 700;
    letter-spacing: -0.01em;
    color: var(--text);
  }

  code {
    color: var(--text);
    word-break: break-all;
  }

  .link {
    color: var(--on);
    text-decoration: underline;
    padding: 0;
  }

  .action {
    margin-top: var(--space-md);
    padding: var(--space-sm) 1.4rem;
    border-radius: var(--radius-pill);
    background: var(--text);
    color: var(--ink);
    font-size: var(--t-body);
    font-weight: 700;
    transition: background-color var(--dur-fast) var(--ease-out);
  }

  .action:hover {
    background: #ffffffe6;
  }

  .warnings {
    margin: 0 var(--gutter) var(--space-3xl);
    color: var(--text-3);
    font-size: var(--t-meta);
  }

  .warnings summary {
    cursor: pointer;
  }

  .warnings ul {
    margin: var(--space-sm) 0 0;
    padding-left: 1.1rem;
    line-height: 1.7;
  }
</style>
