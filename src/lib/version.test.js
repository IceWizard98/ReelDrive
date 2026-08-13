import { describe, expect, it, vi } from "vitest";
import { versionLabel } from "./version.js";
import { gitVersion } from "../../scripts/git-version.js";

// The label is covered end to end through the footer in credit.test.js. These
// are the inputs that only a build machine produces, and mounting the whole app
// to state them would say nothing extra.
describe("versionLabel", () => {
  it("leaves a commit description alone", () => {
    // What `git describe` answers on a checkout with no tag on it. Prefixing a
    // `v` there would invent a version that does not exist.
    expect(versionLabel("efd249e-dirty")).toBe("efd249e-dirty");
  });

  it("leaves a hash alone when the hash happens to start with a digit", () => {
    // Ten of the sixteen first characters a short hash can have are digits, so
    // this is what most untagged builds look like — not the exception. A `v`
    // here is the same invented release the case above exists to prevent.
    expect(versionLabel("1a2b3c4")).toBe("1a2b3c4");
    expect(versionLabel("9f3ac21-dirty")).toBe("9f3ac21-dirty");
  });

  it("keeps the detail of a build past a tag", () => {
    expect(versionLabel("v0.2.0-3-gabc1234")).toBe("v0.2.0-3-gabc1234");
  });

  it("trims what a dispatch form let through", () => {
    // GitHub does not trim workflow_dispatch inputs, and a trailing space is
    // the easiest thing to paste.
    expect(versionLabel("  v1.2.0  ")).toBe("v1.2.0");
    expect(versionLabel("   ")).toBe("dev");
  });
});

// The other half of the chain: no release env, but the person building has the
// repository, so the checkout itself can say which commit this is.
describe("gitVersion", () => {
  const answer = (result) => vi.fn().mockReturnValue(result);

  it("reports what the checkout is on", () => {
    const run = answer({ status: 0, stdout: "v0.2.0\n" });
    expect(gitVersion(run)).toBe("v0.2.0");
    expect(run).toHaveBeenCalledWith("git", ["describe", "--tags", "--always", "--dirty"]);
  });

  it("says nothing when git is not installed", () => {
    // spawnSync does not throw for a missing binary: it answers with a null
    // status and an `error`. Reading that as a version would put the word
    // "null" in the footer.
    expect(gitVersion(answer({ status: null, error: new Error("ENOENT") }))).toBe("");
  });

  it("says nothing outside a repository", () => {
    // A source tarball from the releases page has no .git at all.
    expect(gitVersion(answer({ status: 128, stdout: "", stderr: "not a git repository" }))).toBe("");
  });

  it("refuses what a failed git still printed", () => {
    // The two cases above pass with no status check at all, because both are
    // also empty on stdout. This is the shape that needs the guard: git failed
    // and wrote something anyway, and what it wrote is not a version.
    expect(gitVersion(answer({ status: 128, stdout: "v9.9.9\n" }))).toBe("");
  });

  it("says nothing when git answered with nothing", () => {
    expect(gitVersion(answer({ status: 0, stdout: "\n" }))).toBe("");
    expect(gitVersion(answer({ status: 0 }))).toBe("");
  });

  it("survives a runner that throws", () => {
    // A locked-down machine can refuse the spawn outright.
    expect(
      gitVersion(() => {
        throw new Error("EPERM");
      }),
    ).toBe("");
  });
});
