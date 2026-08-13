import { render, screen, fireEvent } from "@testing-library/svelte";
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

describe("carrying on with a series", () => {
  const S1 = [
    { file: "Scrubs/S01/e1.mkv", number: 1, title: "My First Day", subtitles: [] },
    { file: "Scrubs/S01/e2.mkv", number: 2, title: "My Mentor", subtitles: [] },
  ];
  const S2 = [{ file: "Scrubs/S02/e1.mkv", number: 1, title: "My Overkill", subtitles: [] }];

  const SCRUBS = series([
    { number: 1, episodes: S1 },
    { number: 2, episodes: S2 },
  ]);

  function open(props = {}) {
    const onplay = vi.fn();
    render(Detail, {
      props: {
        detail: SCRUBS,
        mediaRoot: "/Volumes/STICK/media",
        onback: vi.fn(),
        onplay,
        ...props,
      },
    });
    return onplay;
  }

  const upNext = (overrides = {}) => ({
    file: "Scrubs/S02/e1.mkv",
    subtitles: [],
    season: 2,
    episode: 1,
    title: "My Overkill",
    seconds: 812,
    fresh: false,
    ...overrides,
  });

  it("offers the first episode of the open season when nothing has been watched", () => {
    // The button this page has always had. The tabs are how you choose where
    // to start something new, and a title with no history is exactly that.
    open({ upNext: upNext({ fresh: true, seconds: 0 }) });
    expect(screen.getByRole("button", { name: /Play Season 1 · Episode 1/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Resume/ })).toBeNull();
  });

  it("names the episode it would resume, even from a season the tabs are not showing", () => {
    // The press this page was missing. The tabs open on season 1; where the
    // viewer actually got to is in season 2, and a button reading only
    // "Resume" would play something off screen without saying so.
    open({ upNext: upNext() });
    expect(
      screen.getByRole("button", { name: /Resume Season 2 · Episode 1 · 13:32/ }),
    ).toBeInTheDocument();
  });

  it("plays that episode from where it was left", async () => {
    const onplay = open({ upNext: upNext() });
    await fireEvent.click(screen.getByRole("button", { name: /Resume/ }));
    expect(onplay).toHaveBeenCalledWith("Scrubs/S02/e1.mkv", [], 812);
  });

  it("offers to start that episode again from the beginning", async () => {
    const onplay = open({ upNext: upNext() });
    await fireEvent.click(screen.getByRole("button", { name: "Start over" }));
    expect(onplay).toHaveBeenCalledWith("Scrubs/S02/e1.mkv", [], 0);
  });

  it("says no time for an episode taken up but not yet started", () => {
    // Its position is zero, and "· 0:00" reads as a position rather than as
    // the absence of one.
    open({ upNext: upNext({ seconds: 0 }) });
    const button = screen.getByRole("button", { name: /Resume/ });
    expect(button.textContent).toContain("Season 2 · Episode 1");
    expect(button.textContent).not.toContain("0:00");
  });

  it("falls back to the ordinary button when the series has been watched through", () => {
    // Nothing left to resume. Offering "Resume" with nothing behind it would
    // be a button that plays whatever it likes.
    open({ upNext: null });
    expect(screen.getByRole("button", { name: /Play Season 1 · Episode 1/ })).toBeInTheDocument();
  });
});

describe("what the episode list says about what has been watched", () => {
  const EPISODES = [
    { file: "Scrubs/S01/e1.mkv", number: 1, title: "My First Day", subtitles: [] },
    { file: "Scrubs/S01/e2.mkv", number: 2, title: "My Mentor", subtitles: [] },
    { file: "Scrubs/S01/e3.mkv", number: 3, title: "My Best Friend", subtitles: [] },
  ];

  function list(progress) {
    render(Detail, {
      props: {
        detail: series([{ number: 1, episodes: EPISODES }]),
        mediaRoot: "/Volumes/STICK/media",
        onback: vi.fn(),
        onplay: vi.fn(),
        progress,
      },
    });
  }

  const mark = (seconds, duration, done = false) => ({ seconds, duration, done, at: 1 });

  it("marks a finished episode as watched", () => {
    list({ "Scrubs/S01/e1.mkv": mark(0, 1300, true) });
    expect(screen.getByText("Watched · Episode 1")).toBeInTheDocument();
  });

  it("shows how far a part watched episode got", () => {
    list({ "Scrubs/S01/e2.mkv": mark(650, 1300) });
    const bar = screen.getByRole("progressbar", { name: "Watched" });
    expect(bar).toHaveAttribute("aria-valuenow", "50");
  });

  it("says nothing at all about an episode nobody has opened", () => {
    list({});
    expect(screen.queryByRole("progressbar")).toBeNull();
    expect(screen.queryByText(/Watched/)).toBeNull();
  });

  it("draws no bar on a finished episode, which already carries its mark", () => {
    // A rail at 100% beside a tick says the same thing twice, and a list of
    // full and empty rails is harder to read than the ticks alone.
    list({ "Scrubs/S01/e1.mkv": mark(0, 1300, true) });
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("survives a mark whose length is missing", () => {
    // A file ffprobe could not measure. Dividing by it gives Infinity, and a
    // bar of Infinity percent is a bar across the whole row.
    list({ "Scrubs/S01/e1.mkv": mark(100, 0) });
    expect(screen.queryByRole("progressbar")).toBeNull();
  });
});

describe("clicking an episode in the list", () => {
  const EPISODES = [
    { file: "Scrubs/S01/e1.mkv", number: 1, title: "My First Day", subtitles: [] },
    { file: "Scrubs/S01/e2.mkv", number: 2, title: "My Mentor", subtitles: [] },
  ];

  function list(progress) {
    const onplay = vi.fn();
    render(Detail, {
      props: {
        detail: series([{ number: 1, episodes: EPISODES }]),
        mediaRoot: "/Volumes/STICK/media",
        onback: vi.fn(),
        onplay,
        progress,
      },
    });
    return onplay;
  }

  const mark = (seconds, duration, done = false) => ({ seconds, duration, done, at: 1 });

  it("carries on from where a part watched one was left", async () => {
    // The row draws a bar saying where you got to. Starting it at zero would
    // throw that position away — and twenty seconds later the player reports
    // the new one over it, so the click destroys what the bar was showing.
    const onplay = list({ "Scrubs/S01/e2.mkv": mark(650, 1300) });
    await fireEvent.click(screen.getByText("My Mentor").closest("button"));
    expect(onplay).toHaveBeenCalledWith("Scrubs/S01/e2.mkv", [], 650);
  });

  it("starts a finished one again from the beginning", async () => {
    const onplay = list({ "Scrubs/S01/e1.mkv": mark(0, 1300, true) });
    await fireEvent.click(screen.getByText("My First Day").closest("button"));
    expect(onplay).toHaveBeenCalledWith("Scrubs/S01/e1.mkv", [], 0);
  });

  it("starts one nobody has opened from the beginning", async () => {
    const onplay = list({});
    await fireEvent.click(screen.getByText("My Mentor").closest("button"));
    expect(onplay).toHaveBeenCalledWith("Scrubs/S01/e2.mkv", [], 0);
  });
});
