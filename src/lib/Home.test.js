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

describe("the grid", () => {
  it("keeps series and films apart", () => {
    home();
    expect(screen.getByText("Series")).toBeInTheDocument();
    expect(screen.getByText("Movies")).toBeInTheDocument();
  });

  it("drops a section nobody has anything in", () => {
    // A stick holding only films should not show an empty "Series" heading.
    home({ contents: CONTENTS.filter((c) => c.kind === "movie") });
    expect(screen.queryByText("Series")).toBeNull();
    expect(screen.getByText("Movies")).toBeInTheDocument();
  });

  it("names the stick from the folder above the media one", () => {
    home();
    expect(screen.getByText("STICK")).toBeInTheDocument();
  });

  it("still names something when the media folder sits at a root", () => {
    // "/media" has no folder above it. A blank heading reads as a bug.
    home({ media_root: "/media" });
    expect(screen.getByText("Disk")).toBeInTheDocument();
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
    expect(screen.getByText("Results")).toBeInTheDocument();
    expect(screen.queryByText("Series")).toBeNull();
    expect(screen.queryByText("Movies")).toBeNull();
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

    expect(onopen).toHaveBeenCalledWith(expect.objectContaining({ id: "Dune (2021)" }));
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
