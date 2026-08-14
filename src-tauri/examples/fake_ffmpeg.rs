//! Stands in for ffmpeg in the stream server's tests, on every platform.
//!
//! The suite used to write a `#!/bin/sh` script and point the server at it,
//! which is why every HLS test was `#[cfg(unix)]` and why not one of them had
//! ever run on Windows: the platform the app is broken on was the platform its
//! playback path was never executed on. This is the same idea as a real
//! process, without the shell.
//!
//! It cannot be given arguments of its own: the command line is built by the
//! code under test. So the plan travels as the *contents of the file being
//! converted*. Every test already writes its own fake film into its own
//! temporary directory, so two tests running at once cannot see each other's
//! plan, which an environment variable could not promise.
//!
//! One directive per line, executed in order:
//!
//! * `playlist <text>` writes `<text>` to the output path, the last argument
//! * `file <name> <text>` writes `<text>` beside that output
//! * `stdout <text>` and `stderr <text>` write to the two streams
//! * `sleep <seconds>` stays alive, which is what a conversion in progress does
//! * `exit <code>` leaves now with that code
//!
//! `\n` inside `<text>` is a newline, because a directive is one line.
//! Anything unreadable is a broken fixture, and a broken fixture exits 1 with a
//! sentence rather than quietly passing for a conversion that failed.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let Some(input) = value_after("-i", &args) else {
        eprintln!("fake ffmpeg: no -i in {args:?}");
        return ExitCode::FAILURE;
    };
    // The output is the last argument for both shapes this stands in for: the
    // playlist for a conversion, `pipe:1` for a subtitle extraction.
    let Some(output) = args.last().map(PathBuf::from) else {
        eprintln!("fake ffmpeg: no output argument");
        return ExitCode::FAILURE;
    };

    let plan = match std::fs::read_to_string(&input) {
        Ok(plan) => plan,
        Err(e) => {
            eprintln!("fake ffmpeg: no plan in {}: {e}", input.display());
            return ExitCode::FAILURE;
        }
    };

    for line in plan.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        match run(line, &output) {
            Ok(Some(code)) => return ExitCode::from(code),
            Ok(None) => {}
            Err(e) => {
                eprintln!("fake ffmpeg: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    ExitCode::SUCCESS
}

/// Carry out one directive. `Ok(Some(code))` means stop here with that code.
fn run(line: &str, output: &Path) -> Result<Option<u8>, String> {
    let (verb, rest) = match line.split_once(' ') {
        Some((verb, rest)) => (verb, rest),
        None => (line, ""),
    };

    match verb {
        "playlist" => write_file(output, &unescape(rest)).map(|()| None),
        "file" => {
            let (name, text) = rest.split_once(' ').unwrap_or((rest, ""));
            // `pipe:1` is the other output shape this stands in for, and its
            // parent is the empty path: `join` would then write the file into
            // whatever directory `cargo test` happens to run in, quietly, and
            // the test would go looking for it beside a playlist that does not
            // exist. A plan asking for that is a broken plan, not a conversion.
            let dir = output.parent().filter(|dir| !dir.as_os_str().is_empty());
            let dir = dir.ok_or_else(|| format!("no directory to write {name} into"))?;
            write_file(&dir.join(name), &unescape(text)).map(|()| None)
        }
        "stdout" => say(&mut std::io::stdout(), &unescape(rest)).map(|()| None),
        "stderr" => say(&mut std::io::stderr(), &unescape(rest)).map(|()| None),
        "sleep" => {
            let seconds: f64 = rest.trim().parse().map_err(|_| format!("sleep {rest}"))?;
            // `from_secs_f64` panics on a negative or non-finite number and on
            // one too large for a `Duration` (`sleep 1e30` parses fine), and a
            // stand-in that panics on a bad plan reads as a conversion that
            // crashed. Every other directive answers a broken plan with a
            // sentence, so this one does too: `try_from_secs_f64` is that same
            // check without a panic behind it.
            let length = std::time::Duration::try_from_secs_f64(seconds)
                .map_err(|_| format!("sleep {rest} is not a length of time"))?;
            std::thread::sleep(length);
            Ok(None)
        }
        "exit" => {
            let code: u8 = rest.trim().parse().map_err(|_| format!("exit {rest}"))?;
            Ok(Some(code))
        }
        other => Err(format!("unknown directive: {other}")),
    }
}

fn write_file(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("writing {}: {e}", path.display()))
}

fn say(out: &mut impl Write, text: &str) -> Result<(), String> {
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|e| format!("writing out: {e}"))
}

/// The one escape a single-line directive needs.
fn unescape(text: &str) -> String {
    text.replace("\\n", "\n")
}

/// The value following `name`, if the arguments carry one.
fn value_after(name: &str, args: &[String]) -> Option<PathBuf> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|at| args.get(at + 1))
        .map(PathBuf::from)
}
