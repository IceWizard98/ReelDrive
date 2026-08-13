//! Print the watch history of a media folder, and what each title would play
//! next, without opening a window.
//!
//!     cargo run --example watched -- /path/to/media
//!
//! The second half is the part worth having: "what comes next" is derived from
//! the seasons and the history together, so the only way to see it go wrong is
//! against a real folder with a real history beside it.

use reeldrive_lib::adapters::json_progress::JsonProgress;
use reeldrive_lib::adapters::std_fs::StdFs;
use reeldrive_lib::core::progress::{self, ProgressData};
use reeldrive_lib::core::scanner;
use reeldrive_lib::ports::progress::ProgressStore;
use std::path::PathBuf;

fn clock(seconds: f64) -> String {
    let whole = seconds.max(0.0) as u64;
    format!(
        "{}:{:02}:{:02}",
        whole / 3600,
        (whole % 3600) / 60,
        whole % 60
    )
}

fn main() {
    let Some(arg) = std::env::args().nth(1) else {
        eprintln!("usage: cargo run --example watched -- <media folder>");
        std::process::exit(2);
    };
    let root = PathBuf::from(arg);

    let store = JsonProgress::in_media_root(&root);
    let history = store.load();
    println!("history: {}", store.path().display());
    if history == ProgressData::default() {
        println!("  (nothing watched yet)");
    }
    for (path, mark) in &history.items {
        println!(
            "  {:<56} {:>9} / {:<9} {}",
            path,
            clock(mark.seconds),
            clock(mark.duration),
            if mark.done { "watched" } else { "" }
        );
    }

    let scan = match scanner::scan_library(&StdFs, &root) {
        Ok(scan) => scan,
        Err(e) => {
            eprintln!("scan failed: {e}");
            std::process::exit(1);
        }
    };

    println!("\nup next");
    for summary in &scan.contents {
        let detail = match scanner::scan_content(&StdFs, &root, &summary.id) {
            Ok(detail) => detail,
            Err(e) => {
                println!("  {:<24} could not be read: {e}", summary.id);
                continue;
            }
        };
        match progress::up_next(&detail, &history) {
            None => println!("  {:<24} finished", summary.id),
            Some(next) => println!(
                "  {:<24} {} {}",
                summary.id,
                next.file,
                if next.seconds > 0.0 {
                    format!("from {}", clock(next.seconds))
                } else if next.fresh {
                    "from the start".to_string()
                } else {
                    "not started".to_string()
                }
            ),
        }
    }
}
