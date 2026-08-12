//! Ask the stream server for a file exactly as the player does, and print what
//! comes back — status line, headers, and the first bytes of the body.
//!
//!     cargo run --example serve -- /path/to/media "Movie/Movie.mkv" [remux|direct|transcode-audio|transcode] [audio-track]
//!
//! `REELDRIVE_TOOL_DIR` points at a folder holding ffmpeg and ffprobe,
//! for checking a shipped pair rather than whatever is on PATH.

use reeldrive_lib::adapters::ffmpeg;
use reeldrive_lib::adapters::stream_server::{StreamConfig, StreamServer};
use reeldrive_lib::core::media::{Capabilities, Delivery};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("media root"));
    let relative = args.next().expect("relative path");
    let forced = args.next();
    // Fourth argument: which audio track to carry, for checking a film that
    // holds more than one language.
    let audio: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);

    let tool_dirs: Vec<PathBuf> = std::env::var("REELDRIVE_TOOL_DIR")
        .map(|dir| vec![PathBuf::from(dir)])
        .unwrap_or_default();
    let ffprobe_path = ffmpeg::tool_path(&tool_dirs, "ffprobe");
    let ffmpeg_path = ffmpeg::tool_path(&tool_dirs, "ffmpeg");
    println!(
        "ffprobe: {}\nffmpeg:  {}",
        ffprobe_path.display(),
        ffmpeg_path.display()
    );

    let file = root.join(&relative);
    let profile = ffmpeg::probe(&ffprobe_path, &file).expect("probe");
    // The same decision the app makes: it follows the chosen audio track.
    let audio = profile.audio_track_or_first(audio);
    let chosen = profile.delivery_for(audio, Capabilities::current());
    let delivery = match forced.as_deref() {
        Some("direct") => Delivery::Direct,
        Some("remux") => Delivery::Remux,
        Some("transcode-audio") => Delivery::TranscodeAudio,
        Some("transcode") => Delivery::Transcode,
        _ => chosen,
    };
    println!(
        "probe: {:?} / {:?}",
        profile.video_codec, profile.audio_codec
    );
    println!("audio tracks: {:?}", profile.audio);
    println!("delivery chosen: {chosen:?}   requested: {delivery:?}   audio track: {audio}\n");

    let server = StreamServer::start(StreamConfig {
        media_root: root,
        ffmpeg: ffmpeg_path,
    })
    .expect("server");

    let url = server
        .stream_url(
            &relative,
            0.0,
            delivery,
            profile.video_codec.as_deref(),
            audio,
        )
        .expect("stream url");
    println!("GET {url}\n");

    let target = url.split_once("127.0.0.1:").expect("host").1;
    let (port, path) = target.split_once('/').expect("path");
    let playlist = path.ends_with(".m3u8");
    let mut socket = TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    socket
        .write_all(
            format!("GET /{path} HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: tauri://localhost\r\n\r\n")
                .as_bytes(),
        )
        .expect("send");

    let mut buffer = vec![0u8; 64 * 1024];
    let mut collected = Vec::new();
    // Enough to see the headers and the start of the playlist or the first
    // fragment. A playlist is small and the connection closes on its own.
    while collected.len() < 8192 {
        match socket.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => collected.extend_from_slice(&buffer[..read]),
            Err(e) => {
                println!("read error: {e}");
                break;
            }
        }
    }

    let split = collected
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .unwrap_or(collected.len());
    println!("--- response head ---");
    println!(
        "{}",
        String::from_utf8_lossy(&collected[..split]).trim_end()
    );
    let body = &collected[split..];
    println!("--- body: {} bytes read ---", body.len());
    if playlist {
        // The playlist is the thing Safari reads: printing it shows both that
        // ffmpeg is producing segments and that their URIs are relative names.
        println!("{}", String::from_utf8_lossy(body).trim_end());
    } else if body.len() >= 12 {
        println!(
            "first box: {:?} (bytes 4..8 of an MP4 should be 'ftyp')",
            String::from_utf8_lossy(&body[4..8])
        );
    } else {
        println!("body too short to inspect: {body:?}");
    }

    // One fetch says the conversion started. It says nothing about whether it
    // finishes, and that is the failure that matters: ffmpeg giving up after a
    // few segments leaves a playlist that never grows, which the player can
    // only sit and watch. Polling it the way the player does is the only way to
    // see that from here.
    if playlist {
        follow(port, path, &server);
    }

    // Playing it takes a browser: the session keeps running so the URL above can
    // be opened in Safari, which is the engine the app's webview is.
    if std::env::var_os("SERVE_KEEP_ALIVE").is_some() {
        println!("\nserver staying up; open the URL above in Safari. Ctrl-C to stop.");
        std::thread::sleep(std::time::Duration::from_secs(600));
    }
}

/// Re-fetch the playlist until it is finished, refused, or the patience runs
/// out — printing how many segments each answer holds.
fn follow(port: &str, path: &str, server: &StreamServer) {
    let patience = std::time::Duration::from_secs(
        std::env::var("SERVE_FOLLOW_SECONDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60),
    );
    println!("\n--- following the playlist ---");
    let started = std::time::Instant::now();
    while started.elapsed() < patience {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let (head, body) = fetch(port, path);
        let status = head.lines().next().unwrap_or("").to_string();
        let segments = body.matches(".m4s").count();
        let done = body.contains("ENDLIST");
        println!(
            "{:5.1}s  {status}  segments: {segments}{}",
            started.elapsed().as_secs_f64(),
            if done { "  ENDLIST" } else { "" }
        );
        if !status.contains(" 200") {
            match server.last_failure() {
                Some(said) => println!("the conversion stopped: {said}"),
                None => println!("no reason recorded"),
            }
            return;
        }
        if done {
            println!("the conversion finished cleanly");
            return;
        }
    }
    println!("still going after {}s", patience.as_secs());
}

fn fetch(port: &str, path: &str) -> (String, String) {
    let Ok(mut socket) = TcpStream::connect(format!("127.0.0.1:{port}")) else {
        return ("connect failed".into(), String::new());
    };
    let _ = socket.write_all(
        format!("GET /{path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes(),
    );
    let mut raw = Vec::new();
    let _ = socket.read_to_end(&mut raw);
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .unwrap_or(raw.len());
    (
        String::from_utf8_lossy(&raw[..split]).into_owned(),
        String::from_utf8_lossy(&raw[split..]).into_owned(),
    )
}
