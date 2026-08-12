import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "../App.svelte";

// The whole bridge, so the app can be mounted without Tauri underneath it.
vi.mock("./api.js", () => ({
  getLibrary: vi.fn(),
  getContent: vi.fn(),
  getPlaybackSource: vi.fn(),
  stopStream: vi.fn(),
  openAuthorSite: vi.fn(),
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

beforeEach(() => {
  vi.clearAllMocks();
  api.getLibrary.mockResolvedValue(LIBRARY);
  api.openAuthorSite.mockResolvedValue(null);
});

describe("the credit", () => {
  it("names the author and shows the address", async () => {
    render(App);
    await screen.findByRole("button", { name: "IceWizard" });
    // Written out as well as linked: this app is built for machines with no
    // network, where the browser may not open at all, and an address you can
    // read is one you can type somewhere else.
    expect(screen.getByText("luise.ac")).toBeInTheDocument();
  });

  it("hands the opening to the backend rather than navigating the window", async () => {
    // A plain link would take the webview itself to the site, and the app has
    // no way back — there is no address bar and no history to speak of.
    render(App);
    await fireEvent.click(await screen.findByRole("button", { name: "IceWizard" }));
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
    await fireEvent.click(await screen.findByRole("button", { name: "IceWizard" }));
    await waitFor(() => expect(api.openAuthorSite).toHaveBeenCalled());
    expect(screen.getByText("luise.ac")).toBeInTheDocument();
  });

  it("is on the welcome screen too, where nothing has been set up yet", async () => {
    // A stick nobody has filled in is exactly when someone wonders what this
    // is and who wrote it.
    api.getLibrary.mockResolvedValue({ ...LIBRARY, media_root_exists: false, contents: [] });
    render(App);
    await screen.findByText(/Content folder missing/);
    expect(screen.getByRole("button", { name: "IceWizard" })).toBeInTheDocument();
  });
});
