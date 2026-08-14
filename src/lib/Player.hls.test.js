import { render, screen, waitFor } from "@testing-library/svelte";
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

vi.mock("hls.js", () => ({ default: FakeHls }));

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
  api.playbackFailure.mockResolvedValue(null);
  api.isFullscreen.mockResolvedValue(false);
  api.setFullscreen.mockResolvedValue(false);
  api.fallbackUrl.mockResolvedValue(PLAYLIST);
});

async function attached() {
  await waitFor(() => expect(instances.length).toBe(1));
  return instances[0];
}

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
