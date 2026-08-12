import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";
import Player from "./Player.svelte";

// Everything the player asks the backend for. Mocked as a whole: the real
// module reaches for Tauri's IPC, which does not exist here.
vi.mock("./api.js", () => ({
  audioUrl: vi.fn(),
  seekUrl: vi.fn(),
  subtitleUrl: vi.fn(),
  fallbackUrl: vi.fn(),
  playbackFailure: vi.fn(),
  isFullscreen: vi.fn(),
  setFullscreen: vi.fn(),
}));

const api = await import("./api.js");

function source(overrides = {}) {
  return {
    url: "http://127.0.0.1:9/stream?token=x&path=Film.mkv",
    delivery: "remux",
    duration: 3600,
    title: "Film",
    video_codec: "h264",
    subtitles: [],
    audio: [],
    audio_track: 0,
    offset: 0,
    ...overrides,
  };
}

const twoLanguages = [
  { index: 0, label: "ENG — Original", language: "eng" },
  { index: 1, label: "ITA", language: "ita" },
];

function play(overrides = {}) {
  return render(Player, {
    props: { source: source(overrides), relativePath: "Film.mkv", onexit: vi.fn() },
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  api.audioUrl.mockResolvedValue({
    url: "http://127.0.0.1:9/hls/t/s/index.m3u8",
    delivery: "transcode-audio",
    audio_track: 1,
    offset: 0,
  });
  api.subtitleUrl.mockResolvedValue("http://127.0.0.1:9/subtitle?track=0");
  api.seekUrl.mockResolvedValue("http://127.0.0.1:9/stream?t=60");
  api.fallbackUrl.mockResolvedValue("http://127.0.0.1:9/stream?fallback=1");
  api.playbackFailure.mockResolvedValue(null);
  api.isFullscreen.mockResolvedValue(false);
  api.setFullscreen.mockResolvedValue(false);
});

const oneTrack = [
  { url: "/s.vtt", label: "Italiano", language: "it", path: "Film.mkv", track: 0 },
];

describe("subtitles", () => {
  it("draws cues at a size that follows the picture, and lets that be changed", async () => {
    // The size that shipped was `1rem` — sixteen pixels whatever the window
    // does, which on a full-screen film is about a third of what every other
    // player draws. The class is what the stylesheet keys the three sizes off,
    // so this is the whole mechanism.
    const { container } = play({ subtitles: oneTrack });
    const stage = container.querySelector(".stage");
    expect(stage.classList.contains("cue-small")).toBe(false);
    expect(stage.classList.contains("cue-large")).toBe(false);

    await fireEvent.click(screen.getByRole("button", { name: "Subtitles" }));
    await fireEvent.click(screen.getByRole("button", { name: "Large" }));
    expect(stage.classList.contains("cue-large")).toBe(true);

    // The list stays open: picking a size is something you do while watching,
    // and reopening the menu for every tap is how you never find the one that
    // fits.
    await fireEvent.click(screen.getByRole("button", { name: "Small" }));
    expect(stage.classList.contains("cue-large")).toBe(false);
    expect(stage.classList.contains("cue-small")).toBe(true);
  });

  it("offers no size control when there is nothing to size", () => {
    play({ subtitles: [] });
    expect(screen.queryByRole("button", { name: "Large" })).toBeNull();
  });

  it("says so when a track cannot be read instead of switching on nothing", async () => {
    // A `<track>` that fails to load does it in complete silence: it appears in
    // the menu, it switches on, and not one line ever arrives. That is
    // indistinguishable from a player whose subtitles are broken, which is
    // exactly how it was reported.
    const { container } = play({ subtitles: oneTrack });
    await fireEvent.error(container.querySelector("track"));

    expect(screen.getByText(/could not be read/)).toBeInTheDocument();
    expect(screen.getByText(/Italiano/)).toBeInTheDocument();

    await fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByText(/could not be read/)).toBeNull();
  });

  it("explains a track it cannot show rather than offering nothing", async () => {
    // A Blu-ray rip whose only tracks are pictures used to produce no menu at
    // all — after the detail page had already said the film had subtitles.
    play({ subtitles: [], bitmap_subtitles: 2 });
    await fireEvent.click(screen.getByRole("button", { name: "Subtitles" }));

    expect(screen.getByText(/2 tracks are pictures/)).toBeInTheDocument();
  });
});

describe("full screen", () => {
  it("asks the window, not the document", async () => {
    // `element.requestFullscreen()` is rejected by the webview this app runs in
    // on macOS, and the rejection was caught and dropped: the button was dead.
    // The window can do it, and the capability for it was already granted.
    const { container } = play();
    const stage = container.querySelector(".stage");
    api.setFullscreen.mockResolvedValue(true);

    await fireEvent.click(screen.getByRole("button", { name: "Full screen" }));

    expect(api.setFullscreen).toHaveBeenCalledWith(true, stage);
    expect(await screen.findByRole("button", { name: "Leave full screen" })).toBeInTheDocument();
  });

  it("believes the answer rather than the request", async () => {
    // A refused request that still flipped the icon would leave the button
    // lying about where the user is.
    play();
    api.setFullscreen.mockResolvedValue(false);

    await fireEvent.click(screen.getByRole("button", { name: "Full screen" }));

    await waitFor(() => expect(api.setFullscreen).toHaveBeenCalled());
    expect(screen.getByRole("button", { name: "Full screen" })).toBeInTheDocument();
  });

  it("Escape leaves the screen before it leaves the film", async () => {
    // Two irreversible things at once otherwise: the film closes and the whole
    // desktop comes back, and only one of them was asked for.
    const onexit = vi.fn();
    const { container } = render(Player, {
      props: { source: source(), relativePath: "Film.mkv", onexit },
    });
    api.setFullscreen.mockResolvedValue(true);
    await fireEvent.click(screen.getByRole("button", { name: "Full screen" }));
    await screen.findByRole("button", { name: "Leave full screen" });

    api.setFullscreen.mockResolvedValue(false);
    await fireEvent.keyDown(window, { key: "Escape" });

    expect(onexit).not.toHaveBeenCalled();
    expect(api.setFullscreen).toHaveBeenLastCalledWith(false, container.querySelector(".stage"));

    // And the second press does leave.
    await screen.findByRole("button", { name: "Full screen" });
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(onexit).toHaveBeenCalledTimes(1);
  });

  it("asks where the window already is instead of assuming it is windowed", async () => {
    // Nothing brings the desktop back on the way out of a film: leaving by the
    // back button — the one way out that is not Escape — puts the library on a
    // full-screen window. The next film then started with the flag at false, so
    // the button offered "Full screen" while the screen was already full, and
    // Escape read the same false and closed the film with the desktop still
    // gone: the two-things-at-once this was fixed to avoid.
    const onexit = vi.fn();
    api.isFullscreen.mockResolvedValue(true);
    render(Player, {
      props: { source: source(), relativePath: "Film.mkv", onexit },
    });

    expect(
      await screen.findByRole("button", { name: "Leave full screen" }),
    ).toBeInTheDocument();

    api.setFullscreen.mockResolvedValue(false);
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(onexit).not.toHaveBeenCalled();
    expect(api.setFullscreen).toHaveBeenCalledWith(false, expect.anything());
  });

  it("leaves no question in flight once the player is gone", async () => {
    // The answer arrives 150 ms after a resize, and leaving the film in that
    // window used to fire the query anyway — a round trip to a window whose
    // player no longer exists, written into state nothing reads.
    const { unmount } = play();
    await waitFor(() => expect(api.isFullscreen).toHaveBeenCalled());

    api.isFullscreen.mockClear();
    await fireEvent(window, new Event("resize"));
    unmount();
    await new Promise((resolve) => setTimeout(resolve, 250));

    expect(api.isFullscreen).not.toHaveBeenCalled();
  });
});

describe("the wait indicator", () => {
  it("puts nothing over the picture once the film is running", async () => {
    // Its backing is a radial gradient the size of its padding, and it used to
    // be painted whether or not anything was being waited for: a dark halo in
    // the middle of every frame of every film, for the whole running time.
    const { container } = play();
    const status = container.querySelector(".status");
    expect(status).toBeTruthy();

    await fireEvent.play(container.querySelector("video"));
    expect(status.classList.contains("waiting")).toBe(false);
  });

  it("still darkens the frame while something is being waited for", async () => {
    // The other half of the same rule: a spinner in #ffffff26 over a daylight
    // scene is invisible without it.
    const { container } = play();
    const video = container.querySelector("video");
    await fireEvent.play(video);
    await fireEvent.waiting(video);

    expect(container.querySelector(".status").classList.contains("waiting")).toBe(true);
  });
});

describe("the audio menu", () => {
  it("is not offered when there is nothing to choose between", () => {
    // One track, or none: a menu with a single entry is a menu that wastes a
    // click and says nothing.
    play({ audio: [] });
    expect(screen.queryByRole("button", { name: "Audio" })).toBeNull();

    play({ audio: [twoLanguages[0]] });
    expect(screen.queryByRole("button", { name: "Audio" })).toBeNull();
  });

  it("lists every language once opened", async () => {
    play({ audio: twoLanguages });
    await fireEvent.click(screen.getByRole("button", { name: "Audio" }));

    expect(screen.getByRole("button", { name: /ENG — Original/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /ITA/ })).toBeInTheDocument();
  });

  it("asks the backend for the second language from the second the film is at", async () => {
    // The second is the whole point: changing language must not restart the
    // film. Asserting only "a number at least zero" would hold just as well
    // for a hardcoded 0, which is exactly the bug it has to catch.
    const { container } = play({ audio: twoLanguages });
    const video = container.querySelector("video");
    Object.defineProperty(video, "currentTime", { configurable: true, value: 90 });
    await fireEvent.timeUpdate(video);

    await fireEvent.click(screen.getByRole("button", { name: "Audio" }));
    await fireEvent.click(screen.getByRole("button", { name: /ITA/ }));

    await waitFor(() => expect(api.audioUrl).toHaveBeenCalledTimes(1));
    const [path, seconds, track] = api.audioUrl.mock.calls[0];
    expect(path).toBe("Film.mkv");
    expect(track).toBe(1);
    expect(seconds).toBe(90);
  });

  it("does not rebuild the stream for the language already playing", async () => {
    // Re-choosing what is playing would restart ffmpeg and drop the picture for
    // no reason at all.
    play({ audio: twoLanguages });
    await fireEvent.click(screen.getByRole("button", { name: "Audio" }));
    await fireEvent.click(screen.getByRole("button", { name: /ENG — Original/ }));

    expect(api.audioUrl).not.toHaveBeenCalled();
  });

  it("follows the track the backend actually used, not the one asked for", async () => {
    // Asking for a track the file does not have comes back as the first one.
    // The tick has to sit where the sound is.
    api.audioUrl.mockResolvedValue({
      url: "http://127.0.0.1:9/hls/t/s/index.m3u8",
      delivery: "remux",
      audio_track: 0,
      offset: 0,
    });
    play({ audio: twoLanguages });
    await fireEvent.click(screen.getByRole("button", { name: "Audio" }));
    await fireEvent.click(screen.getByRole("button", { name: /ITA/ }));

    await waitFor(() => expect(api.audioUrl).toHaveBeenCalled());
    await fireEvent.click(screen.getByRole("button", { name: "Audio" }));
    const chosen = screen.getByRole("button", { name: /ENG — Original/ });
    await waitFor(() => expect(chosen.className).toContain("current"));
  });

  it("keeps the chosen language when the picture fails and the film is converted", async () => {
    // The fallback is the one restart that does not go through a seek, and it
    // used to ask for track 0: the film came back in the wrong language, and
    // the next seek — which does carry the track — flipped it back mid-scene.
    const { container } = play({ audio: twoLanguages });
    await fireEvent.click(screen.getByRole("button", { name: "Audio" }));
    await fireEvent.click(screen.getByRole("button", { name: /ITA/ }));
    await waitFor(() => expect(api.audioUrl).toHaveBeenCalled());

    await fireEvent.error(container.querySelector("video"));

    await waitFor(() => expect(api.fallbackUrl).toHaveBeenCalled());
    const [path, , track] = api.fallbackUrl.mock.calls[0];
    expect(path).toBe("Film.mkv");
    expect(track).toBe(1);
  });

  it("shows the failure instead of leaving a dead picture", async () => {
    api.audioUrl.mockRejectedValue("no such track");
    play({ audio: twoLanguages });
    await fireEvent.click(screen.getByRole("button", { name: "Audio" }));
    await fireEvent.click(screen.getByRole("button", { name: /ITA/ }));

    expect(await screen.findByText(/no such track/)).toBeInTheDocument();
  });
});

describe("subtitles", () => {
  const italian = [
    { url: "http://127.0.0.1:9/subtitle?track=0", label: "ITA", language: "ita", path: "Film.it.srt", track: 0 },
  ];

  it("offers no menu when the film carries none", () => {
    play({ subtitles: [] });
    expect(screen.queryByRole("button", { name: "Subtitles" })).toBeNull();
  });

  it("puts one track element per subtitle into the video", () => {
    const { container } = play({ subtitles: italian });
    const tracks = container.querySelectorAll("track");
    expect(tracks).toHaveLength(1);
    expect(tracks[0].getAttribute("srclang")).toBe("ita");
  });

  it("starts with subtitles off", () => {
    // A film that opens with subtitles nobody asked for is worse than one that
    // needs a click to get them.
    //
    // `textTracks[i].mode` would be the direct reading, but jsdom never builds
    // a TextTrackList from <track> elements — it stays empty, so the mode says
    // nothing here. `default` is the other half of the same behaviour and is
    // real in jsdom: a track carrying it is shown by the engine at load, before
    // any of this component's code runs, and no amount of `mode` handling
    // afterwards would stop the first cue appearing uninvited.
    const { container } = play({ subtitles: italian });

    expect(container.querySelector("track").hasAttribute("default")).toBe(false);
    expect(screen.getByRole("button", { name: "Subtitles" }).className).not.toContain("on");
  });
});

describe("a paused film", () => {
  it("says so over the picture, not only in the bar", async () => {
    // A still frame is indistinguishable from a film that never started, and on
    // a dark shot there is nothing else on screen to tell them apart.
    const { container } = play();
    const video = container.querySelector("video");
    await fireEvent.playing(video);
    expect(container.querySelector(".resume")).toBeNull();

    await fireEvent.pause(video);
    expect(container.querySelector(".resume")).not.toBeNull();
  });
});

describe("the delivery badge", () => {
  it("names the delivery in words rather than in the backend's own key", () => {
    // "transcode-audio" printed over a film is the internal name of a decision
    // the user never made and cannot act on.
    play({ delivery: "transcode-audio" });
    expect(screen.getByText("Audio converted")).toBeInTheDocument();
  });

  it("stays out of the way when the file plays untouched", () => {
    // Nothing happened to it, so there is nothing to report.
    const { container } = play({ delivery: "direct" });
    expect(container.querySelector(".mode")).toBeNull();
  });
});

describe("the speed menu", () => {
  it("changes the rate of the element itself", async () => {
    const { container } = play();
    await fireEvent.click(screen.getByRole("button", { name: "Speed" }));
    await fireEvent.click(screen.getByRole("button", { name: /1\.5×/ }));

    expect(container.querySelector("video").playbackRate).toBe(1.5);
  });

  it("reports the chosen speed on the bar", async () => {
    play();
    await fireEvent.click(screen.getByRole("button", { name: "Speed" }));
    await fireEvent.click(screen.getByRole("button", { name: /0\.5×/ }));

    expect(screen.getByRole("button", { name: "0.5×" })).toBeInTheDocument();
  });
});

describe("two seeks in flight at once", () => {
  // The film has subtitles on purpose: `retimeSubtitles` is an extra await
  // between "this is the newest request" and the point where the element is
  // pointed at a stream, and the IPC calls behind it come back in whatever
  // order the backend answers. Without subtitles the ordering is a single
  // microtask and the race cannot be seen — which is why it was never noticed.
  const withSubtitles = {
    subtitles: [{ path: "Film.srt", track: 0, label: "ITA", url: "http://127.0.0.1:9/sub?t=0" }],
  };

  function seek(container, seconds) {
    const slider = container.querySelector("input.seek");
    fireEvent.input(slider, { target: { value: String(seconds) } });
    return fireEvent.change(slider);
  }

  it("lands on the newest target even when the older request answers last", async () => {
    // The failure this catches: the element ends up on the stream that starts
    // at 100 while `offset` says 500, so the bar reads 8:20 over a picture
    // showing 1:40, and every seek after it is computed from an offset that is
    // 400 seconds wrong.
    const { container } = play(withSubtitles);
    const video = container.querySelector("video");
    video.play = vi.fn().mockResolvedValue(undefined);

    const answers = [];
    api.subtitleUrl.mockImplementation(
      (_path, _track, at) => new Promise((resolve) => answers.push(() => resolve(`sub?at=${at}`))),
    );
    api.seekUrl.mockImplementation(async (_path, at) => `stream?at=${at}`);

    await seek(container, 100);
    await seek(container, 500);
    await waitFor(() => expect(answers.length).toBe(2));

    // The older request comes back last, which is the whole point.
    answers[1]();
    answers[0]();
    await waitFor(() => expect(video.src).toContain("at=500"));

    expect(video.src).not.toContain("at=100");
    expect(screen.getByLabelText("Position")).toHaveValue("500");
  });

  it("keeps the subtitle tracks of the newest target, not of the one it replaced", async () => {
    // Cues cut for second 100 under a picture that starts at 500 is 400
    // seconds of drift: no line ever appears at the right moment again.
    const { container } = play(withSubtitles);
    const video = container.querySelector("video");
    video.play = vi.fn().mockResolvedValue(undefined);

    const answers = [];
    api.subtitleUrl.mockImplementation(
      (_path, _track, at) => new Promise((resolve) => answers.push(() => resolve(`sub?at=${at}`))),
    );
    api.seekUrl.mockImplementation(async (_path, at) => `stream?at=${at}`);

    await seek(container, 100);
    await seek(container, 500);
    await waitFor(() => expect(answers.length).toBe(2));

    answers[1]();
    answers[0]();
    await waitFor(() => expect(video.src).toContain("at=500"));

    const track = container.querySelector("track");
    expect(track.getAttribute("src")).toContain("at=500");
  });
});

describe("the keyboard shortcuts", () => {
  // Nine of them, and j/k/l are not guessable. Without a list they are dead
  // code for anyone who has not used the same keys somewhere else.
  it("are not on screen until asked for", () => {
    const { container } = play();
    expect(container.querySelector(".keys")).toBeNull();
  });

  it("open with ? and close with the same key", async () => {
    const { container } = play();
    await fireEvent.keyDown(window, { key: "?" });
    expect(screen.getByRole("note", { name: "Keyboard shortcuts list" })).toBeInTheDocument();
    expect(screen.getByText("Play or pause")).toBeInTheDocument();

    await fireEvent.keyDown(window, { key: "?" });
    expect(container.querySelector(".keys")).toBeNull();
  });

  it("close on Escape without leaving the film", async () => {
    // Escape is layered: the innermost thing goes first. Leaving the film from
    // inside a list of key bindings is an expensive way to close it.
    const onexit = vi.fn();
    const { container } = render(Player, {
      props: { source: source(), relativePath: "Film.mkv", onexit },
    });

    await fireEvent.keyDown(window, { key: "?" });
    await fireEvent.keyDown(window, { key: "Escape" });

    expect(container.querySelector(".keys")).toBeNull();
    expect(onexit).not.toHaveBeenCalled();
  });
});

describe("a playback failure while a nudge is still pending", () => {
  it("does not let the nudge fire afterwards and supersede the fallback", async () => {
    // ArrowRight arms a 400ms timer. If the picture fails inside that window
    // the fallback starts, and the orphaned timer then bumps the token and
    // abandons it — after `converted` and the transcode delivery have already
    // been set. An ffmpeg launched for nothing, and a second reload.
    vi.useFakeTimers();
    try {
      const { container } = play();
      const video = container.querySelector("video");
      video.play = vi.fn().mockResolvedValue(undefined);

      await fireEvent.keyDown(window, { key: "ArrowRight" });
      await fireEvent.error(video);
      await vi.advanceTimersByTimeAsync(1000);

      expect(api.fallbackUrl).toHaveBeenCalledTimes(1);
      expect(api.seekUrl).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });
});
