//! Print the build number baked into this binary, and the cache file it will
//! accept. A release should report the workflow's run number; a build from a
//! developer's machine reports 0.
//!
//!     cargo run --example build_number
//!     REELDRIVE_BUILD=42 cargo run --example build_number

use reeldrive_lib::defaults;
use reeldrive_lib::ports::cache::{BUILD_NUMBER, CACHE_VERSION};

fn main() {
    // The number the operating system will show for this build. Unlike the
    // footer's, it does not come from the tag by itself: the release workflow
    // writes the tag into `Cargo.toml` and `tauri.conf.json` before compiling,
    // and this is where it can check that the writing worked. A build from a
    // developer's machine reports whatever is in the tree.
    println!("version: {}", env!("CARGO_PKG_VERSION"));
    println!("build: {BUILD_NUMBER}");
    println!("cache format version: {CACHE_VERSION}");
    println!("cache file: {}", defaults::CACHE_FILE_NAME);
    println!("removed on sight: {:?}", defaults::LEGACY_CACHE_FILE_NAMES);
}
