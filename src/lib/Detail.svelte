<script>
  import { clock, fileUrl, initials, placeholderStyle } from "./api.js";
  import Icon from "./Icon.svelte";

  let {
    detail,
    mediaRoot,
    onback,
    onplay,
    // What has been watched, keyed by the file's path under the media folder.
    progress = {},
    // Which episode to carry on with, worked out by the backend from the
    // seasons and the history together — including across the end of a season,
    // which is the press this page used to be missing.
    upNext = null,
  } = $props();

  let isSeries = $derived(detail.body.kind === "series");
  let seasons = $derived(isSeries ? detail.body.seasons : []);

  // Open on the first real season: extras (season 0) sort first but are not
  // where anyone starts watching. Computed up front rather than corrected in an
  // effect, which would show the wrong tab for one frame.
  //
  // `chosen` is what the user picked and nothing else; the tab actually shown is
  // derived from it. Seeding a `$state` from the props instead took a copy at
  // mount — which is what `state_referenced_locally` was warning about — and
  // would have kept pointing at the old title's season the day something hands
  // this component a second title without unmounting it.
  const firstRealSeason = (list) => Math.max(list.findIndex((s) => s.number > 0), 0);
  let chosen = $state(null);
  let selected = $derived(chosen ?? firstRealSeason(seasons));

  // Clamped: a detail with fewer seasons than the last one must never leave the
  // index pointing past the end.
  let currentSeason = $derived(seasons[Math.min(selected, seasons.length - 1)]);
  // Normalised once: `?.` on the season still left every use of `.episodes`
  // free to throw on a season that carries no list at all.
  let episodes = $derived(currentSeason?.episodes ?? []);
  let firstEpisode = $derived(episodes[0]);
  let cover = $derived(fileUrl(mediaRoot, detail.summary.cover));
  let coverFailed = $state(false);
  let art = $derived(cover && !coverFailed ? cover : null);

  let episodeCount = $derived(
    seasons.reduce((total, s) => total + (s.episodes?.length ?? 0), 0),
  );

  let classification = $derived(
    isSeries
      ? `Series · ${seasons.length} ${seasons.length === 1 ? "season" : "seasons"} · ${episodeCount} ${episodeCount === 1 ? "episode" : "episodes"}`
      : "Movie",
  );

  // Only once something here has actually been played. A title nobody has
  // opened has nothing to resume, and "Resume · Episode 1 · 0:00" over a series
  // never watched is a button lying about where you were.
  let resumable = $derived(!!upNext && !upNext.fresh);

  /// What the resume button says it will do.
  ///
  /// The episode is named because it may not be the one the tabs are showing —
  /// a season that ended leads into the next one — so a button reading only
  /// "Resume" would silently play something off-screen. The time is left off
  /// when there is none: an episode taken up but not yet started resumes at
  /// zero, and "· 0:00" reads as a position rather than as its absence.
  let resumeLabel = $derived.by(() => {
    if (!upNext) return "";
    const where =
      upNext.season === null
        ? []
        : [seasonLabel(seasons.find((s) => s.number === upNext.season) ?? { number: upNext.season }),
           `Episode ${upNext.episode}`];
    if (upNext.seconds > 0) where.push(clock(upNext.seconds));
    return where.join(" · ");
  });

  /// Play what comes next, from `seconds` in — or from where it was left.
  function resume(seconds = upNext.seconds) {
    onplay(upNext.file, upNext.subtitles, seconds);
  }

  /// Which second picking an episode out of the list starts it at.
  ///
  /// The rail on the row says where that episode got to, and a click that
  /// ignored it would not merely restart the episode: twenty seconds later the
  /// player reports the new position over the old one, so the only way to reach
  /// a particular episode would be the way that erases where you were in it.
  /// The rule is the backend's `resume_seconds` — a finished one starts again.
  function resumeAt(file) {
    const mark = progress[file];
    return mark && !mark.done ? mark.seconds : 0;
  }

  /// How much of one episode has been watched, 0 to 1, or `null` for one that
  /// has never been opened. A finished episode reads as a full bar whatever
  /// second it happened to stop on.
  function watched(file) {
    const mark = progress[file];
    if (!mark) return null;
    if (mark.done) return 1;
    if (!(mark.duration > 0)) return null;
    return Math.min(1, Math.max(0, mark.seconds / mark.duration));
  }

  /// The folder's own name wins when it has one. A user who typed
  /// "Season 1 - inizio" gets that back: the part after the number is theirs,
  /// nobody else knows what it means, and printing "Season 1" over it makes two
  /// folders they deliberately named differently look identical in the list.
  /// The backend leaves this empty for a folder that says nothing beyond its
  /// number — `S02`, `stagione 2` — and for a season inferred from the files.
  function seasonLabel(season) {
    if (season.title) return season.title;
    return season.number === 0 ? "Extras" : `Season ${season.number}`;
  }

  // Arriving here, the thing you came to do is play. Focusing the button makes
  // that one keystroke instead of a trip back to the mouse.
  function focusOnMount(node) {
    node.focus({ preventScroll: true });
  }

  // A tab strip that only answers to Tab makes the user leave and re-enter the
  // strip for every season; arrows move within it, as they do in every other
  // tab strip.
  function onTabKey(event) {
    const step = { ArrowRight: 1, ArrowLeft: -1, Home: -Infinity, End: Infinity }[event.key];
    if (step === undefined) return;
    event.preventDefault();
    const target = Number.isFinite(step) ? selected + step : step;
    chosen = Math.max(0, Math.min(target, seasons.length - 1));
    event.currentTarget.querySelectorAll("button")[chosen]?.focus();
  }
</script>

<div class="detail" style={placeholderStyle(detail.summary.title)}>
  <!-- The cover doubles as the backdrop: it is the only artwork on the stick,
       and blurring it fills the page the way a streaming app would. Without a
       cover, the title's own colour stands in. -->
  <div class="backdrop" style={art ? `background-image: url("${art}")` : ""} class:flat={!art}></div>

  <div class="content">
    <button class="back" onclick={onback}>
      <Icon name="back" size={16} /> Library
    </button>

    <div class="hero">
      <div class="poster">
        {#if art}
          <img class="art" src={art} alt="" onerror={() => (coverFailed = true)} />
        {:else}
          <span class="art art-generated">
            <span>{initials(detail.summary.title)}</span>
          </span>
        {/if}
      </div>

      <div class="meta">
        <!-- One line of classification instead of two: the year used to sit
             under the title as a lone grey number with nothing to group it
             with, so it read as a caption for the heading rather than as a
             fact about the title. -->
        <p class="classification">
          <span class="eyebrow">{classification}</span>
          {#if detail.summary.year}
            <span class="year">{detail.summary.year}</span>
          {/if}
        </p>
        <h1>{detail.summary.title}</h1>

        <!-- What the button offers depends on whether anything here has been
             watched. Untouched, it is the button this page has always had: the
             first episode of whichever season the tabs are showing, which is
             what the tabs are for. Once there is a history, the one thing
             anybody came here to do is carry on — and where to carry on from
             may be in a season the tabs are not showing, which is exactly the
             press that used to be missing. -->
        <div class="actions">
          {#if resumable}
            <button class="play" use:focusOnMount onclick={() => resume()}>
              <Icon name="play" size={15} />
              Resume {resumeLabel}
            </button>
            <button class="again" onclick={() => resume(0)}>Start over</button>
          {:else if !isSeries}
            <button
              class="play"
              use:focusOnMount
              onclick={() => onplay(detail.body.file, detail.body.subtitles)}
            >
              <Icon name="play" size={15} /> Play
            </button>
          {:else if firstEpisode}
            <button
              class="play"
              use:focusOnMount
              onclick={() => onplay(firstEpisode.file, firstEpisode.subtitles)}
            >
              <Icon name="play" size={15} />
              Play {seasonLabel(currentSeason)} · Episode {firstEpisode.number}
            </button>
          {/if}
        </div>
      </div>
    </div>

    {#if !isSeries}
      <!-- A movie page has nothing else to list, and the one thing this app
           knows that a streaming service cannot is what is actually on the
           stick. -->
      <section class="files">
        <p class="eyebrow">On disk</p>
        <ul class="panel">
          <li><span class="kind">Video</span><span class="path">{detail.body.file}</span></li>
          {#each detail.body.subtitles as subtitle}
            <li><span class="kind">Subtitle</span><span class="path">{subtitle}</span></li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if seasons.length > 0}
      {#if seasons.length === 1}
        <!-- With one season there is no tab strip, and the panel below started
             with nothing above it: a list of numbered rows on a page that had
             not said what they were. -->
        <p class="eyebrow">Episodes</p>
      {:else}
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <div class="tabs" role="tablist" tabindex="-1" aria-label="Seasons" onkeydown={onTabKey}>
          {#each seasons as season, index (season.number)}
            <button
              role="tab"
              id="season-tab-{index}"
              aria-controls="episode-list"
              aria-selected={index === selected}
              tabindex={index === selected ? 0 : -1}
              class:active={index === selected}
              onclick={() => (chosen = index)}
            >
              {seasonLabel(season)}
            </button>
          {/each}
        </div>
      {/if}

      <!-- The list is the panel the season tabs switch between; without the
           pairing a screen reader is told there are tabs but not what they
           change. Focusable rows inside, so the panel needs no tabindex. -->
      <ul
        class="episodes panel"
        id="episode-list"
        role={seasons.length > 1 ? "tabpanel" : null}
        aria-labelledby={seasons.length > 1 ? `season-tab-${selected}` : null}
      >
        {#if episodes.length === 0}
          <!-- A folder the scan could not read anything out of. Saying nothing
               left a selected tab over a void, with no way to tell whether the
               fault was the app, the stick or the file names — the library
               screen answers the same case with a cause and a way on. -->
          <li class="empty">
            No playable files in {seasonLabel(currentSeason)}. Check the folder on the
            disk — anything the scan could not read is listed under “folders skipped”
            in the library.
          </li>
        {/if}
        {#each episodes as episode (episode.file)}
          {@const seen = watched(episode.file)}
          <li>
            <button onclick={() => onplay(episode.file, episode.subtitles, resumeAt(episode.file))}>
              <!-- The number is replaced rather than decorated: a tick beside
                   "05" needs somewhere to live on a row that is already three
                   columns, and the number of an episode you have finished is
                   the part you no longer need. -->
              <span class="num" class:seen={seen === 1}>
                {#if seen === 1}
                  <Icon name="check" size={14} />
                  <span class="sr-only">Watched · Episode {episode.number}</span>
                {:else}
                  {String(episode.number).padStart(2, "0")}
                {/if}
              </span>
              <!-- The badge belongs to the title, so it travels with it. Pushed
                   to the far edge it ended up hundreds of pixels from the words
                   it qualifies, and the eye had to cross the row twice. -->
              <span class="name">
                {episode.title || `Episode ${episode.number}`}
                {#if episode.subtitles.length > 0}<span class="sub">SUB</span>{/if}
              </span>
              <span class="go"><Icon name="play" size={13} /></span>
              <!-- Only for an episode part way through. A finished one carries
                   its tick and an untouched one has nothing to say, so a bar on
                   either would be a row of full and empty rails saying what the
                   left-hand column already says. -->
              {#if seen !== null && seen < 1}
                <span
                  class="seen-bar"
                  style="--seen: {seen * 100}%"
                  role="progressbar"
                  aria-valuenow={Math.round(seen * 100)}
                  aria-valuemin="0"
                  aria-valuemax="100"
                  aria-label="Watched"
                ></span>
              {/if}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</div>

<style>
  .detail {
    position: relative;
    min-height: 100%;
  }

  /* Blurred cover as atmosphere, not as an image: tight enough that it reads as
     lighting behind the poster rather than a smear across the page.
     It used to be so faint it may as well not have been drawn — 340px of it at
     half opacity behind a mostly-dark poster, and the top of every detail page
     was flat black. Taller, brighter and reaching down past the hero, so the
     page has a top: the mask still takes it to nothing before the lists, which
     is where text has to stay readable. */
  .backdrop {
    position: absolute;
    inset: 0 0 auto;
    height: min(60vh, 480px);
    background-size: cover;
    background-position: center 25%;
    filter: blur(40px) saturate(1.7) brightness(0.8);
    transform: scale(1.25);
    opacity: 0.75;
    -webkit-mask-image: linear-gradient(#000 0%, #000000cc 40%, transparent 92%);
    mask-image: linear-gradient(#000 0%, #000000cc 40%, transparent 92%);
    pointer-events: none;
  }

  .backdrop.flat {
    background: linear-gradient(hsl(var(--tile-hue) 45% 26%), transparent);
    filter: none;
    transform: none;
    opacity: 0.6;
  }

  /* Bounded and centred. Left-aligned against a maximised window the page was
     a column of content down one side with 600px of black beside it — the
     poster, the title and every episode row sat in the left half and nothing
     ever used the right. A measure the eye can cross is worth more than filling
     the window with something. */
  .content {
    position: relative;
    max-width: 1180px;
    margin: 0 auto;
    padding: var(--space-xl) var(--gutter) var(--space-4xl);
  }

  /* A floating pill, like the controls in the player, and the only way off this
     screen for anyone not using Escape — so it stays put. On a season of
     twenty-four episodes it used to scroll away with the poster, and getting
     back to the library meant scrolling the whole list up again. Its own
     backing, because episode rows now pass underneath it. */
  .back {
    display: flex;
    width: fit-content;
    align-items: center;
    gap: var(--space-xs);
    font-size: var(--t-body);
    font-weight: 600;
    color: var(--text-2);
    padding: var(--space-sm) 0.9rem var(--space-sm) 0.7rem;
    position: sticky;
    top: var(--space-md);
    z-index: 2;
    border-radius: var(--radius-pill);
    background: var(--glass);
    border: 1px solid var(--glass-line);
    box-shadow: var(--lift);
    backdrop-filter: blur(22px) saturate(150%);
    transition:
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }

  /* --ink-raised is 1.05:1 against the page: on the only navigation control of
     the screen that was a hover state in name only. */
  .back:hover {
    color: var(--text);
    background: #ffffff1f;
  }

  .hero {
    display: flex;
    gap: clamp(1.25rem, 3vw, 2.25rem);
    align-items: flex-end;
    margin: var(--space-2xl) 0 var(--space-3xl);
    flex-wrap: wrap;
  }

  .poster {
    width: clamp(150px, 18vw, 240px);
    flex: 0 0 auto;
  }

  .meta {
    flex: 1 1 20rem;
    padding-bottom: var(--space-2xs);
  }

  .classification {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-sm) var(--space-md);
    margin: 0 0 0.7rem;
  }

  h1 {
    margin: 0;
    font-size: var(--t-display);
    font-weight: 800;
    letter-spacing: -0.03em;
    line-height: 1.02;
  }

  .year {
    font-size: var(--t-meta);
    font-weight: 600;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }

  /* Round and filled, the same shape and the same reasoning as the player's
     primary control: it is the one thing on the page the eye should find
     without looking. */
  .play {
    margin-top: var(--space-xl);
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    background: var(--text);
    color: var(--ink);
    padding: var(--space-md) 1.6rem;
    border-radius: var(--radius-pill);
    font-size: var(--t-body);
    font-weight: 700;
    transition:
      transform var(--dur-fast) var(--ease-out),
      background-color var(--dur-fast) var(--ease-out);
  }

  /* A single pixel of lift on the primary action of the page is not something
     anyone can see; the colour is what carries the feedback. */
  .play:hover {
    transform: translateY(-2px);
    background: #ffffffe6;
  }

  /* Baseline-aligned rather than centred: the two buttons are different
     heights, and centring them put the smaller one's text off the line the
     rest of the column sits on. */
  .actions {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: var(--space-lg);
  }

  /* Quiet on purpose. Starting again is the rarer of the two — nobody opens a
     part-watched series meaning to go back to the beginning — and given the
     same weight as Resume it would be a coin toss at a glance which was which. */
  .again {
    margin-top: var(--space-xl);
    padding: var(--space-sm) 0;
    color: var(--text-3);
    font-size: var(--t-meta);
    text-decoration: underline;
    text-underline-offset: 3px;
    text-decoration-color: var(--line-strong);
    transition: color var(--dur-fast) var(--ease-out);
  }

  .again:hover {
    color: var(--text);
    text-decoration-color: currentColor;
  }

  /* The one surface both lists share. Bare rows on the page were a table with
     the table taken away — nothing said where the list started or ended, and
     next to a player made of floating panels the detail page read as a
     different application. */
  .panel {
    list-style: none;
    margin: var(--space-md) 0 0;
    padding: var(--space-2xs);
    border-radius: var(--radius-lg);
    background: var(--glass);
    border: 1px solid var(--glass-line);
    box-shadow: var(--lift);
    backdrop-filter: blur(22px) saturate(150%);
  }

  /* Every eyebrow on this page labels the panel directly under it, so it keeps
     the panel's own top margin and drops the browser's paragraph one. */
  .eyebrow {
    margin: 0;
  }

  .files {
    max-width: 80ch;
  }

  .files li {
    display: flex;
    gap: var(--space-lg);
    align-items: baseline;
    padding: 0.6rem var(--space-md);
    border-bottom: 1px solid var(--line-soft);
  }

  /* A separator after the last row draws a line under a panel that already has
     an edge. */
  .panel li:last-child {
    border-bottom: none;
  }

  .kind {
    font-size: var(--t-eyebrow);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--text-3);
    min-width: 6ch;
  }

  .path {
    font-size: var(--t-meta);
    color: var(--text-2);
    word-break: break-all;
  }

  /* Pills rather than an underlined strip. The underline was the one place in
     the app still using a 2013 tab, and the strip's own bottom rule now
     collides with the panel edge directly beneath it. */
  .tabs {
    display: flex;
    gap: var(--space-2xs);
    flex-wrap: wrap;
    margin-bottom: var(--space-md);
  }

  .tabs button {
    padding: 0.45rem 0.9rem;
    border-radius: var(--radius-pill);
    font-size: var(--t-meta);
    font-weight: 600;
    color: var(--text-2);
    transition:
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }

  .tabs button:hover {
    background: #ffffff14;
    color: var(--text);
  }

  /* "This is the one you are looking at": a fill as well as the brighter text,
     because a word one shade lighter than its neighbours is no signal at all to
     anyone who cannot separate the two. */
  .tabs button.active {
    background: #ffffff1f;
    color: var(--on);
  }

  /* Bounded so the row does not stretch to a width the eye has to travel
     twice, once out to the title and once back for the number. */
  .episodes {
    max-width: 80ch;
  }

  .episodes button {
    /* The anchor for the watched bar, which sits along the row's bottom edge
       rather than in the flow: as a fourth column it would take width from the
       title on every row that had one. */
    position: relative;
    display: flex;
    align-items: center;
    gap: 1.1rem;
    width: 100%;
    text-align: left;
    padding: 0.8rem var(--space-md);
    border-radius: var(--radius-md);
    border-bottom: 1px solid var(--line-soft);
    border-left: 2px solid transparent;
    font-size: var(--t-body);
    transition:
      background-color var(--dur-fast) var(--ease-out),
      border-color var(--dur-fast) var(--ease-out);
  }

  .episodes li:last-child button {
    border-bottom: none;
  }

  /* The amber icon at the far right was the only reliable sign of which row
     the pointer was on — 78 characters away from the title the eye is reading.
     A guide on the left is where the eye already is, and --ink-hi is now a
     surface that actually changes. */
  .episodes button:hover,
  .episodes button:focus-visible {
    background: var(--ink-hi);
    border-left-color: var(--on);
  }

  .episodes .empty {
    max-width: 60ch;
    padding: 1.25rem var(--space-md);
    color: var(--text-2);
    font-size: var(--t-body);
    line-height: 1.65;
  }

  .num {
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
    font-size: var(--t-meta);
    letter-spacing: 0.05em;
    /* Fixed, because the tick that replaces the number is not the same width
       as two digits: without it every finished episode shifted its own title
       sideways, and a list half watched read as a ragged left edge. */
    min-width: 2ch;
    display: inline-flex;
    justify-content: center;
  }

  .num.seen {
    color: var(--on);
  }

  /* Along the bottom edge of the row, inside it. A bar in the flow would be a
     fourth column stealing width from the title on every row that had one. */
  .seen-bar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 2px;
    border-radius: var(--radius-pill);
    background: linear-gradient(
      to right,
      var(--on) 0,
      var(--on) var(--seen),
      transparent var(--seen)
    );
  }

  .name {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--space-md);
    color: var(--text-2);
  }

  .episodes button:hover .name {
    color: var(--text);
  }

  .sub {
    flex: none;
    font-size: 0.625rem;
    font-weight: 700;
    letter-spacing: 0.1em;
    color: var(--text-3);
    border: 1px solid var(--line);
    border-radius: var(--radius-pill);
    padding: var(--space-3xs) var(--space-sm);
  }

  .go {
    display: grid;
    place-items: center;
    color: var(--text-3);
    opacity: 0;
    transition: opacity var(--dur-fast) var(--ease-out);
  }

  .episodes button:hover .go,
  .episodes button:focus-visible .go {
    opacity: 1;
    color: var(--on);
  }
</style>
