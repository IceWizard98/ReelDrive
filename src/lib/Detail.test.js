import { render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";
import Detail from "./Detail.svelte";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (path) => `asset://localhost/${encodeURIComponent(path)}`,
}));

function series(seasons) {
  return {
    summary: { id: "Scrubs", title: "Scrubs", year: 2001, kind: "series", cover: null },
    body: { kind: "series", seasons },
  };
}

function show(detail) {
  return render(Detail, {
    props: { detail, mediaRoot: "/Volumes/STICK/media", onback: vi.fn(), onplay: vi.fn() },
  });
}

const EPISODE = { file: "Scrubs/S01E01.mkv", number: 1, title: "My First Day", subtitles: [] };

describe("a season with no playable files", () => {
  // The normalisation to `[]` exists because the case is real: a `Season 3`
  // folder whose files the scanner could not make sense of. The list then
  // rendered as nothing at all — a selected tab with a void under it, and no
  // way to tell whether the fault was the app, the stick or the file names.

  it("says so instead of showing an empty page", () => {
    show(series([{ number: 3, episodes: [] }]));
    expect(screen.getByText(/No playable files in Season 3/)).toBeInTheDocument();
  });

  it("points at where the answer is, the way the library screen does", () => {
    show(series([{ number: 3, episodes: [] }]));
    expect(screen.getByText(/folders skipped/)).toBeInTheDocument();
  });

  it("handles a season carrying no list at all, not just an empty one", () => {
    show(series([{ number: 1 }]));
    expect(screen.getByText(/No playable files in Season 1/)).toBeInTheDocument();
  });

  it("stays out of the way when the season has episodes", () => {
    show(series([{ number: 1, episodes: [EPISODE] }]));
    expect(screen.queryByText(/No playable files/)).toBeNull();
    expect(screen.getByText("My First Day")).toBeInTheDocument();
  });
});

describe("what a season is called", () => {
  it("uses the name the folder was given, when it has one", () => {
    // "Season 1 - inizio" was listed as "Season 1": the label the user typed
    // reached the window and the window threw it away, so two seasons they had
    // deliberately named differently looked identical in the row of tabs.
    show(
      series([
        { number: 1, title: "Season 1 - inizio", episodes: [EPISODE] },
        { number: 2, title: "", episodes: [EPISODE] },
      ]),
    );

    expect(screen.getByRole("tab", { name: "Season 1 - inizio" })).toBeInTheDocument();
    // And the one that says nothing beyond its number still gets the canonical
    // label rather than an empty tab.
    expect(screen.getByRole("tab", { name: "Season 2" })).toBeInTheDocument();
  });

  it("falls back to the number for a season inferred from the files", () => {
    // Nothing named it: the season exists because `S03E01.mkv` said so.
    show(series([{ number: 3, title: "", episodes: [EPISODE] }]));
    expect(screen.getByText(/Season 3/)).toBeInTheDocument();
  });

  it("calls season zero what its folder is called", () => {
    show(
      series([
        { number: 0, title: "Specials", episodes: [EPISODE] },
        { number: 1, title: "", episodes: [EPISODE] },
      ]),
    );
    expect(screen.getByRole("tab", { name: "Specials" })).toBeInTheDocument();
  });

  it("says Extras for a season zero with no folder name", () => {
    show(
      series([
        { number: 0, title: "", episodes: [EPISODE] },
        { number: 1, title: "", episodes: [EPISODE] },
      ]),
    );
    expect(screen.getByRole("tab", { name: "Extras" })).toBeInTheDocument();
  });
});
