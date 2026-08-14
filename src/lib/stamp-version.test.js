import { describe, expect, it } from "vitest";
import { releaseNumber, stampedConfig, stampedManifest } from "../../scripts/stamp-version.js";

// The number the operating system shows is not the footer's. The footer takes
// the tag through an environment variable; the exe's properties, Info.plist and
// the AppImage's name are read out of `tauri.conf.json` and `Cargo.toml`, and
// until this script existed nothing put the tag into either. v0.1.0 shipped an
// executable that said 0.1.0 by coincidence.
//
// The first attempt at this was one line of `sed -i.bak "0,/^version = /s//.../"`
// in the workflow. GNU sed does what it says; BSD sed, which is what the macOS
// runners have, exits 0 and changes nothing. A release would have gone out with
// three platforms stamped and one not, and the only reason it would have been
// noticed is a separate check downstream. Hence a script, and hence these.

describe("releaseNumber", () => {
  it("takes the v off a tag, because the field will not have one", () => {
    expect(releaseNumber("v0.1.1")).toBe("0.1.1");
    expect(releaseNumber("0.1.1")).toBe("0.1.1");
  });

  it("refuses anything that is not three numbers, rather than writing it", () => {
    // Both files take `x.y.z` and nothing else. A tag this script cannot read
    // has to stop the release: an executable with no version at all is worse
    // than one nobody stamped, because it looks like a build that went wrong.
    for (const bad of ["", "v", "0.1", "0.1.1-rc1", "1.2.3.4", "latest", "v0.1.1 "]) {
      expect(() => releaseNumber(bad), bad).toThrow(/not a version/);
    }
  });
});

describe("stampedConfig", () => {
  const config = JSON.stringify({ productName: "ReelDrive", version: "0.1.0" }, null, 2);

  it("writes the number and leaves everything else where it was", () => {
    const stamped = JSON.parse(stampedConfig(config, "0.1.1"));
    expect(stamped.version).toBe("0.1.1");
    expect(stamped.productName).toBe("ReelDrive");
  });

  it("ends with a newline, like the file it replaces", () => {
    expect(stampedConfig(config, "0.1.1").endsWith("\n")).toBe(true);
  });

  it("says so when there was no version to replace", () => {
    // A renamed or restructured config would otherwise be stamped into a file
    // with a `version` key nobody reads, silently.
    expect(() => stampedConfig(JSON.stringify({ productName: "ReelDrive" }), "0.1.1")).toThrow(
      /no version/,
    );
  });
});

describe("stampedManifest", () => {
  const manifest = [
    "[package]",
    'name = "reeldrive"',
    'version = "0.1.0"',
    'edition = "2021"',
    "",
    "[dependencies]",
    'tiny_http = "0.12.0"',
    "",
  ].join("\n");

  it("writes the package's own version", () => {
    expect(stampedManifest(manifest, "0.1.1")).toContain('version = "0.1.1"');
  });

  it("leaves the dependencies alone, which carry the same key", () => {
    // The whole reason the sed had `0,` in front of it: a plain substitution
    // rewrites every pinned dependency to the app's version number.
    expect(stampedManifest(manifest, "0.1.1")).toContain('tiny_http = "0.12.0"');
    expect(stampedManifest(manifest, "0.1.1").match(/0\.1\.1/g)).toHaveLength(1);
  });

  it("only touches the line inside [package]", () => {
    const other = ["[package]", 'name = "reeldrive"', "", "[lib]", 'version = "9.9.9"', ""].join(
      "\n",
    );
    expect(() => stampedManifest(other, "0.1.1")).toThrow(/no version/);
  });

  it("says so when [package] carries no version at all", () => {
    expect(() => stampedManifest('[package]\nname = "reeldrive"\n', "0.1.1")).toThrow(/no version/);
  });
});
