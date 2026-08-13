// The number the footer shows. It is stamped in at build time by the release
// workflow (`VITE_REELDRIVE_VERSION`, taken from the tag), so the only place a
// release number is written is the tag itself — nothing in the tree has to be
// bumped by hand and then forgotten.
//
// A build made anywhere else has nothing there, and says "dev" rather than
// claiming a release it is not.
export function versionLabel(raw) {
  const text = (raw ?? "").trim();
  if (!text) return "dev";
  // A bare `1.2.0` in a line that also carries an address reads like anything;
  // the `v` is what makes it read as a version. Only a dotted number gets it:
  // what git answers on an untagged checkout is a commit hash, and ten of the
  // sixteen characters one can start with are digits — so "starts with a digit"
  // would put a `v` on most hashes and claim a release nobody cut.
  return /^\d+\./.test(text) ? `v${text}` : text;
}
