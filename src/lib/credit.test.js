import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "../App.svelte";

// The whole bridge, so the app can be mounted without Tauri underneath it.
// The pure formatters come through as themselves: they touch nothing outside
// this module, and a second copy written here would let the two drift while
// every test went on passing.
vi.mock("./api.js", async (importOriginal) => ({
  clock: (await importOriginal()).clock,
  getLibrary: vi.fn(),
  getContent: vi.fn(),
  getPlaybackSource: vi.fn(),
  stopStream: vi.fn(),
  openAuthorSite: vi.fn(),
  getProgress: vi.fn(),
  getUpNext: vi.fn(),
  getNextAfter: vi.fn(),
  recordProgress: vi.fn(),
  takeUp: vi.fn(),
  // The player reaches for its own half of the bridge the moment it mounts,
  // and one of these tests takes the app all the way into a film.
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
  contents: [{ id: "Dune (2021)", title: "Dune", year: 2021, kind: "movie", cover: null }],
  warnings: [],
};

const MOVIE = {
  summary: LIBRARY.contents[0],
  body: { kind: "movie", file: "Dune (2021)/Dune.mkv", subtitles: [] },
};

const SOURCE = {
  url: "http://127.0.0.1:9/stream?path=Dune.mkv",
  delivery: "direct",
  duration: 3600,
  title: "Dune",
  video_codec: "h264",
  subtitles: [],
  audio: [],
  audio_track: 0,
  offset: 0,
  bitmap_subtitles: 0,
};

beforeEach(() => {
  vi.clearAllMocks();
  api.getLibrary.mockResolvedValue(LIBRARY);
  api.openAuthorSite.mockResolvedValue(null);
  api.getContent.mockResolvedValue(MOVIE);
  api.getPlaybackSource.mockResolvedValue(SOURCE);
  api.playbackFailure.mockResolvedValue(null);
  api.isFullscreen.mockResolvedValue(false);
  api.getProgress.mockResolvedValue({});
  api.getUpNext.mockResolvedValue(null);
  api.getNextAfter.mockResolvedValue(null);
  api.recordProgress.mockResolvedValue(null);
  api.takeUp.mockResolvedValue(null);
});

describe("the credit", () => {
  it("names the author and shows the address", async () => {
    render(App);
    await screen.findByRole("button", { name: "Luis Enriquez" });
    // Written out as well as linked: this app is built for machines with no
    // network, where the browser may not open at all, and an address you can
    // read is one you can type somewhere else.
    expect(screen.getByText("luise.ac")).toBeInTheDocument();
  });

  it("hands the opening to the backend rather than navigating the window", async () => {
    // A plain link would take the webview itself to the site, and the app has
    // no way back — there is no address bar and no history to speak of.
    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: "Luis Enriquez" }));
    await waitFor(() => expect(api.openAuthorSite).toHaveBeenCalledTimes(1));
    // No argument: the address lives in the backend, so nothing on this side
    // can ask it to open something else.
    expect(api.openAuthorSite).toHaveBeenCalledWith();
  });

  it("survives a machine with nothing to open a browser with", async () => {
    // The one case this has to not turn into a broken app: no desktop session,
    // no default browser, a stick plugged into something minimal.
    api.openAuthorSite.mockRejectedValue("no handler");
    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: "Luis Enriquez" }));
    await waitFor(() => expect(api.openAuthorSite).toHaveBeenCalled());
    expect(screen.getByText("luise.ac")).toBeInTheDocument();
  });

  it("says so when the click opened nothing", async () => {
    // Surviving is not enough. The button is the only thing on screen that
    // answers a press, and on a machine with no browser it answers with
    // nothing at all — a dead control, indistinguishable from a broken app,
    // and the user is never told that the printed address is now their only
    // way to the site.
    api.openAuthorSite.mockRejectedValue("no handler");
    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: "Luis Enriquez" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/browser/i);
  });

  it("is not over the film", async () => {
    // A credit line across the bottom of a picture is the one place it must
    // never be, and it is the guard a later hand is most likely to drop.
    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: /Dune/ }));
    await fireEvent.click(await screen.findByRole("button", { name: /^Play/ }));
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Luis Enriquez" })).not.toBeInTheDocument(),
    );
  });

  it("is on the welcome screen too, where nothing has been set up yet", async () => {
    // A stick nobody has filled in is exactly when someone wonders what this
    // is and who wrote it.
    api.getLibrary.mockResolvedValue({ ...LIBRARY, media_root_exists: false, contents: [] });
    render(App);
    await screen.findByText(/Content folder missing/);
    expect(screen.getByRole("button", { name: "Luis Enriquez" })).toBeInTheDocument();
  });
});

describe("the version", () => {
  // Every case states the variable, including the one that wants it absent: the
  // release workflow exports it for the whole job, so the suite itself runs
  // with a tag in the environment and a test that just reads what is there
  // would fail on exactly the builds that matter. Unstubbed here rather than at
  // the end of each test, so a failing assertion cannot leak a tag into the
  // next one.
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it("says dev when no release stamped one in", async () => {
    // A build off a laptop is not a release and must not read like one.
    vi.stubEnv("VITE_REELDRIVE_VERSION", undefined);
    render(App);
    await screen.findByRole("button", { name: "Luis Enriquez" });
    expect(screen.getByText("dev")).toBeInTheDocument();
  });

  it("says dev when the tag came through empty", async () => {
    // A dispatch with no tag leaves the variable set and empty, which is not
    // the same input as an absent one.
    vi.stubEnv("VITE_REELDRIVE_VERSION", "");
    render(App);
    await screen.findByRole("button", { name: "Luis Enriquez" });
    expect(screen.getByText("dev")).toBeInTheDocument();
  });

  it("shows the tag the release was built from", async () => {
    vi.stubEnv("VITE_REELDRIVE_VERSION", "v1.2.0");
    render(App);
    await screen.findByRole("button", { name: "Luis Enriquez" });
    expect(screen.getByText("v1.2.0")).toBeInTheDocument();
  });

  it("prefixes a bare number, so the footer never reads as a date", async () => {
    vi.stubEnv("VITE_REELDRIVE_VERSION", "0.3.1");
    render(App);
    await screen.findByRole("button", { name: "Luis Enriquez" });
    expect(screen.getByText("v0.3.1")).toBeInTheDocument();
  });

  it("stands before the credit", async () => {
    // Asked for explicitly: the number is what the line opens with, and the
    // name follows it.
    vi.stubEnv("VITE_REELDRIVE_VERSION", "v1.2.0");
    render(App);
    const footer = (await screen.findByRole("button", { name: "Luis Enriquez" })).closest("footer");
    expect(footer.textContent.replace(/\s+/g, " ")).toMatch(/v1\.2\.0 · Made by Luis Enriquez/);
  });
});
