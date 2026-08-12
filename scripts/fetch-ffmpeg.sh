#!/bin/sh
# Download the ffmpeg and ffprobe the app ships with.
#
# The app runs them as separate processes, so they have to be there. A copy
# taken from a package manager is not portable: those are linked against
# libraries that exist on the machine that installed them and nowhere else.
# The builds fetched here are static — system libraries only — so they run on
# a bare machine, which is the whole point of a stick you can carry.
#
# Every archive is pinned to an immutable URL and checked against a SHA256
# recorded here. A mismatch stops the script: an archive that changed under a
# fixed URL is not something to unpack and run.
#
# Usage: scripts/fetch-ffmpeg.sh <target> <destination-directory>
#   targets: macos-arm64 macos-x86_64 windows-x86_64 linux-x86_64
#
# To refresh a build, replace the URL and its checksum below. The macOS builds
# come from ffmpeg.martin-riedl.de (snapshot paths are immutable), the others
# from BtbN/FFmpeg-Builds (dated release tags are immutable). Both are GPL
# builds and carry libx264, which the transcode path needs.
set -eu

# --- the pinned builds -------------------------------------------------------

MACOS_ARM64_BASE="https://ffmpeg.martin-riedl.de/download/macos/arm64/1785661721_N-125892-g406c5a37aa"
MACOS_ARM64_FFMPEG_SHA="2d96af0e28b81d5215d8b9a6dec4e751b5b97408205ccfae5760084fa107d936"
MACOS_ARM64_FFPROBE_SHA="0e60dca1df67bffc0b232d54e9833de3508787af6822aec60233dd4f2687645e"

MACOS_X86_64_BASE="https://ffmpeg.martin-riedl.de/download/macos/amd64/1767299902_N-122320-g38e89fe502"
MACOS_X86_64_FFMPEG_SHA="6f0736d424b7426f8cbb7bba5c5448d94c976c138c669593b1f40ba534ed192f"
MACOS_X86_64_FFPROBE_SHA="46852ce00ae6f722ed30be269f310ea96c106a4246f2905d3147c5e02293081c"

BTBN_BASE="https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-08-11-13-11"
WINDOWS_X86_64_ARCHIVE="ffmpeg-n8.1.2-34-g9b6c8969e0-win64-gpl-8.1.zip"
WINDOWS_X86_64_SHA="05eedc113542be39af5d0f78f0b1093bafb89c98cecf25b77e8644670293107f"
LINUX_X86_64_ARCHIVE="ffmpeg-n8.1.2-34-g9b6c8969e0-linux64-gpl-8.1.tar.xz"
LINUX_X86_64_SHA="dbe4b5d3c6b31ad4078736a8b440f4e1939bcf53453194dc156aec34f837f19a"

# --- plumbing ----------------------------------------------------------------

usage() {
  echo "usage: $0 <macos-arm64|macos-x86_64|windows-x86_64|linux-x86_64> <destination>" >&2
  exit 2
}

[ $# -eq 2 ] || usage
TARGET="$1"
DEST="$2"

for tool in curl unzip tar; do
  command -v "$tool" >/dev/null || { echo "$0: $tool is not installed" >&2; exit 1; }
done

# Downloads are kept so a second run costs nothing. The directory is ignored
# by git.
CACHE="${FFMPEG_CACHE:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)/.ffmpeg-cache}"
mkdir -p "$CACHE" "$DEST"

sha256_of() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# Fetch into the cache and refuse to go on unless the bytes are the expected
# ones. A cached file that no longer matches is treated as damaged, not as an
# update: it is removed and fetched again once.
fetch() {
  url="$1"
  expected="$2"
  out="$CACHE/$3"

  if [ -f "$out" ] && [ "$(sha256_of "$out")" != "$expected" ]; then
    echo "  cached copy does not match its checksum, fetching again"
    rm -f "$out"
  fi

  if [ ! -f "$out" ]; then
    echo "  $url"
    curl -fsSL --retry 3 --connect-timeout 30 -o "$out.part" "$url"
    mv "$out.part" "$out"
  fi

  actual=$(sha256_of "$out")
  if [ "$actual" != "$expected" ]; then
    rm -f "$out"
    echo "$0: checksum mismatch for $url" >&2
    echo "  expected $expected" >&2
    echo "  got      $actual" >&2
    exit 1
  fi
}

# The macOS builds ship one bare binary per zip.
fetch_macos() {
  base="$1"
  ffmpeg_sha="$2"
  ffprobe_sha="$3"
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT

  for tool in ffmpeg ffprobe; do
    case "$tool" in
      ffmpeg) sha="$ffmpeg_sha" ;;
      ffprobe) sha="$ffprobe_sha" ;;
    esac
    fetch "$base/$tool.zip" "$sha" "$TARGET-$tool.zip"
    unzip -o -q "$CACHE/$TARGET-$tool.zip" -d "$work"
    mv "$work/$tool" "$DEST/$tool"
    chmod +x "$DEST/$tool"
  done
}

# The BtbN archives carry a whole tree; only the two binaries in bin/ are kept.
fetch_btbn() {
  archive="$1"
  sha="$2"
  suffix="$3"
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT

  fetch "$BTBN_BASE/$archive" "$sha" "$archive"
  case "$archive" in
    *.zip) unzip -o -q "$CACHE/$archive" -d "$work" ;;
    *.tar.xz) tar -xJf "$CACHE/$archive" -C "$work" ;;
    *) echo "$0: unexpected archive type: $archive" >&2; exit 1 ;;
  esac

  for tool in "ffmpeg$suffix" "ffprobe$suffix"; do
    found=$(find "$work" -type f -name "$tool" -print | head -n 1)
    [ -n "$found" ] || { echo "$0: $tool not found in $archive" >&2; exit 1; }
    mv "$found" "$DEST/$tool"
    chmod +x "$DEST/$tool"
  done
}

echo "fetching ffmpeg and ffprobe for $TARGET"
case "$TARGET" in
  macos-arm64)
    fetch_macos "$MACOS_ARM64_BASE" "$MACOS_ARM64_FFMPEG_SHA" "$MACOS_ARM64_FFPROBE_SHA"
    ;;
  macos-x86_64)
    fetch_macos "$MACOS_X86_64_BASE" "$MACOS_X86_64_FFMPEG_SHA" "$MACOS_X86_64_FFPROBE_SHA"
    ;;
  windows-x86_64)
    fetch_btbn "$WINDOWS_X86_64_ARCHIVE" "$WINDOWS_X86_64_SHA" ".exe"
    ;;
  linux-x86_64)
    fetch_btbn "$LINUX_X86_64_ARCHIVE" "$LINUX_X86_64_SHA" ""
    ;;
  *)
    usage
    ;;
esac

# Being able to run it is the only proof that matters. On macOS an unsigned
# downloaded binary is also quarantined, so this is where that shows up.
case "$TARGET" in
  macos-*) [ "$(uname -s)" = "Darwin" ] || { echo "done (not run: host is not macOS)"; exit 0; } ;;
  windows-*) case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) ;; *) echo "done (not run: host is not Windows)"; exit 0 ;; esac ;;
  linux-*) [ "$(uname -s)" = "Linux" ] || { echo "done (not run: host is not Linux)"; exit 0; } ;;
esac

for tool in ffmpeg ffprobe; do
  binary="$DEST/$tool"
  [ -f "$binary" ] || binary="$DEST/$tool.exe"
  "$binary" -version | head -n 1
done
