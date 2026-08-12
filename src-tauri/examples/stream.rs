//! Probe a file, decide how it must be delivered, and run that conversion into
//! a temporary file so the result can be inspected.
//!
//!     cargo run --example stream -- /path/to/video.mkv [seconds]

use reeldrive_lib::adapters::ffmpeg;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(file) = args.next().map(PathBuf::from) else {
        eprintln!("usage: cargo run --example stream -- <file> [seconds]");
        std::process::exit(2);
    };
    let start: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);

    let profile = match ffmpeg::probe(Path::new("ffprobe"), &file) {
        Ok(profile) => profile,
        Err(e) => {
            eprintln!("probe failed: {e}");
            std::process::exit(1);
        }
    };

    let delivery = profile.delivery(reeldrive_lib::core::media::Capabilities::current());
    println!("container: {}", profile.container);
    println!(
        "video: {}   audio: {}",
        profile.video_codec.as_deref().unwrap_or("-"),
        profile.audio_codec.as_deref().unwrap_or("-")
    );
    println!(
        "duration: {}",
        profile
            .duration
            .map(|d| format!("{d:.1}s"))
            .unwrap_or_else(|| "unknown".into())
    );
    println!("subtitles: {}", profile.subtitles.len());
    for track in &profile.subtitles {
        println!(
            "  {} {} {}",
            track.index,
            track.language.as_deref().unwrap_or("--"),
            if track.textual {
                "text"
            } else {
                "bitmap (unusable)"
            }
        );
    }
    println!("delivery: {delivery:?}\n");

    let out = std::env::temp_dir().join("reeldrive-stream.mp4");
    let args = ffmpeg::stream_args(&file, delivery, start, profile.video_codec.as_deref(), 0);
    println!("ffmpeg {}\n", args.join(" "));

    let began = Instant::now();
    let output = Command::new("ffmpeg")
        .args(args.iter().map(|a| {
            if a == "pipe:1" {
                out.to_string_lossy().into_owned()
            } else {
                a.clone()
            }
        }))
        .arg("-y")
        .output()
        .expect("run ffmpeg");

    if !output.status.success() {
        eprintln!(
            "ffmpeg failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        std::process::exit(1);
    }

    let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    println!(
        "wrote {} ({:.1} MB) in {:?}",
        out.display(),
        size as f64 / 1_048_576.0,
        began.elapsed()
    );

    // The point of the exercise: whatever came out has to be a file the webview
    // would accept, so probe it back.
    match ffmpeg::probe(Path::new("ffprobe"), &out) {
        Ok(result) => println!(
            "result: {} / {} / {}",
            result.container,
            result.video_codec.as_deref().unwrap_or("-"),
            result.audio_codec.as_deref().unwrap_or("-")
        ),
        Err(e) => eprintln!("the output could not be probed: {e}"),
    }
}
