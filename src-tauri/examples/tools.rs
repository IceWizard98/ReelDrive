//! Print where this executable would look for ffmpeg and ffprobe, and what it
//! found — the same resolution the app does at startup.
//!
//! Useful copied into a built bundle: run it from inside
//! `ReelDrive.app/Contents/MacOS/` and it reports what the app itself
//! would resolve from there.
//!
//!     cargo run --example tools

use reeldrive_lib::adapters::{ffmpeg, std_fs};
use std::path::Path;

fn main() {
    let exe = std_fs::running_exe().expect("current executable");
    let tool_dirs = [
        exe.parent().unwrap_or(Path::new(".")).to_path_buf(),
        std_fs::app_dir_for_exe(&exe),
    ];

    println!("executable: {}", exe.display());
    println!("media root: {}", std_fs::media_root_for_exe(&exe).display());
    println!("searched:");
    for dir in &tool_dirs {
        println!("  {}", dir.display());
    }

    let mut missing = false;
    for name in ["ffmpeg", "ffprobe"] {
        let resolved = ffmpeg::tool_path(&tool_dirs, name);
        let shipped = resolved != Path::new(name);
        println!(
            "{name}: {} ({})",
            resolved.display(),
            if shipped {
                "shipped"
            } else {
                "not shipped — falling back to PATH"
            }
        );
        missing |= !shipped;
    }

    if missing {
        std::process::exit(1);
    }
}
