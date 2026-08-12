// Runs the real UI in a plain browser by faking the Tauri bridge, so the design
// can be reviewed (and screenshotted) without building the app.
//
//   npm run dev  ->  http://localhost:1420/preview.html
//
// Every component, store and style is the production one; only the data and the
// two Tauri entry points are stubbed.
import { mount } from "svelte";
import App from "./App.svelte";

const POSTERS = {};

// The harness's own fixtures, under `public/` and gitignored — not in
// `media/`, which is the user's real library and not a place for a test
// clip to squat. Generate them with:
//
//   mkdir -p public
//   ffmpeg -f lavfi -i testsrc2=size=1280x720:rate=25:duration=60 \
//          -f lavfi -i sine=frequency=220:duration=60 \
//          -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest public/clip.mp4
//
// Without them the player runs in its error state, which is also worth
// looking at.
const CLIP = "/clip.mp4";
const CLIP_VTT = "/clip.vtt";

const svg = (label, from, to) =>
  "data:image/svg+xml;charset=utf-8," +
  encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 600">
      <defs><linearGradient id="g" x1="0" y1="0" x2="0.35" y2="1">
        <stop offset="0" stop-color="${from}"/><stop offset="1" stop-color="${to}"/>
      </linearGradient></defs>
      <rect width="400" height="600" fill="url(#g)"/>
      <text x="200" y="548" text-anchor="middle" fill="#ffffffd9"
            font-family="Helvetica,Arial" font-size="28" font-weight="700"
            letter-spacing="1.5">${label}</text>
    </svg>`,
  );

// Covers travel through fileUrl(mediaRoot, relative) exactly like real ones:
// the stub stores a relative path and resolves it back in convertFileSrc.
const cover = (id, label, from, to) => {
  const relative = `${id}/cover.svg`;
  POSTERS[relative] = svg(label, from, to);
  return relative;
};

const CONTENTS = [
  { id: "Arrival (2016)", title: "Arrival", year: 2016, kind: "movie", cover: cover("Arrival (2016)", "ARRIVAL", "#1d3b53", "#0a1720") },
  { id: "Blade Runner 2049", title: "Blade Runner 2049", year: null, kind: "movie", cover: cover("Blade Runner 2049", "2049", "#8a3b12", "#2a1004") },
  { id: "Chernobyl", title: "Chernobyl", year: 2019, kind: "series", cover: cover("Chernobyl", "CHERNOBYL", "#3f4a2f", "#14180f") },
  { id: "Dune (2021)", title: "Dune", year: 2021, kind: "movie", cover: null },
  { id: "How I Met Your Mother", title: "How I Met Your Mother", year: null, kind: "series", cover: cover("How I Met Your Mother", "HIMYM", "#4a2f6b", "#170f22") },
  { id: "Inception (2010)", title: "Inception", year: 2010, kind: "movie", cover: cover("Inception (2010)", "INCEPTION", "#1b2a4a", "#080d18") },
  { id: "Il Padrino (1972)", title: "Il Padrino", year: 1972, kind: "movie", cover: null },
  { id: "Mr. Robot", title: "Mr. Robot", year: 2015, kind: "series", cover: cover("Mr. Robot", "MR ROBOT", "#2b0f0f", "#0d0505") },
  { id: "Scrubs", title: "Scrubs", year: 2001, kind: "series", cover: cover("Scrubs", "SCRUBS", "#1f4a3f", "#0a1a16") },
  { id: "Severance", title: "Severance", year: 2022, kind: "series", cover: cover("Severance", "SEVERANCE", "#123a4a", "#06161c") },
  { id: "The Bear", title: "The Bear", year: 2022, kind: "series", cover: null },
  { id: "The Matrix (1999)", title: "The Matrix", year: 1999, kind: "movie", cover: cover("The Matrix (1999)", "MATRIX", "#0f2e18", "#04120a") },
  { id: "Twin Peaks", title: "Twin Peaks", year: 1990, kind: "series", cover: cover("Twin Peaks", "TWIN PEAKS", "#4a2416", "#160a06") },
  { id: "Whiplash (2014)", title: "Whiplash", year: 2014, kind: "movie", cover: cover("Whiplash (2014)", "WHIPLASH", "#5c4409", "#1a1303") },
];

const episode = (number, title, subs = 0) => ({
  number,
  title,
  file: `Scrubs/Season 1/S01E${String(number).padStart(2, "0")}.mkv`,
  subtitles: subs ? ["Scrubs/Season 1/sub.srt"] : [],
});

const find = (id) => CONTENTS.find((content) => content.id === id);

const DETAILS = {
  Scrubs: {
    summary: find("Scrubs"),
    body: {
      kind: "series",
      seasons: [
        { number: 0, title: "Specials", episodes: [episode(1, "Behind the scenes")] },
        {
          number: 1,
          title: "Season 1",
          episodes: [
            episode(1, "My First Day", 1),
            episode(2, "My Mentor", 1),
            episode(3, ""),
            episode(4, "My Old Lady"),
            episode(5, "My Two Dads", 1),
            episode(6, "My Bad"),
            episode(7, "My Super Ego"),
            episode(8, "My Fifteen Minutes"),
          ],
        },
        { number: 2, title: "S02", episodes: [episode(1, "My Overkill"), episode(2, "My Nightingale")] },
      ],
    },
  },
  "Inception (2010)": {
    summary: find("Inception (2010)"),
    body: {
      kind: "movie",
      file: "Inception (2010)/Inception.mkv",
      subtitles: ["Inception (2010)/Inception.it.srt"],
    },
  },
  "The Bear": {
    summary: find("The Bear"),
    body: {
      kind: "series",
      seasons: [
        {
          number: 1,
          title: "Season 1",
          episodes: [episode(1, "System"), episode(2, "Hands"), episode(3, "Brigade")],
        },
      ],
    },
  },
};

const LIBRARY = {
  media_root: "/Volumes/STICK/media",
  media_root_exists: true,
  contents: CONTENTS,
  warnings: ['“Notes” contains no video', '“To watch” contains no video'],
};

// The two entry points @tauri-apps/api goes through.
window.__TAURI_INTERNALS__ = {
  transformCallback: (callback) => callback,
  convertFileSrc: (path) => POSTERS[path.replace(/^.*\/media\//, "")] ?? path,
  invoke: async (command, args) => {
    await new Promise((resolve) => setTimeout(resolve, 120));
    switch (command) {
      case "library":
        return LIBRARY;
      case "content": {
        const detail = DETAILS[args.id];
        if (!detail) throw `no playable file in: ${args.id}`;
        return detail;
      }
      // Vite serves the repo root, and `media/` is ignored by git: drop a clip
      // there and the player runs on a real <video> instead of its error state.
      //
      //   ffmpeg -f lavfi -i testsrc2=size=1280x720:rate=25:duration=120 \
      //          -f lavfi -i sine=frequency=220:duration=120 \
      //          -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest media/clip.mp4
      case "playback_source":
        return {
          url: CLIP,
          delivery: "remux",
          duration: 2532,
          title: args.path.split("/").pop(),
          video_codec: "h264",
          subtitles: [
            { url: CLIP_VTT, label: "Italiano", language: "it", path: args.path, track: 0 },
            {
              url: CLIP_VTT,
              label: "English (SDH)",
              language: "en",
              path: args.path,
              track: 1,
            },
          ].slice(0, Math.max(args.external.length, 1)),
          audio: [],
          audio_track: 0,
          offset: 0,
          // A Blu-ray rip with its forced track as pictures: the one case the
          // menu now has to explain instead of silently offering nothing.
          bitmap_subtitles: 1,
        };
      case "seek_url":
      case "fallback_url":
        return CLIP;
      // Re-cut to the offset a restarted stream begins at. The stub has one
      // fixed file, but the player must still exercise the call: leaving it out
      // is how a command the app depends on goes unnoticed here.
      case "subtitle_url":
        return CLIP_VTT;
      // Nothing to stop here, but an unknown command throws, and the preview
      // should exercise the same call the app makes on leaving a film.
      case "stop_playback":
        return null;
      // Nothing failed here, and the player has to cope with that answer: it
      // asks on every dead end and falls back to its own sentence.
      case "playback_failure":
        return null;
      default:
        throw `unknown command: ${command}`;
    }
  },
};

const app = mount(App, { target: document.getElementById("app") });

// preview.html#Scrubs opens that content straight away, which is how the detail
// view gets screenshotted without a human clicking. Append `!play` to land in
// the player, and `!play!subs` to land there with a menu open.
if (location.hash) {
  const [wanted, ...steps] = decodeURIComponent(location.hash.slice(1)).split("!");
  const buttons = () => [...document.querySelectorAll("button")];
  // By id, not by text: a tile with no cover starts with its initials, so
  // "Dune" never matched the tile whose caption reads Dune.
  const open = (text) =>
    buttons()
      .find((b) => b.dataset.id?.startsWith(text))
      ?.click();
  const press = (text) => buttons().find((b) => b.textContent.includes(text))?.click();

  // Headless Chrome does not decode video, so the position has to be set by
  // hand for the progress bar to be worth a screenshot. `!at0.35` puts it at
  // 35% of the running time.
  const type = (selector, value) => {
    const input = document.querySelector(selector);
    if (!input) return;
    input.value = value;
    input.dispatchEvent(new Event("input", { bubbles: true }));
  };

  const scrubTo = (fraction) => {
    const input = document.querySelector('input[aria-label="Position"]');
    if (input) type('input[aria-label="Position"]', String(Number(input.max) * fraction));
  };

  // `!key:ArrowDown` sends a key, which is the only way to screenshot what the
  // keyboard paths actually do.
  const sendKey = (key) =>
    (document.activeElement ?? document.body).dispatchEvent(
      new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }),
    );

  let delay = 400;
  if (wanted) setTimeout(() => open(wanted), delay);
  for (const step of steps) {
    delay += 700;
    setTimeout(() => {
      if (step.startsWith("find:")) type('input[type="search"]', step.slice(5));
      else if (step.startsWith("key:")) sendKey(step.slice(4));
      else if (step.startsWith("at")) scrubTo(Number(step.slice(2)));
      else press(step === "play" ? "Play" : step);
    }, delay);
  }
}

export default app;
