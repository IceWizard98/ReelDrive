# ReelDrive

A portable video library: one executable and one content folder on a USB stick.
No installation, no server, no network. It reads the filesystem and works out on
its own what is a movie and what is a series.

## Usage

This is all the stick needs:

```
/  (stick root)
├── ReelDrive.app/        # macOS   (or .exe on Windows, a binary on Linux)
└── media/                      # fixed name, you create it
    ├── Scrubs/
    │   ├── cover.jpg           # optional
    │   ├── Season 1/  Scrubs S01E01 - My First Day.mkv …
    │   └── S02/       …
    ├── How I Met Your Mother/  # single-season series: episodes in the root
    │   └── HIMYM S01E01.mkv …
    └── Inception (2010)/       # movie: a single video
        └── Inception.mkv
```

If `media/` is missing, the app says so and explains what to do. Content without
a cover gets a generated tile with its initials. There is no configuration file
to write.

The only file the app creates is `media/.reeldrive-cache.json`: the library
index — hidden, deletable at any time, and what makes later starts instant. On
read-only media the app still works, it just rescans every time.

It is the only file the app deletes, too. A cache it cannot use — damaged, or
written by a different release — is removed rather than left behind, and so is
one left under a name an older version used. The first start after an update
therefore rescans the stick once; the ones after it are instant again.

## How content is classified

| What the folder contains | How it is read |
|---|---|
| subfolders like `Season 1` / `S02` / `Stagione 3` | multi-season series |
| two or more videos in the root, no season folders | single-season series |
| one video carrying an `S01E01` marker | series with one episode |
| one video | movie |
| no video | skipped, reported on the home screen |

`Specials` and `Extras` become season 0. Episode numbers are read from `S01E02`,
`1x02`, `Ep 04`, `E05`, or a leading number; if any file in a season lacks one,
the whole season is numbered in natural order (`ep2` before `ep10`) so numbers
can never collide. External subtitles (`.srt`, `.ass`, …) are matched to the
video sharing their name.

## Playback

Video plays inside the app window, in the webview's own `<video>` element, with
`ffmpeg` in front of the file when the format needs it — the model Jellyfin
uses. What the platform can already decode is served untouched; an MKV whose
streams are playable is rewrapped, which costs milliseconds; only a codec the
platform genuinely cannot decode is converted. The player shows which of the
four it is doing.

**`ffmpeg` and `ffprobe` travel with the app.** Every release archive carries a
static pair, so a stick works on a machine that has never seen ffmpeg. On macOS
they live inside the bundle (`ReelDrive.app/Contents/MacOS/`), where
tidying up the stick cannot separate them from the app; on Windows and Linux
they sit next to the executable. Failing that, the folder the app was launched
from is searched, then `PATH` — so a hand-placed pair still works, and a
developer machine needs nothing. Without them nothing plays.

A pair copied from a package manager is not a substitute: those are linked
against libraries that exist only on the machine that installed them.
`scripts/fetch-ffmpeg.sh` downloads the static builds the releases use, each
pinned to an immutable URL and checked against a recorded SHA256:

```sh
scripts/fetch-ffmpeg.sh macos-arm64 path/to/ReelDrive.app/Contents/MacOS
```

**Audio language.** A film carrying more than one audio track gets an Audio
menu; picking a language rebuilds the stream from the same second, because only
one track travels and the element has no working way to choose between several
once they have arrived. The treatment is decided again for the track chosen, not
inherited: a second language is often in a codec the first one is not, and a
track that has to be converted forces conversion even where the first played
untouched. For the same reason, choosing anything but the first track rules out
handing the file over as it is.

A file served untouched is seekable by the element itself, over byte ranges.
Anything `ffmpeg` touches is delivered as HLS instead: a playlist of fragmented
MP4 segments written to a temporary folder, which is what WebKit will actually
play — it refuses a `<video>` served as an endless stream with no length and no
ranges. The playlist grows as `ffmpeg` writes, so seeking inside what has been
produced is native too; only a jump past it restarts the conversion there.
WebKit reads the playlist itself, and `hls.js` covers the engines that do not.

## Development

```sh
npm install
REELDRIVE_MEDIA=/path/to/media npm run tauri dev
make test          # Rust and frontend
make check         # tests, clippy, formatting, frontend build
```

The Rust tests cover the decisions — classification, delivery, path validation,
the arguments handed to ffmpeg — and the frontend tests cover the player and the
grid: which menus appear, what a chosen audio track asks the backend for, what a
cover-less folder falls back to. `npm run test:watch` reruns them as you type.

`REELDRIVE_MEDIA` exists for development only, because under `tauri dev`
the executable lives in `target/debug/`. A released build always looks for
`media/` next to itself, and nowhere else.

To see what the app would read from a folder without opening any window:

```sh
cd src-tauri && cargo run --example scan -- /path/to/media
```

`http://localhost:1420/preview.html` runs the real interface against stub data
in a plain browser — useful for working on the design without building the app.
Append a title to open its detail view directly, e.g. `preview.html#Scrubs`.

### Architecture

Hexagonal, so the classification rules can be tested against in-memory trees
rather than real files:

- `src-tauri/src/core/` — model, filename parsing, scanner, path validation. No
  knowledge of `std::fs`, ffmpeg, or Tauri.
- `src-tauri/src/ports/` — the `FileSystem` and `LibraryCache` traits.
- `src-tauri/src/adapters/` — `std::fs`, the JSON cache, ffmpeg, and the
  loopback HTTP server that feeds `<video>`.
- `src-tauri/src/defaults.rs` — every convention in one place: folder name,
  extensions, cover names.

### Building

Nothing is linked in: the app talks to ffmpeg as a process, so the build needs
no media libraries.

```sh
make build                      # leaves it in src-tauri/target/release
make build DEST=/Volumes/STICK  # and copies it onto the stick
```

`just build` and `just build /Volumes/STICK` do the same; both wrap the same
steps and pick the right ffmpeg for the machine they run on.

The build always finishes the app **where it is made** — the binary plus the
two tools beside it — so what sits under `src-tauri/target/release` is already
the whole product. That is what to open while working on it; naming a
destination as well would only leave a second copy of a hundred-odd megabytes
to keep straight. A destination is for the stick: copy it there, add a `media/`
folder, and there is nothing to install.

The three binaries **cannot be built from one machine**: each needs its own
platform's webview toolchain, and cross-compiling is not practical.
`.github/workflows/release.yml` handles it — publishing a release builds on four
runners (macOS arm64, macOS Intel, Windows, Linux) and attaches the archives to
that release. To rehearse it, run the workflow manually from Actions with an
empty tag: it builds and uploads artifacts without touching any release.

Each archive carries its platform's static `ffmpeg` and `ffprobe`, fetched by
`scripts/fetch-ffmpeg.sh` during the build, which is what makes the macOS
archives around 130 MB. To move to a newer ffmpeg, replace the URL and its
checksum in that script — the two are meant to be changed together.

### The index cache

The scan is remembered in `.reeldrive-cache.json`, hidden inside the media
folder, so a stick full of titles opens at once instead of being walked again.
It is an optimisation and never a source of truth: missing, unreadable or
outdated all mean the same thing — scan again.

Every binary carries a build number, set by the release workflow from its run
counter and 0 for anything built locally, and the cache records the one that
wrote it. A cache from any other build is discarded. That is deliberately
blunter than it needs to be: bumping a format version by hand is exactly the
step that gets forgotten, and a cache read back into a shape that no longer
means the same thing is a bug nobody can see. The cost is one walk of the stick
after each update.

Anything unreadable is **deleted**, not merely ignored, along with cache files
left under names earlier releases used. Otherwise every rename and every format
change would leave one more hidden file on the user's stick for good.

## Platform notes

**macOS.** The bundle is not signed with a Developer ID, so Gatekeeper blocks
the first launch from an external volume: right-click → Open, or
`xattr -dr com.apple.quarantine ReelDrive.app`.

**Windows.** `ffmpeg.exe` and `ffprobe.exe` come out of the archive next to
`ReelDrive.exe` and have to stay there. SmartScreen will warn about an
unknown app (no signature): "More info" → "Run anyway".

**Linux.** `ffmpeg` and `ffprobe` sit next to the AppImage and have to stay
there: an AppImage runs from a temporary read-only mount, so the app follows
`$APPIMAGE` back to the file itself to find both them and `media/`.

The AppImage is built on Ubuntu 24.04 and therefore needs glibc ≥
2.39, so a distribution from 2024 onwards. It must be executable (`chmod +x`), which is a
problem if the stick is formatted FAT32 or exFAT: those filesystems have no
execute bit, and depending on the mount options the AppImage may refuse to run
from there. An ext4 stick avoids this.
