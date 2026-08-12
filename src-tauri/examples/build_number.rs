//! Print the build number baked into this binary, and the cache file it will
//! accept. A release should report the workflow's run number; a build from a
//! developer's machine reports 0.
//!
//!     cargo run --example build_number
//!     REELDRIVE_BUILD=42 cargo run --example build_number

use reeldrive_lib::defaults;
use reeldrive_lib::ports::cache::{BUILD_NUMBER, CACHE_VERSION};

fn main() {
    println!("build: {BUILD_NUMBER}");
    println!("cache format version: {CACHE_VERSION}");
    println!("cache file: {}", defaults::CACHE_FILE_NAME);
    println!("removed on sight: {:?}", defaults::LEGACY_CACHE_FILE_NAMES);
}
