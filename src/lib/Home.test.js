import { render, screen, fireEvent } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";
import Home from "./Home.svelte";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (path) => `asset://localhost/${encodeURIComponent(path)}`,
}));

const CONTENTS = [
  { id: "Scrubs", title: "Scrubs", year: 2001, kind: "series", cover: null },
  { id: "How I Met Your Mother", title: "How I Met Your Mother", year: 2005, kind: "series", cover: null },
  { id: "Dune (2021)", title: "Dune", year: 2021, kind: "movie", cover: null },
  { id: "Inception (2010)", title: "Inception", year: 2010, kind: "movie", cover: null },
];

function library(overrides = {}) {
  return {
    media_root: "/Volumes/STICK/media",
    media_root_exists: true,
    contents: CONTENTS,
    warnings: [],
    ...overrides,
  };
}

function home(overrides = {}, props = {}) {
  return render(Home, {
    props: { library: library(overrides), onopen: vi.fn(), onreload: vi.fn(), ...props },
  });
}

/// The section headings, by role rather than by text. The filter chips carry
/// the same two words, so a bare text query now finds either one — which is
/// exactly what these three tests reported the moment the chips arrived.
const headings = () =>
  screen
    .queryAllByRole("heading", { level: 2 })
    .map((h) => h.textContent.trim().split(/\s+/)[0]);

describe("the grid", () => {
  it("keeps series and films apart", () => {
    home();
    expect(headings()).toEqual(["Series", "Movies"]);
  });

  it("drops a section nobody has anything in", () => {
    // A stick holding only films should not show an empty "Series" heading.
    home({ contents: CONTENTS.filter((c) => c.kind === "movie") });
    expect(headings()).toEqual(["Movies"]);
  });

  it("names the stick from the folder above the media one", () => {
    home();
    expect(screen.getByText("STICK")).toBeInTheDocument();
  });

  it("keeps a clipped volume name readable somewhere", () => {
    // The name is one line that ends in an ellipsis: a folder called
    // "Backup_Filmoteca_Completa_2026" used to paint straight across the type
    // filter beside it. Clipping is the fix, and clipped text has to say
    // somewhere what it clipped.
    home({ media_root: "/Volumes/Backup_Filmoteca_Completa_2026/media" });
    expect(screen.getByText("Backup_Filmoteca_Completa_2026")).toHaveAttribute(
      "title",
      "Backup_Filmoteca_Completa_2026",
    );
  });

  it("still names something when the media folder sits at a root", () => {
    // "/media" has no folder above it. A blank heading reads as a bug.
    home({ media_root: "/media" });
    expect(screen.getByText("Disk")).toBeInTheDocument();
  });
});

describe("the type filter", () => {
  const chip = (name) => screen.getByRole("button", { name });

  it("shows everything until asked otherwise", () => {
    home();
    expect(chip("All")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByText("Scrubs")).toBeInTheDocument();
    expect(screen.getByText("Dune")).toBeInTheDocument();
  });

  it("keeps only series, under no heading at all", async () => {
    home();
    await fireEvent.click(chip("Series"));

    expect(screen.getByText("Scrubs")).toBeInTheDocument();
    expect(screen.queryByText("Dune")).toBeNull();
    // A lit chip already says which kind this is, and the count it would carry
    // is in the volume line above: the heading would be a stutter.
    expect(headings()).toEqual([]);
  });

  it("keeps only films", async () => {
    home();
    await fireEvent.click(chip("Movies"));

    expect(screen.getByText("Dune")).toBeInTheDocument();
    expect(screen.queryByText("Scrubs")).toBeNull();
  });

  it("narrows the search rather than replacing it", async () => {
    // The two are not alternatives, and the fixture has to be able to tell
    // which one got dropped: "u" keeps Scrubs and Dune and leaves Inception
    // out, so an Inception on screen means the chip threw the query away and a
    // Scrubs means the query threw the chip away.
    home({}, { query: "u" });
    await fireEvent.click(chip("Movies"));

    expect(screen.getByText("Dune")).toBeInTheDocument();
    expect(screen.queryByText("Inception")).toBeNull();
    expect(screen.queryByText("Scrubs")).toBeNull();
  });

  it("names the kind in the empty state when a query emptied a filtered list", async () => {
    // The query is what narrowed it, but the sentence still has to say which
    // list was searched: "No titles match" over a lit Movies chip sends the
    // user looking for a series that the chip is hiding on purpose.
    home({}, { query: "scrubs" });
    await fireEvent.click(chip("Movies"));

    expect(screen.getByText(/No movies match “scrubs”/)).toBeInTheDocument();
  });

  it("says which of the two emptied the grid", async () => {
    // Naming the wrong one sends the user checking the spelling of a query
    // they never typed.
    home({ contents: CONTENTS.filter((c) => c.kind === "movie") });
    await fireEvent.click(chip("Series"));

    expect(screen.getByText(/No series on this stick/)).toBeInTheDocument();
    expect(screen.queryByText(/Check the spelling/)).toBeNull();
  });

  it("offers the way back out of an empty filter", async () => {
    home({ contents: CONTENTS.filter((c) => c.kind === "movie") });
    await fireEvent.click(chip("Series"));
    await fireEvent.click(screen.getByRole("button", { name: /titles?$/ }));

    expect(chip("All")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByText("Dune")).toBeInTheDocument();
  });
});

describe("searching", () => {
  it("keeps only what matches, whatever the case", () => {
    home({}, { query: "dune" });
    expect(screen.getByText("Dune")).toBeInTheDocument();
    expect(screen.queryByText("Scrubs")).toBeNull();
  });

  it("collapses the two sections into one list of hits", () => {
    // With a filter applied, one list reads faster than two headed groups.
    home({}, { query: "e" });
    expect(headings()).toEqual(["Results"]);
  });

  it("ignores the spaces around what was typed", () => {
    home({}, { query: "  scrubs  " });
    expect(screen.getByText("Scrubs")).toBeInTheDocument();
  });

  it("says so when nothing matches instead of showing an empty page", () => {
    // The saying-so is the behaviour. Asserting only that the titles are gone
    // passes just as well for a blank page, which is the thing to avoid.
    home({}, { query: "zzzz" });
    expect(screen.getByText(/No titles match/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /show all 4/ })).toBeInTheDocument();
    expect(screen.queryByText("Dune")).toBeNull();
  });
});

describe("an empty stick", () => {
  it("names the folder it looked in", () => {
    home({ media_root_exists: false, contents: [] });
    expect(screen.getByText(/\/Volumes\/STICK\/media/)).toBeInTheDocument();
  });

  it("shows what the scan could not make sense of", () => {
    home({ warnings: ["Empty: no video files"] });
    expect(screen.getByText(/Empty: no video files/)).toBeInTheDocument();
  });
});

describe("opening a title", () => {
  it("hands it to the caller", async () => {
    const onopen = vi.fn();
    home({}, { onopen });
    await fireEvent.click(screen.getByText("Dune"));

    expect(onopen).toHaveBeenCalledWith(
      expect.objectContaining({ id: "Dune (2021)" }),
      expect.any(String),
    );
  });
});

describe("the arrow keys, when nothing in the grid has focus", () => {
  // The ordinary state: the wheel scrolled the page, or a click landed on the
  // background. The grid used to answer for every one of these keys, which is
  // how pressing Down halfway through a library threw the page back to the top
  // and Home/End stopped reaching the document at all.

  it("lets the document keep Home and End", async () => {
    home();
    for (const key of ["Home", "End"]) {
      const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
      window.dispatchEvent(event);

      expect(event.defaultPrevented).toBe(false);
      expect(document.activeElement).toBe(document.body);
    }
  });

  it("still enters the grid on Down and Up", async () => {
    // The way in has to stay: this is how a keyboard reaches the library at all.
    const { container } = home();
    const event = new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true });
    window.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(container.querySelector("button.tile"));
  });
});

describe("a title that is opening", () => {
  it("marks the tile that was clicked, and only that one", () => {
    // The click costs a process launch. Without this the only answer was a 2px
    // line at the top of the window, and a second click was dropped in silence.
    const { container } = home({}, { pending: "Scrubs" });
    const pending = container.querySelector("button.tile.pending");

    expect(pending.dataset.id).toBe("Scrubs");
    expect(pending.getAttribute("aria-busy")).toBe("true");
    expect(container.querySelectorAll("button.tile.pending")).toHaveLength(1);
    expect(container.querySelectorAll(".grid-pending").length).toBeGreaterThan(0);
  });

  it("marks nothing while the library is just sitting there", () => {
    const { container } = home();
    expect(container.querySelector(".grid-pending")).toBeNull();
    expect(container.querySelector("button.tile.pending")).toBeNull();
  });
});

describe("the continue watching row", () => {
  const mark = (at, seconds = 100, duration = 1000, done = false) => ({
    seconds,
    duration,
    done,
    at,
  });

  const watching = (progress, props = {}) => home({}, { progress, ...props });

  it("stays away until something has been watched", () => {
    watching({});
    expect(headings()).toEqual(["Series", "Movies"]);
  });

  it("comes first, above the library it is drawn from", () => {
    watching({ "Scrubs/S01/e1.mkv": mark(10) });
    expect(headings()).toEqual(["Continue", "Series", "Movies"]);
  });

  it("puts the most recently watched title at the front", () => {
    watching({
      "Scrubs/S01/e1.mkv": mark(10),
      "Dune (2021)/Dune.mkv": mark(90),
      "Inception (2010)/Inception.mkv": mark(50),
    });
    const row = screen.getAllByRole("heading", { level: 2 })[0].parentElement;
    const titles = [...row.querySelectorAll(".title")].map((t) => t.textContent);
    expect(titles).toEqual(["Dune", "Inception", "Scrubs"]);
  });

  it("takes a mark under a title's folder, not one that merely starts like it", () => {
    // `Scrubs` and `Scrubs 2` are two shows on one stick, and matching the
    // bare name would give the first one the history of the second.
    watching({ "Scrubs 2/S01/e1.mkv": mark(10) });
    expect(headings()).toEqual(["Series", "Movies"]);
  });

  it("counts a title by its latest episode, not its first", () => {
    watching({
      "Scrubs/S01/e1.mkv": mark(10, 1000, 1000, true),
      "Scrubs/S01/e2.mkv": mark(99, 250, 1000),
    });
    const bar = document.querySelector('[data-row="Continue watching"] .watched');
    expect(bar.getAttribute("style")).toContain("25%");
  });

  it("goes away when the user narrows the library themselves", () => {
    // A search or a type filter is the user saying what they want on screen.
    // A row of something else above it, in an order they did not ask for, is
    // the app arguing with them.
    watching({ "Scrubs/S01/e1.mkv": mark(10) }, { query: "dune" });
    expect(headings()).toEqual(["Results"]);
  });

  it("holds no more than a dozen titles", () => {
    // A row of everything ever started is not a row of what is being watched:
    // it is the library again, in a worse order.
    const many = Array.from({ length: 20 }, (_, i) => ({
      id: `Show ${i}`,
      title: `Show ${i}`,
      year: 2000,
      kind: "series",
      cover: null,
    }));
    const progress = Object.fromEntries(
      many.map((content, i) => [`${content.id}/e1.mkv`, mark(i)]),
    );
    home({ contents: many }, { progress });

    const row = screen.getAllByRole("heading", { level: 2 })[0].parentElement;
    expect(row.querySelectorAll(".tile")).toHaveLength(12);
  });
});

describe("what a tile says about what has been watched", () => {
  const mark = (seconds, duration, done = false) => ({ seconds, duration, done, at: 1 });

  it("draws how far the title got", () => {
    home({}, { progress: { "Dune (2021)/Dune.mkv": mark(250, 1000) } });
    const tile = document.querySelector('[data-id="Dune (2021)"][data-row="Movies"]');
    expect(tile.querySelector(".watched").getAttribute("style")).toContain("25%");
  });

  it("says it in words as well, for anyone not looking at the bar", () => {
    home({}, { progress: { "Dune (2021)/Dune.mkv": mark(250, 1000) } });
    expect(screen.getAllByText("25% watched").length).toBeGreaterThan(0);
  });

  it("shows a full bar for something finished, whatever second it stopped on", () => {
    home({}, { progress: { "Dune (2021)/Dune.mkv": mark(0, 1000, true) } });
    const tile = document.querySelector('[data-id="Dune (2021)"][data-row="Movies"]');
    expect(tile.querySelector(".watched").getAttribute("style")).toContain("100%");
    expect(screen.getAllByText("Watched").length).toBeGreaterThan(0);
  });

  it("still shows a mark for a title only just taken up", () => {
    // A bar of zero width is indistinguishable from a title never opened,
    // which is the opposite of what a zero-second mark means.
    home({}, { progress: { "Dune (2021)/Dune.mkv": mark(0, 1000) } });
    const tile = document.querySelector('[data-id="Dune (2021)"][data-row="Movies"]');
    expect(tile.querySelector(".watched")).not.toBeNull();
  });

  it("draws nothing on a title nobody has opened", () => {
    home({}, { progress: { "Dune (2021)/Dune.mkv": mark(250, 1000) } });
    const tile = document.querySelector('[data-id="Scrubs"][data-row="Series"]');
    expect(tile.querySelector(".watched")).toBeNull();
  });

  it("draws nothing when the length could not be measured", () => {
    // A file ffprobe could not read. Dividing by it gives Infinity.
    home({}, { progress: { "Dune (2021)/Dune.mkv": mark(250, 0) } });
    const tile = document.querySelector('[data-id="Dune (2021)"][data-row="Movies"]');
    expect(tile.querySelector(".watched")).toBeNull();
  });
});

describe("coming back to a title that is on screen twice", () => {
  it("puts the focus on the copy it was opened from", async () => {
    // Started titles appear both in "continue watching" and in their own
    // group. Matching on the id alone put the ring on whichever copy the
    // document happened to hold first, which is never the one clicked.
    home(
      {},
      {
        progress: { "Scrubs/S01/e1.mkv": { seconds: 100, duration: 1000, done: false, at: 5 } },
        focusId: "Scrubs",
        focusRow: "Series",
      },
    );

    expect(document.activeElement.getAttribute("data-row")).toBe("Series");
  });

  it("falls back to the other copy when that section is gone", async () => {
    // Finishing the last episode takes a series out of "continue watching",
    // and the tile to come back to is the one in "Series".
    home({}, { focusId: "Scrubs", focusRow: "Continue watching" });
    expect(document.activeElement.getAttribute("data-id")).toBe("Scrubs");
    expect(document.activeElement.getAttribute("data-row")).toBe("Series");
  });
});

describe("two marks landing in the same second", () => {
  it("counts the episode taken up, not the one just finished", async () => {
    // Autoplay records the finished episode and dates the next one within a
    // second of each other, and `at` is whole seconds. Reading the tie the
    // other way leaves a series the viewer is now one episode into showing a
    // full bar and the word "Watched" — for the whole of the next episode's
    // first fifteen seconds, and for good if they stop inside it.
    home(
      {},
      {
        progress: {
          "Scrubs/S01/e1.mkv": { seconds: 0, duration: 1000, done: true, at: 42 },
          "Scrubs/S01/e2.mkv": { seconds: 0, duration: 1000, done: false, at: 42 },
        },
      },
    );

    const tile = document.querySelector('[data-id="Scrubs"][data-row="Series"]');
    expect(tile.querySelector(".watched").getAttribute("style")).toContain("2%");
    expect(screen.queryByText("Watched")).toBeNull();
  });
});
