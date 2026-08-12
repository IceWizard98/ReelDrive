//! Print what the app would show for a media folder, without opening a window.
//!
//!     cargo run --example scan -- /path/to/media

use reeldrive_lib::adapters::json_cache::JsonCache;
use reeldrive_lib::adapters::std_fs::StdFs;
use reeldrive_lib::core::model::ContentBody;
use reeldrive_lib::core::scanner;
use reeldrive_lib::ports::cache::{LibraryCache, LibraryCacheData};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let Some(arg) = std::env::args().nth(1) else {
        eprintln!("usage: cargo run --example scan -- <media folder>");
        std::process::exit(2);
    };
    let root = PathBuf::from(arg);

    let cache_file = JsonCache::in_media_root(&root);
    let cached = cache_file.load().unwrap_or_default();
    println!(
        "cache: {} entries{}",
        cached.entries.len(),
        if cached == LibraryCacheData::default() {
            " (missing or empty)"
        } else {
            ""
        }
    );

    let started = Instant::now();
    let (scan, fresh) = match scanner::scan_library_cached(&StdFs, &root, &cached) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    println!("library scan: {:?}\n", started.elapsed());

    for summary in &scan.contents {
        let year = summary.year.map(|y| format!(" ({y})")).unwrap_or_default();
        let cover = summary.cover.as_deref().unwrap_or("- no cover -");
        println!("{}{}  [{}]", summary.title, year, cover);

        match scanner::scan_content(&StdFs, &root, &summary.id) {
            Ok(detail) => match detail.body {
                ContentBody::Movie { file, subtitles } => {
                    println!("    MOVIE  {file}");
                    for sub in subtitles {
                        println!("          sub: {sub}");
                    }
                }
                ContentBody::Series { seasons } => {
                    println!(
                        "    SERIES  {} season{}",
                        seasons.len(),
                        if seasons.len() == 1 { "" } else { "s" }
                    );
                    for season in seasons {
                        println!("      season {} ({})", season.number, season.episodes.len());
                        for episode in season.episodes {
                            let subs = if episode.subtitles.is_empty() {
                                String::new()
                            } else {
                                format!("  [{} sub]", episode.subtitles.len())
                            };
                            println!(
                                "        {:>2}. {}{}",
                                episode.number,
                                if episode.title.is_empty() {
                                    "—"
                                } else {
                                    &episode.title
                                },
                                subs
                            );
                        }
                    }
                }
            },
            Err(e) => println!("    ERROR: {e}"),
        }
        println!();
    }

    for warning in &scan.warnings {
        println!("skipped: {warning}");
    }

    match cache_file.store(&fresh) {
        Ok(()) => println!("\ncache written: {}", cache_file.path().display()),
        Err(e) => println!("\ncache not written: {e}"),
    }
}
