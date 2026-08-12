import { render, screen, fireEvent } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";
import Tile from "./Tile.svelte";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (path) => `asset://localhost/${encodeURIComponent(path)}`,
}));

function content(overrides = {}) {
  return { id: "Dune (2021)", title: "Dune", year: 2021, kind: "movie", cover: null, ...overrides };
}

function tile(overrides = {}) {
  return render(Tile, {
    props: { content: content(overrides), mediaRoot: "/mnt/usb/media", onopen: vi.fn(), index: 0 },
  });
}

describe("the cover", () => {
  it("is the image when the folder has one", () => {
    const { container } = tile({ cover: "Dune (2021)/cover.jpg" });
    const image = container.querySelector("img.art");
    expect(image).not.toBeNull();
    expect(image.getAttribute("src")).toContain("cover.jpg");
  });

  it("falls back to the initials when the folder has none", () => {
    // Most folders have no cover. This is the ordinary look, not an error state.
    const { container } = tile({ cover: null });
    expect(container.querySelector("img.art")).toBeNull();
    expect(screen.getByText("D")).toBeInTheDocument();
  });

  it("falls back to the initials when the image will not load", async () => {
    // A cover.jpg that is truncated, or not an image at all, would otherwise
    // leave a blank rectangle with no hint of which title it is.
    const { container } = tile({ cover: "Dune (2021)/cover.jpg" });
    await fireEvent.error(container.querySelector("img.art"));

    expect(container.querySelector("img.art")).toBeNull();
    expect(screen.getByText("D")).toBeInTheDocument();
  });
});

describe("the caption", () => {
  it("carries the title and the year", () => {
    tile();
    expect(screen.getByText("Dune")).toBeInTheDocument();
    expect(screen.getByText("2021")).toBeInTheDocument();
  });

  it("leaves the year blank rather than printing nothing-at-all", () => {
    // A folder with no year in its name is normal; the caption must not read
    // "undefined" under the poster.
    const { container } = tile({ year: null });
    expect(container.querySelector(".year").textContent).toBe("");
  });
});

describe("opening", () => {
  it("hands the whole content back, not just its id", async () => {
    const onopen = vi.fn();
    render(Tile, { props: { content: content(), mediaRoot: "/mnt/usb/media", onopen, index: 0 } });
    await fireEvent.click(screen.getByRole("button"));

    expect(onopen).toHaveBeenCalledWith(expect.objectContaining({ id: "Dune (2021)" }));
  });
});
