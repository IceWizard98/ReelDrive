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

// "maybe" for everything, which is what Chromium answers for a playlist it
// cannot necessarily play — the answer that made the app skip hls.js on
// Windows and hand a `.m3u8` to an element that then failed without a reason.
// A test engine that answered "" would have hidden that, so this one lies the
// same way the real one does.
Object.defineProperty(HTMLMediaElement.prototype, "canPlayType", {
  configurable: true,
  value: () => "maybe",
});

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
