// Write the release number into the two files the operating system reads.
//
// The footer's number comes from the tag through `VITE_REELDRIVE_VERSION`, and
// nothing in the tree has to be bumped by hand for it. The number Windows shows
// in the exe's properties, macOS in Info.plist and Linux in the AppImage's name
// does not: it is read out of `src-tauri/tauri.conf.json` and
// `src-tauri/Cargo.toml`, which stay at a placeholder for local builds. This is
// what puts the tag into them, before anything is compiled.
//
// Node rather than a line of shell, because the line of shell was
// `sed -i.bak "0,/^version = /s//.../"`: GNU sed does what that says and BSD
// sed, which is what the macOS runners have, exits 0 and changes nothing. A
// silent no-op on one platform out of four is exactly the failure this project
// keeps finding after a release rather than before one.
//
//     node scripts/stamp-version.js v0.1.1
//
// Node-only: never bundled into the app.

import { readFileSync, writeFileSync } from "node:fs";

/// The number a tag names, or an error naming what was wrong with it.
///
/// Both files take `x.y.z` and nothing else, so a tag this cannot read has to
/// stop the release. An executable carrying no version looks like a build that
/// went wrong, which is worse than one nobody stamped.
export function releaseNumber(tag) {
  const text = tag ?? "";
  const bare = text.startsWith("v") ? text.slice(1) : text;
  if (!/^\d+\.\d+\.\d+$/.test(bare)) {
    throw new Error(`${text || "(empty)"} is not a version this can stamp`);
  }
  return bare;
}

/// `tauri.conf.json` with its version replaced.
export function stampedConfig(source, number) {
  const config = JSON.parse(source);
  if (typeof config.version !== "string") {
    throw new Error("the config has no version to replace");
  }
  config.version = number;
  // Two spaces and a trailing newline: what the file already looks like, so the
  // stamped copy is a one-line diff rather than a reformat.
  return `${JSON.stringify(config, null, 2)}\n`;
}

/// `Cargo.toml` with the package's own version replaced.
///
/// The package's, not every `version = ` in the file: the dependencies below
/// carry the same key, and rewriting those pins every one of them to the app's
/// number. So the search stops at the next section header.
export function stampedManifest(source, number) {
  const lines = source.split("\n");
  const start = lines.findIndex((line) => line.trim() === "[package]");
  if (start === -1) throw new Error("the manifest has no [package] section");

  for (let at = start + 1; at < lines.length; at += 1) {
    if (lines[at].trimStart().startsWith("[")) break;
    if (/^version\s*=/.test(lines[at])) {
      lines[at] = `version = "${number}"`;
      return lines.join("\n");
    }
  }
  throw new Error("the manifest's [package] has no version to replace");
}

// Run as a script: the tag is the one argument.
if (process.argv[1]?.endsWith("stamp-version.js")) {
  const number = releaseNumber(process.argv[2]);
  for (const [path, stamp] of [
    ["src-tauri/tauri.conf.json", stampedConfig],
    ["src-tauri/Cargo.toml", stampedManifest],
  ]) {
    writeFileSync(path, stamp(readFileSync(path, "utf8"), number));
    console.log(`${path}: ${number}`);
  }
}
