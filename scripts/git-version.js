// Where the version comes from when nobody stamped one in.
//
// A release gets its number from the tag, through the workflow's
// `VITE_REELDRIVE_VERSION`. Anyone else who clones this repository and builds
// it has no such variable — but they do have the checkout, and the checkout
// knows which commit it is on. `git describe --tags --always --dirty` answers
// the tag when sitting on one, `v0.2.0-3-gabc1234` a few commits past it, and
// the bare short hash when there is no tag anywhere behind. All three are worth
// more in a bug report than "dev".
//
// Node-only: this file is read by vite.config.js, never bundled into the app.

// The runner is an argument so the failures below can be stated as tests. Every
// one of them ends in the same place — an empty string, which the footer reads
// as "dev" — because a build must not stop over a missing `git`.
export function gitVersion(run) {
  let result;
  try {
    result = run("git", ["describe", "--tags", "--always", "--dirty"]);
  } catch {
    // A machine that refuses the spawn outright.
    return "";
  }
  // `spawnSync` does not throw when the binary is absent: it answers with a
  // null status. A non-zero one is what a directory with no .git gives — a
  // source tarball, most often.
  if (!result || result.status !== 0) return "";
  return (result.stdout ?? "").trim();
}
