import { describe, expect, it, vi } from "vitest";

// The module reaches for Tauri's IPC at import time, which does not exist in a
// plain browser. Only the pure helpers below are under test here.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (path) => `asset://localhost/${encodeURIComponent(path)}`,
}));

const { invoke } = await import("@tauri-apps/api/core");
const api = await import("./api.js");
const { initials, placeholderStyle, fileUrl } = api;

describe("the calls into the backend", () => {
  // The one place the two languages have to agree. Every other test mocks
  // `./api.js` whole, so nothing else ever sees these names: rename a command
  // or one of its arguments on either side and the app throws at runtime with
  // a green suite behind it. The expected values are the `#[tauri::command]`
  // signatures in src-tauri/src/lib.rs — Tauri matches a snake_case Rust
  // argument to a camelCase key, which is why `video_codec` is sent as
  // `videoCodec`. Arguments left out are the ones declared `Option<_>`.
  it("names every command and argument the way Rust declares it", () => {
    const cases = [
      [() => api.getLibrary(), "library", undefined],
      [() => api.getContent("Dune (2021)"), "content", ["id"]],
      [() => api.getPlaybackSource("Film.mkv", []), "playback_source", ["path", "external"]],
      [() => api.audioUrl("Film.mkv", 90, 1), "audio_url", ["path", "seconds", "audio"]],
      [
        () => api.seekUrl("Film.mkv", 90, "remux", "h264", 1),
        "seek_url",
        ["path", "seconds", "delivery", "videoCodec", "audio"],
      ],
      [() => api.subtitleUrl("Film.mkv", 0, 90), "subtitle_url", ["path", "track", "seconds"]],
      [() => api.fallbackUrl("Film.mkv", 90, 1), "fallback_url", ["path", "seconds", "audio"]],
      [() => api.stopStream(), "stop_playback", undefined],
    ];

    for (const [call, command, keys] of cases) {
      invoke.mockClear();
      call();
      expect(invoke).toHaveBeenCalledTimes(1);
      const [name, args] = invoke.mock.calls[0];
      expect(name).toBe(command);
      expect(args === undefined ? undefined : Object.keys(args).sort()).toEqual(
        keys && [...keys].sort(),
      );
    }
  });
});

describe("initials", () => {
  it("takes the first letter of the first two words", () => {
    expect(initials("Blade Runner 2049")).toBe("BR");
    expect(initials("Dune")).toBe("D");
  });

  it("ignores words that carry no letter or digit", () => {
    // Punctuation between words is common in folder names, and a tile reading
    // "-" instead of a letter looks like a bug rather than a placeholder.
    expect(initials("Spider-Man - Homecoming")).toBe("SH");
    expect(initials("... Dune")).toBe("D");
  });

  it("reads letters outside ASCII", () => {
    expect(initials("Émile Zola")).toBe("ÉZ");
    expect(initials("Ödipus")).toBe("Ö");
  });

  it("gives nothing back rather than throwing on an empty title", () => {
    expect(initials("")).toBe("");
    expect(initials("   ")).toBe("");
    expect(initials("---")).toBe("");
  });
});

describe("placeholderStyle", () => {
  it("gives the same title the same colour every time", () => {
    expect(placeholderStyle("Scrubs")).toBe(placeholderStyle("Scrubs"));
  });

  it("stays inside the hue circle", () => {
    for (const title of ["", "A", "Scrubs", "How I Met Your Mother", "日本語"]) {
      const hue = Number(placeholderStyle(title).replace("--tile-hue: ", ""));
      expect(Number.isFinite(hue)).toBe(true);
      expect(hue).toBeGreaterThanOrEqual(0);
      expect(hue).toBeLessThan(360);
    }
  });
});

describe("fileUrl", () => {
  it("joins the media root and a relative path", () => {
    expect(fileUrl("/mnt/usb/media", "Dune/cover.jpg")).toContain("%2Fmnt%2Fusb%2Fmedia%2FDune%2Fcover.jpg");
  });

  it("normalises Windows separators and trailing slashes", () => {
    // Paths from the backend always use "/", so a Windows root has to be
    // brought to the same shape before joining or the two halves disagree.
    const windows = fileUrl("C:\\Media\\", "Dune/cover.jpg");
    expect(windows).toContain("C%3A%2FMedia%2FDune%2Fcover.jpg");
    expect(fileUrl("/mnt/usb/media///", "Dune/cover.jpg")).toContain("media%2FDune");
  });

  it("has no URL for content with no cover", () => {
    // Most folders have no cover, so this is the ordinary case, not an error.
    expect(fileUrl("/mnt/usb/media", null)).toBeNull();
    expect(fileUrl("/mnt/usb/media", undefined)).toBeNull();
    expect(fileUrl("/mnt/usb/media", "")).toBeNull();
  });
});
