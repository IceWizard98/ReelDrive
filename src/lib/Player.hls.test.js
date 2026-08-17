import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";
import Player from "./Player.svelte";

// The engine this file is about is the one nobody develops on: WKWebView plays
// HLS by itself, so on macOS hls.js is never even fetched. Windows and Linux go
// through it for every converted film, and until this file existed not one line
// of that path was executed anywhere.
//
// hls.js is mocked rather than driven for real: jsdom has no Media Source
// Extensions, so the real library answers `isSupported() === false` and hands
// the URL straight back to the element. That is a fourth path, not this one.
const instances = [];

class FakeHls {
  static Events = { ERROR: "hlsError" };
  static isSupported = () => true;

  constructor(config) {
    this.config = config;
    this.handlers = {};
    this.destroyed = false;
    instances.push(this);
  }
  on(event, handler) {
    this.handlers[event] = handler;
  }
  loadSource(url) {
    this.source = url;
  }
  attachMedia(media) {
    this.media = media;
  }
  destroy() {
    this.destroyed = true;
  }
  /// What the library does when a load has failed for good.
  fail(data) {
    this.handlers[FakeHls.Events.ERROR]?.(FakeHls.Events.ERROR, data);
  }
}

// Set by the one test about a chunk that never arrives. A getter rather than a
// rejected promise: the import is what fails, and reading `default` off the
// module is where the app finds out.
let failImport = null;

vi.mock("hls.js", () => ({
  get default() {
    if (failImport) throw failImport;
    return FakeHls;
  },
}));

vi.mock("./api.js", async (importOriginal) => ({
  clock: (await importOriginal()).clock,
  audioUrl: vi.fn(),
  seekUrl: vi.fn(),
  subtitleUrl: vi.fn(),
  fallbackUrl: vi.fn(),
  playbackFailure: vi.fn(),
  isFullscreen: vi.fn(),
  setFullscreen: vi.fn(),
}));

const api = await import("./api.js");

const PLAYLIST = "http://127.0.0.1:9/hls/token/session/index.m3u8";

function play(overrides = {}) {
  return render(Player, {
    props: {
      source: {
        url: PLAYLIST,
        // Already converted: the first failure is the last one, so the error
        // reaches the screen instead of starting a fallback.
        delivery: "transcode",
        duration: 3600,
        title: "Film",
        video_codec: "hevc",
        subtitles: [],
        audio: [],
        audio_track: 0,
        offset: 0,
        ...overrides,
      },
      relativePath: "Film.mkv",
      onexit: vi.fn(),
    },
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  instances.length = 0;
  // Both are reached into by single tests, and a leaked one would silently
  // change the path every test after it takes.
  FakeHls.isSupported = () => true;
  failImport = null;
  // Put back what the setup file installs: "maybe", the way Chromium answers.
  HTMLMediaElement.prototype.canPlayType = () => "maybe";
  api.playbackFailure.mockResolvedValue(null);
  api.isFullscreen.mockResolvedValue(false);
  api.setFullscreen.mockResolvedValue(false);
  api.fallbackUrl.mockResolvedValue(PLAYLIST);
});

async function attached() {
  await waitFor(() => expect(instances.length).toBe(1));
  return instances[0];
}

describe("which of the two ways the playlist reaches the element", () => {
  it("goes through hls.js on an engine that only claims to play playlists", async () => {
    // The bug this file was written for and still missed. Chromium — WebView2
    // on Windows is the same engine — answers "maybe" to
    // canPlayType("application/vnd.apple.mpegurl") whether or not it can play
    // one, so a test on that answer read as "native HLS" and skipped hls.js.
    // The element then got a `.m3u8` it could do nothing with and failed with
    // no reason attached, which is the sentence the user met on every film
    // that was not already an MP4.
    play();

    const hls = await attached();
    expect(hls.source).toBe(PLAYLIST);
    expect(document.querySelector("video").getAttribute("src")).not.toBe(PLAYLIST);
  });

  it("hands the playlist over untouched when the platform plays HLS itself", async () => {
    // WKWebView does, and there it is the better path: it decodes HEVC in
    // hardware, which is why macOS gets a rewrapped x265 film rather than a
    // re-encoded one. The backend says so, because whether the window is
    // WKWebView is a fact about the platform and not something to ask the
    // engine, which answers "maybe" on both.
    play({ native_hls: true });

    await waitFor(() =>
      expect(document.querySelector("video").getAttribute("src")).toBe(PLAYLIST),
    );
    expect(instances.length).toBe(0);
  });
});

describe("what the window says when hls.js gives up", () => {
  it("names the failure rather than the file, when the backend has nothing to say", async () => {
    // The message the user actually met on Windows: "This file could not be
    // played", which says nothing about what happened and leaves nothing to
    // act on. ffmpeg had not failed, so the backend had no sentence of its
    // own; the reason was in the error hls.js reported and this threw it away.
    play();
    const hls = await attached();

    hls.fail({
      fatal: true,
      type: "networkError",
      details: "manifestLoadError",
      response: { code: 504 },
      url: PLAYLIST,
    });

    const shown = await screen.findByRole("alert");
    expect(shown).toHaveTextContent(/manifestLoadError/);
    expect(shown).toHaveTextContent(/504/);
    expect(shown).toHaveTextContent(/networkError/);
  });

  it("leads with what ffmpeg said and keeps the library's word for it too", async () => {
    // Two different questions, and on a machine nobody here can reach both
    // answers are wanted at once: what was wrong with the film, and where the
    // picture gave up on it.
    api.playbackFailure.mockResolvedValue("Invalid data found when processing input");
    play();
    const hls = await attached();

    hls.fail({ fatal: true, type: "mediaError", details: "bufferAppendError" });

    const shown = await screen.findByRole("alert");
    expect(shown).toHaveTextContent(/^Invalid data found when processing input/);
    expect(shown).toHaveTextContent(/bufferAppendError/);
  });

  it("falls back to the plain sentence when the error carries no detail at all", async () => {
    play();
    const hls = await attached();

    hls.fail({ fatal: true });

    expect(await screen.findByText(/This file could not be played/)).toBeInTheDocument();
  });

  it("says nothing while the failure is not fatal, because those recover by themselves", async () => {
    play();
    const hls = await attached();

    hls.fail({ fatal: false, type: "networkError", details: "fragLoadError" });

    // Long enough for the box to have appeared if it were going to: the error
    // path awaits the backend before it writes anything, so a single microtask
    // shows nothing whether the failure was fatal or not, and this assertion
    // held either way. The fatal failure below is what proves it can still fail
    // this test at all.
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByRole("alert")).toBeNull();

    hls.fail({ fatal: true, type: "networkError", details: "fragLoadError" });
    expect(await screen.findByRole("alert")).toBeInTheDocument();
  });
});

describe("no failure without a reason", () => {
  /// Make the element report the failure it would report on a real engine.
  function elementFails(code, message) {
    const video = document.querySelector("video");
    Object.defineProperty(video, "error", {
      configurable: true,
      value: { code, message },
    });
    video.dispatchEvent(new Event("error"));
  }

  it("prints what the element itself said when nobody else has anything", async () => {
    // `MediaError.message` is empty on WebKit and a sentence on Chromium
    // ("DEMUXER_ERROR_COULD_NOT_OPEN"), which is exactly the machine where
    // this app is hardest to reach. Read nowhere, it left "This file could not
    // be played" as the whole report.
    play();
    await attached();

    elementFails(4, "DEMUXER_ERROR_COULD_NOT_OPEN");

    const shown = await screen.findByRole("alert");
    expect(shown).toHaveTextContent(/DEMUXER_ERROR_COULD_NOT_OPEN/);
    expect(shown).not.toHaveTextContent(/This file could not be played/);
  });

  it("names a code the element gave no words for", async () => {
    play();
    await attached();

    elementFails(3, "");

    expect(await screen.findByRole("alert")).toHaveTextContent(/media error 3/);
  });

  it("says the engine cannot play a playlist at all, rather than blaming the film", async () => {
    // Neither Media Source Extensions nor an element that claims a playlist.
    // Handing the URL over anyway is a dead end that reports itself as a broken
    // film, and the film is fine: it is the window that cannot open it.
    FakeHls.isSupported = () => false;
    HTMLMediaElement.prototype.canPlayType = () => "";
    play();

    const shown = await screen.findByRole("alert");
    expect(shown).toHaveTextContent(/Media Source Extensions/);
    expect(instances.length).toBe(0);
  });

  it("still lets an element that claims playlists try, once hls.js is ruled out", async () => {
    // webkit2gtk can be in exactly that position: no usable MSE, and a
    // GStreamer plugin that opens the playlist anyway. Refusing outright would
    // take away the path that worked there — but this is a last resort, after
    // hls.js, not the guess ahead of it that broke Windows.
    FakeHls.isSupported = () => false;
    play();

    await waitFor(() =>
      expect(document.querySelector("video").getAttribute("src")).toBe(PLAYLIST),
    );
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("converts once when the element it was handed to gives up, and then stops", async () => {
    // The handoff is not a dead end: the element failing is what asks for the
    // conversion, exactly as it did when the URL was handed over unconditionally.
    // And exactly once — on an engine with no MSE the converted stream is
    // another playlist going the same way, so a second failure has to be the
    // last word rather than a third ffmpeg.
    const CONVERTED = "http://127.0.0.1:9/hls/token/second/index.m3u8";
    api.fallbackUrl.mockResolvedValue(CONVERTED);
    FakeHls.isSupported = () => false;
    play({ delivery: "remux" });

    const src = () => document.querySelector("video").getAttribute("src");
    await waitFor(() => expect(src()).toBe(PLAYLIST));
    elementFails(4, "DEMUXER_ERROR_COULD_NOT_OPEN");

    // The second attach is also where `recovering` is dropped, so the failure
    // below is the one being answered rather than the dead stream's last word.
    await waitFor(() => expect(src()).toBe(CONVERTED));
    elementFails(4, "DEMUXER_ERROR_COULD_NOT_OPEN");

    expect(await screen.findByRole("alert")).toHaveTextContent(/DEMUXER_ERROR_COULD_NOT_OPEN/);
    expect(api.fallbackUrl).toHaveBeenCalledTimes(1);
  });

  it("says so when hls.js itself could not be loaded", async () => {
    // A chunk that does not arrive is not a film that cannot be played, and
    // unhandled it left the spinner turning for ever with nothing on screen.
    failImport = new Error("Failed to fetch dynamically imported module");
    play();

    expect(await screen.findByRole("alert")).toHaveTextContent(/dynamically imported module/);
  });

  it("drops the complaint once the picture moves again, because that is it recovering", async () => {
    // Kept for ever it stops describing anything: a fragment that timed out in
    // the first minute would be handed to the user as the reason for a failure
    // an hour later, over a film that had been playing the whole time.
    play();
    const hls = await attached();

    hls.fail({ fatal: false, type: "networkError", details: "fragLoadTimeOut" });
    await fireEvent.playing(document.querySelector("video"));
    elementFails(4, "DEMUXER_ERROR_COULD_NOT_OPEN");

    const shown = await screen.findByRole("alert");
    expect(shown).not.toHaveTextContent(/fragLoadTimeOut/);
    expect(shown).toHaveTextContent(/DEMUXER_ERROR_COULD_NOT_OPEN/);
  });

  it("keeps the last complaint that was not fatal, for the failure that follows it", async () => {
    // A stall recovers by itself, so it says nothing at the time. When the
    // picture dies afterwards it is the only description of where it stopped,
    // and the element's own error carries none.
    play();
    const hls = await attached();

    hls.fail({ fatal: false, type: "networkError", details: "fragLoadTimeOut" });
    elementFails(4, "");

    expect(await screen.findByRole("alert")).toHaveTextContent(/fragLoadTimeOut/);
  });

  it("does not put a dead end over a conversion that is still on its way", async () => {
    // The film that only needed rewrapping fails, so the fallback conversion
    // is asked for: two round trips (the url and the re-cut subtitles), and
    // the hls.js instance for the stream that just died is alive through both
    // of them. It goes on reporting that stream, and by then `converted` is
    // true, so its next fatal error is read as the last word on a film whose
    // second chance has not been given yet: the error box covers a conversion
    // that is about to play, and nothing takes it down again.
    let release;
    api.fallbackUrl.mockReturnValue(new Promise((resolve) => (release = () => resolve(PLAYLIST))));
    play({ delivery: "remux" });
    const hls = await attached();

    document.querySelector("video").dispatchEvent(new Event("error"));
    await waitFor(() => expect(api.fallbackUrl).toHaveBeenCalled());

    hls.fail({ fatal: true, type: "networkError", details: "fragLoadError" });

    // Long enough for the box to have appeared if it were going to: the error
    // path awaits the backend before it writes anything.
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByRole("alert")).toBeNull();
    release();
  });
});

describe("a conversion that is still starting is not a conversion that failed", () => {
  it("gives the first playlist longer than one attempt before calling it lost", async () => {
    // The backend answers 504 until ffmpeg has written the first segment, and
    // on Windows that is a full libx264 encode rather than the `-c copy` macOS
    // gets. One try and out turned slowness into a dead player.
    play();
    const hls = await attached();

    const policy = hls.config?.manifestLoadPolicy?.default;
    expect(policy).toBeTruthy();
    expect(policy.errorRetry.maxNumRetry).toBeGreaterThanOrEqual(4);
    expect(policy.maxLoadTimeMs).toBeGreaterThanOrEqual(30_000);
  });
});
