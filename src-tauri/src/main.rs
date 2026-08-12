// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The one place allowed to end the process: a failure to start is the
    // user's problem to read, not a backtrace to decode.
    if let Err(e) = reeldrive_lib::run() {
        eprintln!("ReelDrive could not start: {e}");
        std::process::exit(1);
    }
}
