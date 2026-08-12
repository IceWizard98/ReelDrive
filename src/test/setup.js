import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

// jsdom ships no media engine: `play`, `load` and `canPlayType` are declared on
// HTMLMediaElement and then throw "not implemented". The player calls all three
// on the way to its first frame, so without these every test of it fails on the
// engine rather than on the component.
Object.defineProperty(HTMLMediaElement.prototype, "play", {
  configurable: true,
  value: vi.fn().mockResolvedValue(undefined),
});

Object.defineProperty(HTMLMediaElement.prototype, "load", {
  configurable: true,
  value: vi.fn(),
});

Object.defineProperty(HTMLMediaElement.prototype, "canPlayType", {
  configurable: true,
  value: () => "",
});

// Read at module load to decide whether HLS needs hls.js. Answering "" — no
// native HLS — is what a Chromium-like engine answers, and it keeps the tests
// off the playlist path unless one asks for it.
Object.defineProperty(HTMLMediaElement.prototype, "seekable", {
  configurable: true,
  get: () => ({ length: 0, end: () => 0 }),
});

Object.defineProperty(HTMLMediaElement.prototype, "buffered", {
  configurable: true,
  get: () => ({ length: 0, end: () => 0 }),
});

// jsdom does not lay anything out, so it has no scrolling either. The grid
// keeps the focused tile in view with this on every arrow key.
Object.defineProperty(Element.prototype, "scrollIntoView", {
  configurable: true,
  value: vi.fn(),
});

// The player asks for fullscreen from a button; jsdom has no such thing.
Object.defineProperty(Element.prototype, "requestFullscreen", {
  configurable: true,
  value: vi.fn().mockResolvedValue(undefined),
});

Object.defineProperty(document, "exitFullscreen", {
  configurable: true,
  value: vi.fn().mockResolvedValue(undefined),
});
