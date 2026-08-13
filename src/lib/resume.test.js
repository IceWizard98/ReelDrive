import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "../App.svelte";

// The whole bridge, so the app can be mounted without Tauri underneath it.
// The pure formatters come through as themselves: a second copy written here
// would let the two drift while every test went on passing.
vi.mock("./api.js", async (importOriginal) => ({
  clock: (await importOriginal()).clock,
  getLibrary: vi.fn(),
  getContent: vi.fn(),
  getPlaybackSource: vi.fn(),
  stopStream: vi.fn(),
  openAuthorSite: vi.fn(),
  getProgress: vi.fn(),
  getUpNext: vi.fn(),
  recordProgress: vi.fn(),
  takeUp: vi.fn(),
  audioUrl: vi.fn(),
  seekUrl: vi.fn(),
  subtitleUrl: vi.fn(),
  fallbackUrl: vi.fn(),
  playbackFailure: vi.fn(),
  isFullscreen: vi.fn(),
  setFullscreen: vi.fn(),
  fileUrl: () => null,
  initials: (t) => t.slice(0, 2).toUpperCase(),
  placeholderStyle: () => "",
}));

const api = await import("./api.js");

const LIBRARY = {
  media_root: "/Volumes/STICK/media",
  media_root_exists: true,
  contents: [{ id: "Scrubs", title: "Scrubs", year: 2001, kind: "series", cover: null }],
  warnings: [],
};

const EPISODE_1 = {
  file: "Scrubs/S01/e1.mkv",
  number: 1,
  title: "My First Day",
  subtitles: [],
};
const EPISODE_2 = { file: "Scrubs/S01/e2.mkv", number: 2, title: "My Mentor", subtitles: [] };
const EPISODE_S2 = { file: "Scrubs/S02/e1.mkv", number: 1, title: "My Overkill", subtitles: [] };

const SCRUBS = {
  summary: LIBRARY.contents[0],
  body: {
    kind: "series",
    seasons: [
      { number: 1, title: "Season 1", episodes: [EPISODE_1, EPISODE_2] },
      { number: 2, title: "Season 2", episodes: [EPISODE_S2] },
    ],
  },
};

function source(overrides = {}) {
  return {
    url: "http://127.0.0.1:9/stream?path=e.mkv",
    delivery: "direct",
    duration: 1300,
    title: "Episode",
    video_codec: "h264",
    subtitles: [],
    audio: [],
    audio_track: 0,
    offset: 0,
    bitmap_subtitles: 0,
    ...overrides,
  };
}

const upNext = (overrides = {}) => ({
  file: EPISODE_1.file,
  subtitles: [],
  season: 1,
  episode: 1,
  title: "My First Day",
  seconds: 0,
  fresh: true,
  ...overrides,
});

const mark = (seconds, duration = 1300, done = false) => ({ seconds, duration, done, at: 5 });

beforeEach(() => {
  vi.clearAllMocks();
  api.getLibrary.mockResolvedValue(LIBRARY);
  api.getContent.mockResolvedValue(SCRUBS);
  api.getPlaybackSource.mockResolvedValue(source());
  api.getProgress.mockResolvedValue({});
  api.getUpNext.mockResolvedValue(upNext());
  api.recordProgress.mockResolvedValue(null);
  api.takeUp.mockResolvedValue(null);
  api.playbackFailure.mockResolvedValue(null);
  api.isFullscreen.mockResolvedValue(false);
  api.setFullscreen.mockResolvedValue(false);
  // A promise, like the real one: the app calls `.catch` on what this returns.
  api.stopStream.mockResolvedValue(undefined);
});

/// Open the series and start whatever the primary button offers.
async function watch() {
  const rendered = render(App);
  // The first of them: a started title is on screen twice, once under
  // "continue watching" and once in its own group.
  await fireEvent.click((await screen.findAllByText("Scrubs"))[0]);
  await fireEvent.click(await screen.findByRole("button", { name: /Play|Resume/ }));
  const video = await waitFor(() => {
    const found = rendered.container.querySelector("video");
    if (!found) throw new Error("no player yet");
    return found;
  });
  return { ...rendered, video };
}

/// Put the element at `seconds` and let the player notice.
async function at(video, seconds) {
  Object.defineProperty(video, "currentTime", { configurable: true, value: seconds });
  await fireEvent.timeUpdate(video);
}

describe("picking a series back up", () => {
  it("starts the episode the backend says comes next, from where it was left", async () => {
    // Across the end of a season, which is where a series is most likely to be
    // abandoned for want of one press: the tabs open on season 1, and what
    // comes next is in season 2.
    api.getProgress.mockResolvedValue({ [EPISODE_2.file]: mark(0, 1300, true) });
    api.getUpNext.mockResolvedValue(
      upNext({ file: EPISODE_S2.file, season: 2, episode: 1, seconds: 812, fresh: false }),
    );

    render(App);
    // The first of them: a started title is on screen twice, once under
  // "continue watching" and once in its own group.
  await fireEvent.click((await screen.findAllByText("Scrubs"))[0]);
    await fireEvent.click(await screen.findByRole("button", { name: /Resume Season 2/ }));

    await waitFor(() => expect(api.getPlaybackSource).toHaveBeenCalledWith(EPISODE_S2.file, []));
  });

  it("asks what comes next again after the player closes", async () => {
    // The episode that was playing may now be finished, and the button behind
    // the player has to point at the one after it.
    const { video } = await watch();
    api.getUpNext.mockClear();

    await fireEvent.keyDown(window, { key: "Escape" });

    await waitFor(() => expect(api.getUpNext).toHaveBeenCalledWith("Scrubs"));
  });
});

describe("the end of an episode", () => {
  it("marks it watched and starts the next one by itself", async () => {
    const { video } = await watch();
    api.recordProgress.mockResolvedValue(mark(0, 1300, true));
    api.getUpNext.mockResolvedValue(
      upNext({ file: EPISODE_2.file, episode: 2, seconds: 0, fresh: false }),
    );

    await fireEvent.ended(video);

    await waitFor(() =>
      expect(api.recordProgress).toHaveBeenCalledWith(EPISODE_1.file, 1300, 1300),
    );
    await waitFor(() => expect(api.getPlaybackSource).toHaveBeenCalledWith(EPISODE_2.file, []));
    expect(screen.queryByText("Scrubs")).toBeNull();
  });

  it("does not file the finished episode's position under the next one", async () => {
    // The old player is unmounted as the next episode starts, and its parting
    // report arrives after `playing` already names the new file. Filed under
    // that name it is a full-length position on an episode nobody has seen —
    // which marks it watched and skips it, and then the one after it, all the
    // way to the end of the series in a few seconds.
    const { video } = await watch();
    api.getUpNext.mockResolvedValue(upNext({ file: EPISODE_2.file, episode: 2, fresh: false }));

    // Watched to the end, which is the only way this happens: the parting
    // report exists precisely because the film had got somewhere.
    await fireEvent.play(video);
    await at(video, 1299);
    await fireEvent.ended(video);
    await waitFor(() => expect(api.getPlaybackSource).toHaveBeenCalledWith(EPISODE_2.file, []));

    const forNext = api.recordProgress.mock.calls.filter(([path]) => path === EPISODE_2.file);
    expect(forNext).toEqual([]);
  });

  it("dates the episode it moves to, so the series stays in continue watching", async () => {
    // For the first fifteen seconds of an episode nothing about it is worth
    // recording, and the only other thing to show is the one just finished.
    const { video } = await watch();
    api.getUpNext.mockResolvedValue(upNext({ file: EPISODE_2.file, episode: 2, fresh: false }));

    await fireEvent.ended(video);

    await waitFor(() => expect(api.takeUp).toHaveBeenCalledWith(EPISODE_2.file, 1300));
  });

  it("goes back to the library after the last episode of the last season", async () => {
    const { video } = await watch();
    api.getUpNext.mockResolvedValue(null);

    await fireEvent.ended(video);

    await waitFor(() => expect(api.stopStream).toHaveBeenCalled());
    expect(await screen.findByText("Scrubs")).toBeInTheDocument();
  });

  it("stops rather than playing the same episode over and over", async () => {
    // The backend answering with the file that just ended means it does not
    // consider it finished — a film whose length could not be probed, so
    // nothing could be recorded about it. Starting it again is an endless loop.
    const { video } = await watch();
    api.getPlaybackSource.mockClear();
    api.getUpNext.mockResolvedValue(upNext({ file: EPISODE_1.file, fresh: false }));

    await fireEvent.ended(video);

    await waitFor(() => expect(api.stopStream).toHaveBeenCalled());
    expect(api.getPlaybackSource).not.toHaveBeenCalled();
  });

  it("carries on when the backend cannot say what comes next", async () => {
    // A folder that moved, or a stick pulled between two episodes. The film
    // that just ended, ended; that is not an error box over the library.
    const { video } = await watch();
    api.getUpNext.mockRejectedValue(new Error("scan failed"));

    await fireEvent.ended(video);

    await waitFor(() => expect(api.stopStream).toHaveBeenCalled());
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("what the app does with a reported position", () => {
  it("hands it to the backend under the file that is playing", async () => {
    const { video } = await watch();
    await fireEvent.play(video);
    await at(video, 300);

    await waitFor(() =>
      expect(api.recordProgress).toHaveBeenCalledWith(EPISODE_1.file, 300, 1300),
    );
  });

  it("keeps watching through a stick that cannot be written to", async () => {
    // This runs every twenty seconds of every film. An error box on repeat
    // over the picture, for something the viewer cannot fix mid-episode, is
    // worse than a position that is not remembered.
    const { video } = await watch();
    api.recordProgress.mockRejectedValue(new Error("read-only"));
    await fireEvent.play(video);
    await at(video, 300);

    await waitFor(() => expect(api.recordProgress).toHaveBeenCalled());
    expect(screen.queryByRole("alert")).toBeNull();
    expect(document.querySelector("video")).not.toBeNull();
  });

  it("does not write down a position the backend refused to keep", async () => {
    // The floor lives in one place, in the backend, and a refusal comes back
    // as `null`. Filing that under the episode's name puts a hole in the map
    // the grid reads: with one real mark already in it, working out which
    // episode of the series was watched last reads `.at` off the hole and the
    // library screen throws instead of drawing.
    api.getProgress.mockResolvedValue({ [EPISODE_2.file]: mark(600) });
    api.recordProgress.mockResolvedValue(null);

    const { video } = await watch();
    await fireEvent.play(video);
    await at(video, 300);
    await waitFor(() => expect(api.recordProgress).toHaveBeenCalled());

    await fireEvent.keyDown(window, { key: "Escape" });
    await fireEvent.click(await screen.findByRole("button", { name: "Library" }));

    // The one mark that was genuinely stored, and nothing invented beside it.
    const bars = await waitFor(() => {
      const found = document.querySelectorAll(".watched");
      if (!found.length) throw new Error("nothing drawn yet");
      return found;
    });
    expect([...bars].every((bar) => bar.getAttribute("style").includes("46"))).toBe(true);
  });
});

describe("leaving the player while it works out what comes next", () => {
  it("does not open the next episode behind someone who has already left", async () => {
    // `up_next` is a deep scan of a folder on a stick and the probe after it is
    // a process launch, so there are seconds between the end of an episode and
    // the start of the next one. Escape during them has to keep meaning Escape:
    // the alternative is a player that reopens itself over the library.
    const { video } = await watch();
    api.getPlaybackSource.mockClear();
    let releaseScan;
    api.getUpNext.mockReturnValue(new Promise((resolve) => (releaseScan = resolve)));

    await fireEvent.ended(video);
    await fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(document.querySelector("video")).toBeNull());

    releaseScan(upNext({ file: EPISODE_2.file, episode: 2, fresh: false }));
    await waitFor(() => expect(api.stopStream).toHaveBeenCalled());

    expect(api.getPlaybackSource).not.toHaveBeenCalled();
    expect(document.querySelector("video")).toBeNull();
  });
});
