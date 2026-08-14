//! Local HTTP server feeding the `<video>` element.
//!
//! A custom URI scheme would be tidier, but Tauri hands back a whole response
//! body at once and a film does not fit in memory. So: a socket on the loopback
//! interface, on a port the system picks, guarded by a token minted at startup —
//! without it, any other process on the machine could read files through this.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::adapters::ffmpeg;
use crate::core::media::{self, Delivery};
use crate::core::paths;

pub struct StreamServer {
    port: u16,
    token: String,
    config: Arc<StreamConfig>,
    session: Session,
    failure: Failure,
}

/// What the last conversion said on its way out, if it died badly.
///
/// A player cannot read the body of a request its media element made, so the
/// sentence has to be fetchable separately: the element reports "it broke" and
/// the window asks here what broke.
type Failure = Arc<Mutex<Option<String>>>;

/// Everything a request needs to be answered, resolved once at startup.
pub struct StreamConfig {
    pub media_root: PathBuf,
    pub ffmpeg: PathBuf,
}

/// The one conversion currently feeding the player, if any.
///
/// Only one thing plays at a time, so a new one replaces the old — and
/// replacing it is what stops its ffmpeg and removes its files. Shared with the
/// request threads, which need the directory to serve segments out of it.
type Session = Arc<Mutex<Option<HlsSession>>>;

/// An ffmpeg writing a playlist into a directory of its own.
struct HlsSession {
    id: String,
    dir: PathBuf,
    child: Child,
    /// The tail of what ffmpeg wrote to stderr. Discarding it was how a
    /// conversion that gave up halfway became a picture that simply stopped:
    /// the playlist stopped growing, the element sat on its last frame, and
    /// the one sentence explaining why went to /dev/null.
    said: Arc<Mutex<String>>,
}

/// The only place a session is torn down, so no path can forget half of it.
///
/// `Child` does not kill on drop: without this an abandoned conversion keeps
/// running and keeps filling the directory nobody is reading any more.
impl Drop for HlsSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Err(e) = std::fs::remove_dir_all(&self.dir) {
            eprintln!("stream: session directory not removed: {e}");
        }
    }
}

/// A poisoned lock still holds a usable session; refusing to play because a
/// request thread panicked would be worse than carrying on.
fn locked(session: &Session) -> std::sync::MutexGuard<'_, Option<HlsSession>> {
    session.lock().unwrap_or_else(|e| e.into_inner())
}

/// How much of ffmpeg's complaint is worth keeping. It runs with `-v error`,
/// so this is several lines of real diagnosis and no banner; the cap is only
/// there so a process that decides to shout cannot grow the buffer for ever.
const STDERR_KEPT: usize = 4096;

/// Drain the child's stderr into a buffer, on a thread of its own.
fn collect_stderr(child: &mut Child) -> Arc<Mutex<String>> {
    let said = Arc::new(Mutex::new(String::new()));
    let Some(mut pipe) = child.stderr.take() else {
        return said;
    };
    let sink = Arc::clone(&said);
    std::thread::spawn(move || {
        let mut buffer = [0u8; 1024];
        loop {
            match pipe.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    let mut held = sink.lock().unwrap_or_else(|e| e.into_inner());
                    if held.len() < STDERR_KEPT {
                        held.push_str(&String::from_utf8_lossy(&buffer[..read]));
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });
    said
}

/// Whether the conversion behind `id` has stopped badly, and what it said.
///
/// Nothing else notices. ffmpeg gives up, the playlist stops growing, and the
/// element goes on asking for a playlist that never changes — a film that
/// stops halfway with no error anywhere. Reported once here, the request fails
/// and the player can say so.
fn conversion_failure(session: &Session, id: &str) -> Option<String> {
    let mut guard = locked(session);
    let current = guard.as_mut().filter(|open| open.id == id)?;
    match current.child.try_wait() {
        Ok(Some(status)) if !status.success() => {
            let said = current
                .said
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .trim()
                .to_string();
            Some(if said.is_empty() {
                format!("The conversion stopped early ({status}).")
            } else {
                said
            })
        }
        // Still running, finished cleanly, or unreadable — none of them is a
        // failure to report. A conversion that ends well writes its own
        // end-of-playlist marker and the player stops asking.
        _ => None,
    }
}

impl StreamServer {
    /// Bind and start serving. The thread runs for the life of the process.
    pub fn start(config: StreamConfig) -> io::Result<Self> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        let token = mint_token();
        let config = Arc::new(config);
        let session: Session = Arc::new(Mutex::new(None));
        let failure: Failure = Arc::new(Mutex::new(None));

        let server = tiny_http::Server::from_listener(listener, None)
            .map_err(|e| io::Error::other(e.to_string()))?;

        let shared = Arc::new((
            Arc::clone(&config),
            token.clone(),
            Arc::clone(&session),
            Arc::clone(&failure),
        ));
        std::thread::spawn(move || {
            for request in server.incoming_requests() {
                let shared = Arc::clone(&shared);
                // One thread per request: a stream occupies its thread until the
                // player disconnects, so serving must not be serialised.
                std::thread::spawn(move || {
                    if let Err(e) = handle(&shared.0, &shared.1, &shared.2, &shared.3, request) {
                        eprintln!("stream: {e}");
                    }
                });
            }
        });

        Ok(Self {
            port,
            token,
            config,
            session,
            failure,
        })
    }

    /// What the conversion said when it gave up, if it did.
    ///
    /// The element reports a failure without a reason — it has none — so the
    /// window asks here for the sentence to put in the error box.
    pub fn last_failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// URL for a media file, from `t` seconds in, with the treatment it needs.
    ///
    /// An untouched file is served as itself, with byte ranges, which is what
    /// gives the element its own seek bar. Everything else becomes an HLS
    /// session: this starts ffmpeg now, so by the time the player asks for the
    /// playlist the first segments are already on disk.
    pub fn stream_url(
        &self,
        relative: &str,
        start: f64,
        delivery: Delivery,
        video_codec: Option<&str>,
        audio: u32,
    ) -> io::Result<String> {
        // A direct URL is the file itself, and `/stream` answers it with
        // `serve_file`, which knows nothing about `t`: the film would arrive
        // from its first byte while the caller anchors the clock at `start`.
        // Rewrapping costs milliseconds and begins where it was asked to, so
        // the offset — not the caller — is what rules Direct out. Kept here
        // rather than in the commands because all three of them can ask for a
        // delivery decided before the offset was known.
        let delivery = media::delivery_at(delivery, start, audio);
        if delivery == Delivery::Direct {
            return Ok(format!(
                "http://127.0.0.1:{}/stream?token={}&path={}&t={:.3}&delivery={}&vcodec={}&audio={}",
                self.port,
                self.token,
                urlencode(relative),
                start,
                delivery_name(delivery),
                urlencode(video_codec.unwrap_or_default()),
                audio
            ));
        }
        self.start_hls(relative, start, delivery, video_codec, audio)
    }

    /// Spawn the conversion and hand back the URL of its playlist.
    ///
    /// The token is a path segment rather than a query parameter because the
    /// segment URIs inside the playlist are relative names: the player resolves
    /// them against the playlist's directory and drops its query string, so a
    /// token in the query would guard the playlist and lock out every segment.
    fn start_hls(
        &self,
        relative: &str,
        start: f64,
        delivery: Delivery,
        video_codec: Option<&str>,
        audio: u32,
    ) -> io::Result<String> {
        // The same check every other route makes: a path from the webview must
        // not name a file outside the media folder.
        let file = paths::resolve_media_file(&self.config.media_root, relative)
            .map_err(|e| io::Error::other(e.to_string()))?;

        let id = mint_token();
        let dir = std::env::temp_dir().join(format!("reeldrive-{id}"));
        std::fs::create_dir_all(&dir)?;

        let spawned = ffmpeg::command(&self.config.ffmpeg)
            .args(ffmpeg::hls_args(
                &file,
                delivery,
                start,
                video_codec,
                audio,
                &dir,
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            // Kept, not discarded: this is the only place that ever learns why
            // a conversion stopped. Piped rather than inherited so the text can
            // reach the window instead of only a terminal nobody is watching.
            .stderr(Stdio::piped())
            .spawn();

        let spawned = spawned.map_err(|e| {
            // The same sentence the probe gives: this is the second half of
            // the pair, and a missing ffmpeg fails here instead.
            io::Error::other(ffmpeg::missing_tool(&self.config.ffmpeg, &e).to_string())
        });

        let mut child = match spawned {
            Ok(child) => child,
            Err(e) => {
                // Nothing owns the directory yet, so nothing else would remove it.
                let _ = std::fs::remove_dir_all(&dir);
                return Err(e);
            }
        };

        // Drained on a thread of its own: a pipe nobody reads fills up, and a
        // full stderr blocks ffmpeg mid-conversion — which is the very stall
        // this is here to explain.
        let said = collect_stderr(&mut child);

        // Whatever it says about this film, it does not describe the last one.
        *self.failure.lock().unwrap_or_else(|e| e.into_inner()) = None;

        // Whatever was playing before has to be torn down — killed, waited on,
        // and its directory removed — and a seek that supersedes another one is
        // the ordinary case, not the exception. Assigning through the guard
        // would run all of that *while still holding the lock*, and every
        // in-flight segment request takes this same lock: a session left
        // running for a few minutes has hundreds of segments to delete, so the
        // cleanup would stall the player at the exact moment the user asked to
        // jump. The old session is lifted out under the lock and dropped after
        // it. It needs a name — `drop(std::mem::replace(..))` would still run
        // the destructor inside the statement, guard and all.
        let superseded = locked(&self.session).replace(HlsSession {
            id: id.clone(),
            dir,
            child,
            said,
        });
        // The guard is gone with the statement above; this runs the teardown
        // with the lock free.
        drop(superseded);

        Ok(format!(
            "http://127.0.0.1:{}/hls/{}/{}/{}",
            self.port,
            self.token,
            id,
            ffmpeg::HLS_PLAYLIST
        ))
    }

    /// End the current session, if there is one.
    ///
    /// Dropping the server would do this, but the application state holding it
    /// is not guaranteed to be dropped on exit — and an ffmpeg that outlives the
    /// window keeps a CPU busy for a film nobody is watching.
    ///
    /// Lifted out under the lock and dropped after it, for the reason
    /// `start_hls` sets out at length: the teardown kills ffmpeg, waits for it
    /// and deletes every segment written so far, and every request thread takes
    /// this same lock. Assigning `None` through the guard runs all of that with
    /// the lock still held.
    pub fn stop(&self) {
        let ended = locked(&self.session).take();
        drop(ended);
    }

    /// URL for one subtitle track, as WebVTT, cut to the same `start` the
    /// picture it accompanies begins at — see `ffmpeg::subtitle_args`.
    pub fn subtitle_url(&self, relative: &str, track: u32, start: f64) -> String {
        format!(
            "http://127.0.0.1:{}/subtitle?token={}&path={}&track={}&t={:.3}",
            self.port,
            self.token,
            urlencode(relative),
            track,
            start
        )
    }
}

/// Losing the server ends the session it was feeding.
///
/// The serving thread holds a handle to the same session and never returns, so
/// the session would otherwise outlive every owner that could stop it — an
/// ffmpeg and a directory of segments with nothing left to read them.
impl Drop for StreamServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 32 hex characters from the OS random source. Falls back to a
/// time-and-address mix only if that fails, which is better than an empty
/// token but is not equivalent — hence the warning.
fn mint_token() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        eprintln!("stream: no OS randomness, falling back to a weaker token");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let mixed = now ^ (&bytes as *const _ as u128);
        bytes.copy_from_slice(&mixed.to_le_bytes());
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn urldecode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Split `a=1&b=2` into pairs, decoding both sides.
pub fn parse_query(url: &str) -> Vec<(String, String)> {
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| match part.split_once('=') {
            Some((key, value)) => (urldecode(key), urldecode(value)),
            None => (urldecode(part), String::new()),
        })
        .collect()
}

/// The `t` parameter: where in the film this request begins. Both routes that
/// take one read it the same way, so a stream and the track that goes with it
/// can never end up on two different clocks.
fn start_param(query: &[(String, String)]) -> f64 {
    param(query, "t")
        .and_then(|t| t.parse::<f64>().ok())
        .filter(|t| t.is_finite() && *t >= 0.0)
        .unwrap_or(0.0)
}

/// A track number: which subtitle stream to extract, or which audio stream to
/// carry. Absent or unreadable means the first one, which is what ffmpeg would
/// pick anyway.
fn track_param(query: &[(String, String)], name: &str) -> u32 {
    param(query, name)
        .and_then(|t| t.parse::<u32>().ok())
        .unwrap_or(0)
}

fn param(query: &[(String, String)], name: &str) -> Option<String> {
    query
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.clone())
}

/// The webview lives on `tauri://localhost`, so every response here is
/// cross-origin. A `<video>` fetches media without CORS, but a `<track>` is
/// dropped unless the response allows the origin — silently, which is how
/// subtitles disappear. The token, not the origin, is what guards this server.
const CORS: (&str, &str) = ("Access-Control-Allow-Origin", "*");

fn header(field: &str, value: &str) -> Result<tiny_http::Header, Box<dyn std::error::Error>> {
    tiny_http::Header::from_bytes(field, value).map_err(|_| "invalid header".into())
}

fn respond_status(
    request: tiny_http::Request,
    code: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = tiny_http::Response::empty(code).with_header(header(CORS.0, CORS.1)?);
    Ok(request.respond(response)?)
}

/// Record why the conversion stopped, then fail the request.
///
/// Failing it is what turns a silent stall into something the player can react
/// to: a playlist that answers 500 makes the element report an error, and the
/// window then asks for this sentence to put on screen.
fn respond_failure(
    request: tiny_http::Request,
    failure: &Failure,
    said: String,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("stream: the conversion stopped: {said}");
    *failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(said);
    respond_status(request, 500)
}

/// How long a playlist request waits for ffmpeg to write the first segment.
/// Copying takes milliseconds; a full conversion of a large frame needs longer,
/// and giving up early shows the user a failure that was only slowness.
#[cfg(not(test))]
const PLAYLIST_WAIT: Duration = Duration::from_secs(20);
/// Two seconds under test. The deadline is a thing several tests wait out, and
/// twenty seconds each would be most of the suite's running time.
#[cfg(test)]
const PLAYLIST_WAIT: Duration = Duration::from_secs(2);

/// The playlist as one whole snapshot, or nothing if it was caught mid-rewrite.
///
/// ffmpeg does not append to this file, it rewrites it in place after every
/// segment, and during a remux that is dozens of times a second. A read landing
/// inside one of those rewrites gets an empty or half-written file, and serving
/// it is worse than serving nothing: hls.js, which is what plays on every
/// engine but WKWebView, treats a body with no `#EXTM3U` as a fatal parse error
/// and never asks for it again. Waiting a few milliseconds for the next whole
/// one costs nothing and is the difference between a film that starts and a
/// film that reports it cannot be played.
///
/// Whole means both ends: the tag it has to open with, and the newline ffmpeg
/// closes every line with: a body cut halfway through the last segment name
/// begins correctly and is still not a playlist.
fn playlist_snapshot(file: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(file).ok()?;
    let whole = bytes.starts_with(b"#EXTM3U") && bytes.last() == Some(&b'\n');
    whole.then_some(bytes)
}

/// Answer with a playlist read in one go, rather than streamed off disk.
///
/// `serve_file` sends a length taken from a `stat` and then reads the file, and
/// for a file another process rewrites continuously those are two different
/// files. This one has the bytes already.
fn respond_playlist(
    request: tiny_http::Request,
    bytes: Vec<u8>,
) -> Result<(), Box<dyn std::error::Error>> {
    let length = bytes.len();
    let response = tiny_http::Response::new(
        200.into(),
        vec![
            header("Content-Type", "application/vnd.apple.mpegurl")?,
            // An event playlist grows: cached once, the player never sees the
            // rest of the film.
            header("Cache-Control", "no-cache")?,
            header(CORS.0, CORS.1)?,
        ],
        io::Cursor::new(bytes),
        Some(length),
        None,
    );
    Ok(request.respond(response)?)
}

/// What the conversion has said so far, whether or not it has stopped.
fn said_so_far(session: &Session, id: &str) -> String {
    match locked(session).as_ref() {
        Some(current) if current.id == id => current
            .said
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .trim()
            .to_string(),
        _ => String::new(),
    }
}

/// The deadline is up and there is still nothing playable.
///
/// It stays a 504 rather than becoming a 500: a conversion this slow is not a
/// conversion that failed. What changes is that it no longer goes quiet. The
/// bare status left `failure` empty, so the player asked the backend why the
/// film had not arrived, got nothing, and printed the one sentence that fits
/// every case and explains none of them.
fn respond_stalled(
    request: tiny_http::Request,
    failure: &Failure,
    said: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let seconds = PLAYLIST_WAIT.as_secs();
    let said = if said.is_empty() {
        format!("Nothing playable after {seconds} seconds of conversion.")
    } else {
        format!("Nothing playable after {seconds} seconds. ffmpeg said: {said}")
    };
    eprintln!("stream: {said}");
    *failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(said);
    respond_status(request, 504)
}

/// `/hls/{token}/{session}/{file}` — the playlist and the segments beside it.
fn handle_hls(
    token: &str,
    session: &Session,
    failure: &Failure,
    rest: &str,
    request: tiny_http::Request,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut parts = rest.splitn(3, '/');
    let (Some(given), Some(id), Some(name)) = (parts.next(), parts.next(), parts.next()) else {
        return respond_status(request, 404);
    };
    if given != token {
        return respond_status(request, 403);
    }
    if !paths::is_plain_name(id) || !paths::is_plain_name(name) {
        return respond_status(request, 404);
    }

    // Only the session that is currently playing can be read: an id from an
    // abandoned one names a directory that no longer exists.
    let dir = match locked(session).as_ref() {
        Some(current) if current.id == id => current.dir.clone(),
        _ => return respond_status(request, 404),
    };
    let file = dir.join(name);

    // ffmpeg writes the playlist only once the first segment is closed, and the
    // player asks for it as soon as the URL reaches the element. Waiting here is
    // what turns that race into a slightly slower start instead of an error.
    if name == ffmpeg::HLS_PLAYLIST {
        let deadline = Instant::now() + PLAYLIST_WAIT;
        loop {
            if let Some(bytes) = playlist_snapshot(&file) {
                // The playlist is there and whole, but it may have stopped
                // growing. This is the poll the player makes every few seconds,
                // so it is also where a conversion that died mid-film shows up.
                if let Some(said) = conversion_failure(session, id) {
                    return respond_failure(request, failure, said);
                }
                // A playlist in hand is the answer to whatever was recorded
                // while it was not. The wait below records a reason at the
                // deadline and the film goes on to play anyway, because hls.js
                // asks again: left standing, that sentence is what the window
                // would print for some later, unrelated failure.
                *failure.lock().unwrap_or_else(|e| e.into_inner()) = None;
                return respond_playlist(request, bytes);
            }
            // Still ours? A session replaced mid-wait has had its directory
            // removed, and waiting out the deadline for it helps nobody.
            //
            // Asked before the deadline, not after: `respond_stalled` writes
            // into the failure every session shares, so timing out on a session
            // the user has already seeked away from would hang that sentence on
            // the one now playing.
            match locked(session).as_ref() {
                Some(current) if current.id == id => {}
                _ => return respond_status(request, 404),
            }
            // A conversion that refused the file is not slowness: waiting the
            // rest of the deadline for a playlist that will never be written
            // only delays the sentence explaining why.
            if let Some(said) = conversion_failure(session, id) {
                return respond_failure(request, failure, said);
            }
            if Instant::now() >= deadline {
                let said = said_so_far(session, id);
                return respond_stalled(request, failure, said);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    if !file.is_file() {
        return respond_status(request, 404);
    }
    serve_file(request, &file)
}

fn handle(
    config: &StreamConfig,
    token: &str,
    session: &Session,
    failure: &Failure,
    request: tiny_http::Request,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = request.url().to_string();

    if let Some(rest) = url
        .split('?')
        .next()
        .unwrap_or("")
        .strip_prefix("/hls/")
        .map(str::to_string)
    {
        return handle_hls(token, session, failure, &rest, request);
    }

    let query = parse_query(&url);

    // Constant-time comparison is overkill here — the token is not a password
    // and an attacker gets no oracle — but a wrong token must reveal nothing.
    if param(&query, "token").as_deref() != Some(token) {
        return respond_status(request, 403);
    }

    let Some(relative) = param(&query, "path") else {
        return respond_status(request, 400);
    };

    // The same check the play command uses: a URL from the webview must not be
    // able to name a file outside the media folder.
    let file = match paths::resolve_media_file(&config.media_root, &relative) {
        Ok(file) => file,
        Err(_) => return respond_status(request, 404),
    };

    let path = url.split('?').next().unwrap_or("");
    match path {
        "/subtitle" => {
            let track = track_param(&query, "track");
            // Run to the end rather than piping. A whole film's captions are a
            // few hundred kilobytes, so there is nothing to stream, and the
            // pipe had no way to fail: the head went out as 200 before ffmpeg
            // had read a byte, so a track it could not extract arrived as an
            // empty file. An empty WebVTT is a track that switches on and shows
            // nothing, which is indistinguishable from a broken player.
            let done = ffmpeg::command(&config.ffmpeg)
                .args(ffmpeg::subtitle_args(&file, track, start_param(&query)))
                .stdin(Stdio::null())
                .output()?;

            if !done.status.success() {
                let said = String::from_utf8_lossy(&done.stderr);
                eprintln!(
                    "stream: subtitle track {track} could not be read: {}",
                    said.trim()
                );
                return respond_status(request, 500);
            }
            if done.stdout.is_empty() {
                eprintln!("stream: subtitle track {track} came back empty");
                return respond_status(request, 500);
            }

            let length = done.stdout.len();
            let response = tiny_http::Response::new(
                200.into(),
                vec![header("Content-Type", "text/vtt")?, header(CORS.0, CORS.1)?],
                io::Cursor::new(done.stdout),
                Some(length),
                None,
            );
            Ok(request.respond(response)?)
        }
        "/stream" => {
            let start = start_param(&query);
            let delivery = param(&query, "delivery")
                .as_deref()
                .and_then(delivery_from_name)
                .unwrap_or(Delivery::Remux);

            // Direct means the file already plays as it is. Handing over the
            // bytes — rather than pushing them through ffmpeg — is what makes
            // the element's own seek bar work, and it is the only way a WebM
            // stays a WebM instead of being rewrapped into a container its
            // codecs are not allowed in.
            if delivery == Delivery::Direct {
                return serve_file(request, &file);
            }

            // Everything else now travels as HLS, so no URL the app builds
            // reaches this: it is the fMP4-on-a-pipe form, which `examples/
            // stream.rs` still exercises and which WebKit will not play.
            let vcodec = param(&query, "vcodec").filter(|c| !c.is_empty());
            let child = ffmpeg::command(&config.ffmpeg)
                .args(ffmpeg::stream_args(
                    &file,
                    delivery,
                    start,
                    vcodec.as_deref(),
                    track_param(&query, "audio"),
                ))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()?;
            pipe_to_client(request, child, "video/mp4")
        }
        _ => respond_status(request, 404),
    }
}

/// What a `Range` header asks for, once checked against the file size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wanted {
    /// No range, or one malformed enough that RFC 9110 says to ignore it.
    Whole,
    /// Inclusive byte offsets, both inside the file.
    Bytes(u64, u64),
    /// Well-formed but starting past the end.
    Unsatisfiable,
}

/// Parse a single-range `bytes=` header. Multi-range requests are answered with
/// the whole file, which is allowed and which no media element asks for.
pub fn parse_range(value: &str, size: u64) -> Wanted {
    let Some(spec) = value.trim().strip_prefix("bytes=") else {
        return Wanted::Whole;
    };
    if spec.contains(',') {
        return Wanted::Whole;
    }
    let Some((from, to)) = spec.split_once('-') else {
        return Wanted::Whole;
    };
    let (from, to) = (from.trim(), to.trim());

    // "bytes=-500" is the last 500 bytes, not a range starting at zero.
    if from.is_empty() {
        return match to.parse::<u64>() {
            Ok(0) => Wanted::Unsatisfiable,
            Ok(last) if size > 0 => Wanted::Bytes(size.saturating_sub(last), size - 1),
            _ => Wanted::Whole,
        };
    }

    let Ok(start) = from.parse::<u64>() else {
        return Wanted::Whole;
    };
    if start >= size {
        return Wanted::Unsatisfiable;
    }
    let end = match to {
        "" => size - 1,
        _ => match to.parse::<u64>() {
            Ok(end) => end.min(size - 1),
            Err(_) => return Wanted::Whole,
        },
    };
    if end < start {
        return Wanted::Unsatisfiable;
    }
    Wanted::Bytes(start, end)
}

/// Media type from the extension: the containers `Delivery::Direct` can choose,
/// plus the two an HLS session produces.
pub fn content_type_for(file: &Path) -> &'static str {
    match file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        // Safari decides whether a URL is a playlist from this header alone;
        // served as text it is downloaded rather than played.
        "m3u8" => "application/vnd.apple.mpegurl",
        // The initialisation segment is a bare .mp4, the media segments are
        // .m4s: both are fragmented MP4 and both are announced as such.
        "mp4" | "m4v" | "m4s" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

/// Serve the file itself, honouring `Range` so the player can seek in it.
fn serve_file(request: tiny_http::Request, file: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut handle = std::fs::File::open(file)?;
    let size = handle.metadata()?.len();

    let wanted = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Range"))
        .map(|h| parse_range(h.value.as_str(), size))
        .unwrap_or(Wanted::Whole);

    let common = vec![
        header("Content-Type", content_type_for(file))?,
        header("Accept-Ranges", "bytes")?,
        header(CORS.0, CORS.1)?,
    ];

    // The playlist no longer comes through here: it is read in one go and
    // answered by `respond_playlist`, which carries this header itself. What is
    // left of this route is files that do not change while they are being read.

    match wanted {
        Wanted::Unsatisfiable => {
            let mut headers = common;
            headers.push(header("Content-Range", &format!("bytes */{size}"))?);
            let response =
                tiny_http::Response::new(416.into(), headers, io::empty(), Some(0), None);
            Ok(request.respond(response)?)
        }
        Wanted::Whole => {
            let response =
                tiny_http::Response::new(200.into(), common, handle, Some(size as usize), None);
            Ok(request.respond(response)?)
        }
        Wanted::Bytes(start, end) => {
            handle.seek(SeekFrom::Start(start))?;
            let length = end - start + 1;
            let mut headers = common;
            headers.push(header(
                "Content-Range",
                &format!("bytes {start}-{end}/{size}"),
            )?);
            let response = tiny_http::Response::new(
                206.into(),
                headers,
                handle.take(length),
                Some(length as usize),
                None,
            );
            Ok(request.respond(response)?)
        }
    }
}

pub fn delivery_name(delivery: Delivery) -> &'static str {
    match delivery {
        Delivery::Direct => "direct",
        Delivery::Remux => "remux",
        Delivery::TranscodeAudio => "transcode-audio",
        Delivery::Transcode => "transcode",
    }
}

fn delivery_from_name(name: &str) -> Option<Delivery> {
    match name {
        "direct" => Some(Delivery::Direct),
        "remux" => Some(Delivery::Remux),
        "transcode-audio" => Some(Delivery::TranscodeAudio),
        "transcode" => Some(Delivery::Transcode),
        _ => None,
    }
}

/// Copy ffmpeg's stdout to the client until either end stops.
///
/// No Content-Length: the size is unknown until the conversion finishes, so the
/// body runs until the connection closes and the player treats it as a live
/// stream. Seeking is handled by asking for a new stream at an offset, not by
/// Range requests — hence no `Accept-Ranges` here.
fn pipe_to_client(
    request: tiny_http::Request,
    mut child: Child,
    content_type: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Every way out of the copy goes through here. Writing the head can fail on
    // its own — a player that asked for a subtitle track and moved on before the
    // first byte leaves a closed socket — and returning from inside it left an
    // ffmpeg running that nothing else in the process knows about.
    let result = pipe_body(request, &mut child, content_type);
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn pipe_body(
    request: tiny_http::Request,
    child: &mut Child,
    content_type: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut stdout = child.stdout.take().ok_or("ffmpeg produced no output")?;

    let mut writer = request.into_writer();
    writer.write_all(b"HTTP/1.1 200 OK\r\n")?;
    writer.write_all(format!("Content-Type: {content_type}\r\n").as_bytes())?;
    writer.write_all(format!("{}: {}\r\n", CORS.0, CORS.1).as_bytes())?;
    writer.write_all(b"Connection: close\r\n\r\n")?;

    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        match stdout.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                // A closed pipe means the player moved on — seek, next episode,
                // or window closed. Stop ffmpeg instead of leaving it encoding
                // into nothing.
                if writer.write_all(&buffer[..read]).is_err() {
                    break;
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // Only the tests build a command by hand now: `process_alive` asks the
    // system about a pid. Everything the app itself runs goes through
    // `ffmpeg::command`, which is what the guard below keeps true.
    use std::process::Command;

    #[test]
    fn a_query_is_split_and_decoded() {
        let query = parse_query("/stream?token=abc&path=Amelie%20%282001%29%2Fa.mkv&t=12.5");
        assert_eq!(param(&query, "token").as_deref(), Some("abc"));
        assert_eq!(
            param(&query, "path").as_deref(),
            Some("Amelie (2001)/a.mkv")
        );
        assert_eq!(param(&query, "t").as_deref(), Some("12.5"));
    }

    #[test]
    fn encoding_survives_a_round_trip() {
        for value in [
            "Amélie (2001)/Amélie.mkv",
            "Scrubs/Season 1/S01E01 - My First Day.mkv",
            "a&b=c?d#e",
            "100% Wolf/film.mkv",
        ] {
            assert_eq!(urldecode(&urlencode(value)), value, "value: {value}");
        }
    }

    #[test]
    fn a_truncated_escape_does_not_panic() {
        for broken in ["%", "%A", "%ZZ", "a%"] {
            let _ = urldecode(broken);
        }
    }

    #[test]
    fn a_chosen_audio_track_rules_out_handing_the_file_over() {
        // The untouched file carries every track and nothing downstream picks
        // one: the element plays whichever the container names first. Being
        // handed the file after asking for the Italian track is a silent no-op
        // — the sound stays in the wrong language and nothing says so. The same
        // hole as an offset the file cannot honour, on the other parameter, so
        // it is closed in the same place: `media::delivery_at`.
        let (server, _dir) = hls_server("exit 0");

        let first = server
            .stream_url("Movie/Movie.mkv", 0.0, Delivery::Direct, None, 0)
            .expect("url");
        assert!(
            first.contains("delivery=direct"),
            "track 0 is what the file plays anyway, so the cheap path stands: {first}"
        );

        let dubbed = server
            .stream_url("Movie/Movie.mkv", 0.0, Delivery::Direct, None, 1)
            .expect("url");
        assert!(
            !dubbed.contains("delivery=direct"),
            "a track the direct route cannot honour: {dubbed}"
        );
        assert!(dubbed.contains("/hls/"), "{dubbed}");
    }

    #[test]
    fn a_start_offset_rules_out_handing_the_file_over() {
        // A direct URL is the file itself: the server answers it with the bytes
        // from the first one and `t` is never read. Handing one back for a
        // stream that has to begin at 631 s plays the film from the opening
        // while the player anchors its clock at 631 — the picture and the seek
        // bar then disagree for the rest of the film, with no error anywhere.
        let (server, _dir) = hls_server("exit 0");

        let from_the_start = server
            .stream_url("Movie/Movie.mkv", 0.0, Delivery::Direct, None, 0)
            .expect("url");
        assert!(
            from_the_start.contains("delivery=direct"),
            "the whole point of Direct is still the cheap path: {from_the_start}"
        );

        let midway = server
            .stream_url("Movie/Movie.mkv", 631.5, Delivery::Direct, None, 0)
            .expect("url");
        assert!(
            !midway.contains("delivery=direct"),
            "an offset the direct route cannot honour: {midway}"
        );
        assert!(midway.contains("/hls/"), "{midway}");
    }

    #[test]
    fn an_unreadable_track_number_reads_as_the_first_track() {
        let query = vec![("audio".to_string(), "second".to_string())];
        assert_eq!(track_param(&query, "audio"), 0);
        assert_eq!(track_param(&[], "audio"), 0);
    }

    #[test]
    fn a_subtitle_url_carries_the_offset_its_stream_begins_at() {
        // The track and the picture have to land on the same clock: a URL that
        // loses the offset here is a subtitle that never appears again after a
        // seek, and nothing reports it.
        let (server, _dir) = direct_server();
        let url = server.subtitle_url("Movie/Movie.mp4", 2, 631.5);
        let query = parse_query(&url);
        assert_eq!(param(&query, "track").as_deref(), Some("2"));
        assert_eq!(start_param(&query), 631.5);

        assert_eq!(
            start_param(&parse_query(&server.subtitle_url(
                "Movie/Movie.mp4",
                0,
                0.0
            ))),
            0.0
        );
    }

    #[test]
    fn an_unusable_offset_reads_as_the_beginning() {
        for value in ["", "abc", "-5", "NaN", "inf"] {
            let query = parse_query(&format!("/subtitle?t={value}"));
            assert_eq!(start_param(&query), 0.0, "t={value}");
        }
        assert_eq!(start_param(&parse_query("/subtitle")), 0.0);
    }

    #[test]
    fn a_missing_parameter_reads_as_absent() {
        let query = parse_query("/stream?token=abc");
        assert_eq!(param(&query, "path"), None);
        assert_eq!(parse_query("/stream").len(), 0);
    }

    #[test]
    fn every_delivery_name_survives_the_round_trip() {
        for delivery in [
            Delivery::Direct,
            Delivery::Remux,
            Delivery::TranscodeAudio,
            Delivery::Transcode,
        ] {
            assert_eq!(
                delivery_from_name(delivery_name(delivery)),
                Some(delivery),
                "a name the server cannot read means silent fallback to remux"
            );
        }
    }

    #[test]
    fn delivery_names_match_the_serialised_form() {
        // These strings cross the IPC boundary as the serde representation of
        // Delivery; a mismatch would silently fall back to remuxing.
        assert_eq!(delivery_from_name("direct"), Some(Delivery::Direct));
        assert_eq!(delivery_from_name("remux"), Some(Delivery::Remux));
        assert_eq!(
            delivery_from_name("transcode-audio"),
            Some(Delivery::TranscodeAudio)
        );
        assert_eq!(delivery_from_name("transcode"), Some(Delivery::Transcode));
        assert_eq!(delivery_from_name("nonsense"), None);
    }

    #[test]
    fn a_range_header_is_read_as_inclusive_offsets() {
        assert_eq!(parse_range("bytes=0-99", 1000), Wanted::Bytes(0, 99));
        assert_eq!(parse_range("bytes=500-", 1000), Wanted::Bytes(500, 999));
        assert_eq!(parse_range(" bytes=10-20 ", 1000), Wanted::Bytes(10, 20));
        // A player asking past the end of the file still gets the tail.
        assert_eq!(parse_range("bytes=900-5000", 1000), Wanted::Bytes(900, 999));
        // Suffix form: the last N bytes, which is how MP4 readers find an index.
        assert_eq!(parse_range("bytes=-100", 1000), Wanted::Bytes(900, 999));
        assert_eq!(parse_range("bytes=-5000", 1000), Wanted::Bytes(0, 999));
    }

    #[test]
    fn a_range_starting_past_the_end_is_refused_not_clamped() {
        // Clamping would hand back the wrong bytes under a 206, which the
        // player would splice into the file as if they belonged there.
        assert_eq!(parse_range("bytes=1000-1010", 1000), Wanted::Unsatisfiable);
        assert_eq!(parse_range("bytes=20-10", 1000), Wanted::Unsatisfiable);
        assert_eq!(parse_range("bytes=-0", 1000), Wanted::Unsatisfiable);
        assert_eq!(parse_range("bytes=0-0", 1), Wanted::Bytes(0, 0));
    }

    #[test]
    fn an_unusable_range_falls_back_to_the_whole_file() {
        for value in [
            "",
            "bytes",
            "bytes=",
            "bytes=abc-def",
            "items=0-10",
            "bytes=0-10, 20-30",
            "bytes=0-abc",
        ] {
            assert_eq!(parse_range(value, 1000), Wanted::Whole, "value: {value}");
        }
        assert_eq!(parse_range("bytes=0-99", 0), Wanted::Unsatisfiable);
    }

    #[test]
    fn a_served_file_keeps_the_media_type_of_its_container() {
        assert_eq!(content_type_for(Path::new("/m/a.mp4")), "video/mp4");
        assert_eq!(content_type_for(Path::new("/m/a.M4V")), "video/mp4");
        assert_eq!(content_type_for(Path::new("/m/a.mov")), "video/quicktime");
        // Rewrapping a WebM into MP4 would strand VP9 in a container browsers
        // will not decode it from, so the type must survive untouched.
        assert_eq!(content_type_for(Path::new("/m/a.webm")), "video/webm");
    }

    /// Minimal HTTP client: the point is to exercise the real socket, so no
    /// dependency and no parsing beyond splitting head from body.
    fn get(port: u16, target: &str, extra: &str) -> (String, Vec<u8>) {
        use std::net::TcpStream;
        let mut socket = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        socket
            .write_all(
                format!(
                    "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n{extra}\r\n"
                )
                .as_bytes(),
            )
            .expect("send");
        let mut raw = Vec::new();
        socket.read_to_end(&mut raw).expect("read");
        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("headers end");
        (
            String::from_utf8_lossy(&raw[..split]).into_owned(),
            raw[split + 4..].to_vec(),
        )
    }

    fn direct_server() -> (StreamServer, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        std::fs::create_dir(dir.path().join("Movie")).expect("mkdir");
        std::fs::write(dir.path().join("Movie/Movie.mp4"), b"0123456789").expect("write");
        let server = StreamServer::start(StreamConfig {
            media_root: dir.path().to_path_buf(),
            // Never reached: every request in these tests is a direct one.
            ffmpeg: PathBuf::from("ffmpeg"),
        })
        .expect("start");
        (server, dir)
    }

    #[test]
    fn a_direct_file_is_served_whole_and_by_range() {
        let (server, _dir) = direct_server();
        let url = server
            .stream_url("Movie/Movie.mp4", 0.0, Delivery::Direct, Some("h264"), 0)
            .expect("url");
        let target = url.split_once("/stream").expect("path").1;
        let target = format!("/stream{target}");

        let (head, body) = get(server.port, &target, "");
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(head.contains("Accept-Ranges: bytes"), "{head}");
        assert!(head.contains("Content-Type: video/mp4"), "{head}");
        assert_eq!(body, b"0123456789");

        // Without this the element reports nothing seekable and the scrub bar
        // moves without the picture following.
        let (head, body) = get(server.port, &target, "Range: bytes=2-5\r\n");
        assert!(head.starts_with("HTTP/1.1 206"), "{head}");
        assert!(head.contains("Content-Range: bytes 2-5/10"), "{head}");
        assert_eq!(body, b"2345");

        let (head, _) = get(server.port, &target, "Range: bytes=99-\r\n");
        assert!(head.starts_with("HTTP/1.1 416"), "{head}");
    }

    #[test]
    fn the_server_answers_only_with_the_token_and_only_inside_the_media_root() {
        let (server, _dir) = direct_server();

        let (head, _) = get(server.port, "/stream?path=Movie%2FMovie.mp4", "");
        assert!(head.starts_with("HTTP/1.1 403"), "{head}");

        let escape = format!(
            "/stream?token={}&path={}&delivery=direct",
            server.token,
            urlencode("../../etc/passwd")
        );
        let (head, _) = get(server.port, &escape, "");
        assert!(head.starts_with("HTTP/1.1 404"), "{head}");

        let unknown = format!("/nope?token={}&path=Movie%2FMovie.mp4", server.token);
        let (head, _) = get(server.port, &unknown, "");
        assert!(head.starts_with("HTTP/1.1 404"), "{head}");
    }

    #[test]
    fn every_response_lets_the_webview_read_it() {
        // A `<track>` from another origin is dropped without this header, and
        // the subtitles simply never appear — with nothing logged anywhere.
        let (server, _dir) = direct_server();
        let ok = format!(
            "/stream?token={}&path=Movie%2FMovie.mp4&delivery=direct",
            server.token
        );
        for target in [ok.as_str(), "/stream?token=wrong&path=x"] {
            let (head, _) = get(server.port, target, "");
            assert!(
                head.contains("Access-Control-Allow-Origin: *"),
                "target {target}: {head}"
            );
        }
    }

    /// The compiled stand-in for ffmpeg, beside the test binary.
    ///
    /// A real process rather than a shell script, which is what lets every test
    /// below run on Windows: `#!/bin/sh` is not something Windows executes, and
    /// gating them all on unix left the playback path untested on the one
    /// platform it was broken on. `cargo test` builds the examples, so it is
    /// already there; `cargo test --lib` does not, hence the sentence.
    fn fake_ffmpeg() -> PathBuf {
        let test_binary = std::env::current_exe().expect("current exe");
        let target = test_binary
            .parent()
            .and_then(Path::parent)
            .expect("target directory");
        let tool = target
            .join("examples")
            .join(format!("fake_ffmpeg{}", std::env::consts::EXE_SUFFIX));
        assert!(
            tool.is_file(),
            "{} is missing: run `cargo test`, not `cargo test --lib`, so the \
             examples are built",
            tool.display()
        );
        tool
    }

    /// A session whose "ffmpeg" carries out `plan`: the routing, the waiting and
    /// the headers are what is under test, and a real conversion would test
    /// ffmpeg instead.
    ///
    /// The plan is the content of the fake film, because that is the only
    /// channel the stand-in has: the command line belongs to the code under
    /// test. See `examples/fake_ffmpeg.rs` for the directives.
    fn hls_server(plan: &str) -> (StreamServer, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        std::fs::create_dir(dir.path().join("Movie")).expect("mkdir");
        std::fs::write(dir.path().join("Movie/Movie.mkv"), plan).expect("write");
        let server = StreamServer::start(StreamConfig {
            media_root: dir.path().to_path_buf(),
            ffmpeg: fake_ffmpeg(),
        })
        .expect("start");
        (server, dir)
    }

    /// The playlist path of a session URL, without the host.
    fn target_of(url: &str) -> String {
        let after_host = url.split_once("127.0.0.1:").expect("host").1;
        format!("/{}", after_host.split_once('/').expect("path").1)
    }

    #[test]
    fn a_session_serves_its_playlist_and_segments_with_the_types_safari_needs() {
        let (server, _dir) = hls_server(
            r#"file init.mp4 x
file seg00000.m4s segment
playlist #EXTM3U\n#EXT-X-MAP:URI="init.mp4"\nseg00000.m4s\n
sleep 30"#,
        );

        let url = server
            .stream_url("Movie/Movie.mkv", 0.0, Delivery::Remux, Some("h264"), 0)
            .expect("session");
        assert!(url.contains("/hls/"), "{url}");
        let playlist = target_of(&url);

        let (head, body) = get(server.port, &playlist, "");
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(
            head.contains("Content-Type: application/vnd.apple.mpegurl"),
            "served as anything else, Safari downloads it instead of playing it: {head}"
        );
        assert!(String::from_utf8_lossy(&body).contains("seg00000.m4s"),);

        // The playlist names its segments relatively, so the player asks for
        // them beside it — token included, because the token is in the path.
        let base = playlist.rsplit_once('/').expect("directory").0.to_string();
        for (name, expected) in [("seg00000.m4s", "segment"), ("init.mp4", "x")] {
            let (head, body) = get(server.port, &format!("{base}/{name}"), "");
            assert!(head.starts_with("HTTP/1.1 200"), "{name}: {head}");
            assert!(head.contains("Content-Type: video/mp4"), "{name}: {head}");
            assert_eq!(body, expected.as_bytes(), "{name}");
        }
    }

    #[test]
    fn a_session_is_reachable_only_with_the_token_and_only_by_name() {
        let (server, _dir) = hls_server(
            r#"playlist #EXTM3U\n
sleep 30"#,
        );
        let url = server
            .stream_url("Movie/Movie.mkv", 0.0, Delivery::Remux, None, 0)
            .expect("session");
        let playlist = target_of(&url);
        let base = playlist.rsplit_once('/').expect("directory").0.to_string();

        let wrong_token = playlist.replacen(&server.token, "0".repeat(32).as_str(), 1);
        let (head, _) = get(server.port, &wrong_token, "");
        assert!(head.starts_with("HTTP/1.1 403"), "{head}");

        // The session directory is ours, so the name is the whole defence.
        for escape in ["..%2F..%2Fetc%2Fpasswd", "../../etc/passwd", ".."] {
            let (head, _) = get(server.port, &format!("{base}/{escape}"), "");
            assert!(head.starts_with("HTTP/1.1 404"), "escape {escape}: {head}");
        }

        // An unknown session names a directory that no longer exists.
        let stale = format!("/hls/{}/{}/index.m3u8", server.token, "a".repeat(32));
        let (head, _) = get(server.port, &stale, "");
        assert!(head.starts_with("HTTP/1.1 404"), "{head}");

        let (head, _) = get(server.port, &format!("{base}/index.m3u8"), "");
        assert!(head.contains("Access-Control-Allow-Origin: *"), "{head}");
    }

    #[test]
    fn a_new_session_stops_the_one_it_replaces() {
        // The stand-in outlives its usefulness on purpose: if replacing a
        // session did not kill it, it would still be running at the end.
        let (server, _dir) = hls_server(
            r#"playlist #EXTM3U\n
sleep 120"#,
        );

        let first = server
            .stream_url("Movie/Movie.mkv", 0.0, Delivery::Remux, None, 0)
            .expect("session");
        let (head, _) = get(server.port, &target_of(&first), "");
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");

        let (old_dir, old_pid) = {
            let guard = locked(&server.session);
            let session = guard.as_ref().expect("a session");
            (session.dir.clone(), session.child.id())
        };
        // The helper has to be able to answer "yes" before either "no" below
        // means anything. On Windows it asks `tasklist`, and a `tasklist` that
        // is missing, refused, or answering in another shape falls through to
        // `false`: both assertions would then pass without anything ever
        // having been killed, on the one platform this whole file exists for.
        assert!(
            process_alive(old_pid),
            "the first conversion is not running, so nothing below is a test"
        );

        let second = server
            .stream_url("Movie/Movie.mkv", 600.0, Delivery::Remux, None, 0)
            .expect("session");
        assert_ne!(first, second, "a seek must not reuse the old session");

        assert!(
            !old_dir.exists(),
            "the segments of the old session are still on disk"
        );
        // The old id now names nothing.
        let (head, _) = get(server.port, &target_of(&first), "");
        assert!(head.starts_with("HTTP/1.1 404"), "{head}");
        assert!(
            !process_alive(old_pid),
            "the superseded ffmpeg is still running: pid {old_pid}"
        );

        // And stopping the server takes the live one with it.
        let (live_dir, live_pid) = {
            let guard = locked(&server.session);
            let session = guard.as_ref().expect("a session");
            (session.dir.clone(), session.child.id())
        };
        server.stop();
        assert!(!live_dir.exists(), "{live_dir:?} outlived the server");
        assert!(
            !process_alive(live_pid),
            "pid {live_pid} outlived the server"
        );
    }

    #[test]
    fn stopping_a_session_lets_go_of_the_lock_before_tearing_it_down() {
        // The same rule `start_hls` spells out, on the other way out. The
        // teardown kills ffmpeg, waits for it and deletes every segment the
        // conversion wrote, and every request thread takes this lock to find
        // the directory it is serving from: run under the lock, a film left
        // after two hours holds up the next one for as long as the delete
        // takes. Assigning through the guard is what does that, and it is one
        // character away from being right.
        let (server, _dir) = hls_server(
            r#"playlist #EXTM3U\n
sleep 30"#,
        );
        server
            .stream_url("Movie/Movie.mkv", 0.0, Delivery::Remux, None, 0)
            .expect("session");

        // A long conversion is what makes the teardown slow, and its cost is
        // the segment count; four thousand of them stands in for it.
        let dir = locked(&server.session)
            .as_ref()
            .expect("a session")
            .dir
            .clone();
        for i in 0..4000 {
            std::fs::write(dir.join(format!("seg{i:05}.m4s")), b"x").expect("segment");
        }

        // The signal is both halves at once: the lock free *and* the session
        // already lifted out of it, while the directory is still being
        // removed. Free but still holding the session is only this thread
        // winning the race to the lock before `stop` reached it.
        let session = Arc::clone(&server.session);
        let watched = dir.clone();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let start = Arc::clone(&barrier);
        let watcher = std::thread::spawn(move || {
            start.wait();
            let deadline = Instant::now() + Duration::from_secs(10);
            while watched.exists() && Instant::now() < deadline {
                if session.try_lock().is_ok_and(|held| held.is_none()) {
                    return true;
                }
                std::thread::yield_now();
            }
            false
        });

        barrier.wait();
        server.stop();

        assert!(
            watcher.join().expect("watcher"),
            "the session lock was held for the whole teardown"
        );
        assert!(!dir.exists(), "the session directory outlived the stop");
    }

    /// `kill -0`: the process is gone once this fails. A zombie would still
    /// answer, which is why every teardown waits after killing.
    #[cfg(unix)]
    fn process_alive(pid: u32) -> bool {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    /// The same question where there is no `kill`. `tasklist` answers a pid it
    /// cannot find with a sentence rather than a failure, so the pid itself has
    /// to appear in the output for the process to count as alive.
    #[cfg(windows)]
    fn process_alive(pid: u32) -> bool {
        Command::new("tasklist")
            .args(["/NH", "/FI", &format!("PID eq {pid}")])
            .stderr(Stdio::null())
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }

    #[test]
    fn a_playlist_that_never_arrives_times_out_instead_of_hanging() {
        // ffmpeg refusing the file leaves the directory empty; the request must
        // end by itself rather than holding the connection open for ever.
        // It ends early *because* the conversion is dead — waiting out the rest
        // of the deadline for a playlist nobody is writing helps nobody.
        let (server, _dir) = hls_server(
            r#"stderr Invalid data found\n
exit 1"#,
        );
        let url = server
            .stream_url("Movie/Movie.mkv", 0.0, Delivery::Remux, None, 0)
            .expect("session");
        let started = Instant::now();
        let (head, _) = get(server.port, &target_of(&url), "");
        assert!(head.starts_with("HTTP/1.1 500"), "{head}");
        assert!(
            started.elapsed() < PLAYLIST_WAIT,
            "waited the whole deadline"
        );
        assert_eq!(
            server.last_failure().as_deref(),
            Some("Invalid data found"),
            "the one sentence explaining the failure has to survive the request"
        );
    }

    #[test]
    fn a_conversion_that_dies_halfway_is_reported_rather_than_left_to_stall() {
        // The failure the user actually meets: one segment is written, ffmpeg
        // gives up, and the playlist never grows again. The element goes on
        // polling the same playlist for ever and the film simply stops partway
        // through, with nothing on screen and nothing in the log to say why.
        let (server, _dir) = hls_server(
            r#"file init.mp4 x
file seg00000.m4s segment
playlist #EXTM3U\n#EXT-X-MAP:URI="init.mp4"\n#EXTINF:4.000000,\nseg00000.m4s\n
stderr Error while decoding stream #0:0\n
exit 1"#,
        );

        let url = server
            .stream_url("Movie/Movie.mkv", 0.0, Delivery::Transcode, None, 0)
            .expect("session");
        let playlist = target_of(&url);

        // The playlist itself is fine and may well be served before the child
        // is reaped; the poll that follows is where the death shows up.
        let deadline = Instant::now() + Duration::from_secs(5);
        let head = loop {
            let (head, _) = get(server.port, &playlist, "");
            if head.starts_with("HTTP/1.1 500") || Instant::now() >= deadline {
                break head;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(
            head.starts_with("HTTP/1.1 500"),
            "a dead conversion must fail the poll instead of serving a playlist \
             that will never grow: {head}"
        );
        assert_eq!(
            server.last_failure().as_deref(),
            Some("Error while decoding stream #0:0")
        );
    }

    #[test]
    fn a_conversion_that_is_still_running_is_not_called_dead() {
        // The opposite mistake would be worse than the bug: reporting a failure
        // for every poll would end a film that was only being converted slowly.
        let (server, _dir) = hls_server(
            r#"playlist #EXTM3U\n
sleep 30"#,
        );
        let url = server
            .stream_url("Movie/Movie.mkv", 0.0, Delivery::Remux, None, 0)
            .expect("session");
        for _ in 0..3 {
            let (head, _) = get(server.port, &target_of(&url), "");
            assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        }
        assert_eq!(server.last_failure(), None);
    }

    #[test]
    fn a_conversion_that_finished_cleanly_is_not_called_dead() {
        // Exit 0 is how every complete film ends: the playlist carries its
        // end-of-list marker and the last poll must still be answered with it.
        let (server, _dir) = hls_server(
            r#"file init.mp4 x
file seg00000.m4s segment
playlist #EXTM3U\n#EXTINF:4.000000,\nseg00000.m4s\n#EXT-X-ENDLIST\n
exit 0"#,
        );
        let url = server
            .stream_url("Movie/Movie.mkv", 0.0, Delivery::Remux, None, 0)
            .expect("session");
        std::thread::sleep(Duration::from_millis(300));
        let (head, body) = get(server.port, &target_of(&url), "");
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(String::from_utf8_lossy(&body).contains("ENDLIST"));
        assert_eq!(server.last_failure(), None);
    }

    #[test]
    fn a_conversion_that_writes_nothing_in_time_says_so_instead_of_going_quiet() {
        // The bare 504 this replaces is what the user actually met: the player
        // gave up, asked the backend why, and the backend had recorded nothing
        // because ffmpeg had not failed, it was only slow. What reached the
        // screen was the one sentence that fits every failure and explains
        // none: "This file could not be played."
        let (server, _dir) = hls_server("sleep 30");
        let url = server
            .stream_url("Movie/Movie.mkv", 0.0, Delivery::Transcode, None, 0)
            .expect("session");

        let (head, _) = get(server.port, &target_of(&url), "");
        assert!(head.starts_with("HTTP/1.1 504"), "{head}");
        let said = server.last_failure().expect("a reason for the timeout");
        assert!(
            said.contains("Nothing playable"),
            "the reason has to name what happened: {said}"
        );
    }

    #[test]
    fn a_stalled_conversion_carries_whatever_ffmpeg_had_already_said() {
        // ffmpeg complaining without giving up is the interesting case: the
        // complaint is the only clue, and it would otherwise sit in a buffer
        // nobody reads until the process ends, which here it never does.
        let (server, _dir) = hls_server(
            r#"stderr Could not find a decoder for stream 0\n
sleep 30"#,
        );
        let url = server
            .stream_url("Movie/Movie.mkv", 0.0, Delivery::Transcode, None, 0)
            .expect("session");

        let (head, _) = get(server.port, &target_of(&url), "");
        assert!(head.starts_with("HTTP/1.1 504"), "{head}");
        let said = server.last_failure().expect("a reason");
        assert!(said.contains("Could not find a decoder"), "{said}");
    }

    #[test]
    fn a_timeout_the_conversion_went_on_to_disprove_is_not_kept_on_the_record() {
        // The 504 is deliberately not a failure, and hls.js is now told to ask
        // again: a conversion slower than the deadline times out once and then
        // plays. The sentence explaining that timeout must not outlive it. Left
        // standing it is what the window prints for the *next* failure, whatever
        // that turns out to be, because the player asks the backend for a reason
        // and gets the one recorded minutes earlier.
        let (server, _dir) = hls_server(
            r#"sleep 3
playlist #EXTM3U\n
sleep 30"#,
        );
        let url = server
            .stream_url("Movie/Movie.mkv", 0.0, Delivery::Transcode, None, 0)
            .expect("session");
        let playlist = target_of(&url);

        let (head, _) = get(server.port, &playlist, "");
        assert!(head.starts_with("HTTP/1.1 504"), "{head}");
        assert!(server.last_failure().is_some(), "the timeout is recorded");

        let (head, _) = get(server.port, &playlist, "");
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert_eq!(
            server.last_failure(),
            None,
            "a playlist in hand answers the timeout that came before it"
        );
    }

    #[test]
    fn a_playlist_is_whole_only_with_both_of_its_ends() {
        // The rule itself, on files that sit still. The test below drives the
        // same rule through a real conversion, but which half-written state a
        // request lands on there depends on when the request arrives: the
        // truncated body exists for three tenths of a second, and a loaded
        // machine can miss that window and pass without ever having read one.
        // Here every state is read on purpose.
        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = dir.path().join(ffmpeg::HLS_PLAYLIST);

        assert_eq!(
            playlist_snapshot(&file),
            None,
            "a playlist not yet written is not a playlist"
        );
        for cut in [
            // Caught before ffmpeg wrote anything.
            "",
            // Caught halfway through the last segment name: opens correctly,
            // and is still half a file.
            "#EXTM3U\n#EXTINF:4.000000,\nseg000",
            // The tag itself half written.
            "#EXT",
            // Every line closed and the tag missing: the exact body hls.js
            // calls a fatal parse error, and the one the trailing newline on
            // its own would let through.
            "#EXTINF:4.000000,\nseg00000.m4s\n",
        ] {
            std::fs::write(&file, cut).expect("write");
            assert_eq!(playlist_snapshot(&file), None, "served {cut:?}");
        }

        let whole = "#EXTM3U\n#EXTINF:4.000000,\nseg00000.m4s\n";
        std::fs::write(&file, whole).expect("write");
        assert_eq!(
            playlist_snapshot(&file).as_deref(),
            Some(whole.as_bytes()),
            "a whole playlist must be served as it is"
        );
    }

    #[test]
    fn a_playlist_caught_mid_rewrite_is_never_served_half_written() {
        // ffmpeg does not append to the playlist, it rewrites it whole after
        // every segment, and during a remux that is dozens of times a second.
        // A read landing inside one of those rewrites gets an empty or partial
        // file. WKWebView asks again and nobody notices; hls.js, which is what
        // plays on Windows and Linux, calls a body with no `#EXTM3U` a fatal
        // parse error and never asks again. One unlucky first request, and the
        // film is over before it started.
        let (server, _dir) = hls_server(
            // Both halves of what "whole" means, in the order a rewrite goes
            // through them: nothing at all, then a body that opens correctly and
            // stops halfway through the last segment name. Without the second
            // step the trailing-newline half of the rule could be deleted and
            // every test here would still pass.
            r#"file init.mp4 x
file seg00000.m4s segment
playlist
sleep 0.3
playlist #EXTM3U\n#EXTINF:4.000000,\nseg000
sleep 0.3
playlist #EXTM3U\n#EXTINF:4.000000,\nseg00000.m4s\n
sleep 30"#,
        );
        let url = server
            .stream_url("Movie/Movie.mkv", 0.0, Delivery::Remux, None, 0)
            .expect("session");

        let (head, body) = get(server.port, &target_of(&url), "");
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.starts_with("#EXTM3U"),
            "a playlist that is not a playlist: {text:?}"
        );
        // The other end, and the reason this is a separate assertion: a body
        // truncated after the segment name still opens with the tag, so the
        // check above passes on exactly the half-written file this test is
        // about.
        assert!(
            text.ends_with("seg00000.m4s\n"),
            "a playlist cut short is still not a playlist: {text:?}"
        );
        assert!(
            head.contains("Content-Type: application/vnd.apple.mpegurl"),
            "{head}"
        );
        assert!(head.contains("Access-Control-Allow-Origin: *"), "{head}");
    }

    #[test]
    fn a_growing_playlist_is_never_served_from_a_cache() {
        // The player asks for the same URL every few seconds and expects a
        // longer answer each time. Cached once, it plays what existed at the
        // first fetch and stops there.
        let (server, _dir) = hls_server(
            r#"file init.mp4 x
file seg00000.m4s segment
playlist #EXTM3U\n#EXTINF:4.000000,\nseg00000.m4s\n
sleep 30"#,
        );
        let url = server
            .stream_url("Movie/Movie.mkv", 0.0, Delivery::Remux, None, 0)
            .expect("session");
        let playlist = target_of(&url);
        let (head, _) = get(server.port, &playlist, "");
        assert!(head.contains("Cache-Control: no-cache"), "{head}");

        // A segment is written once and never changes, so it carries no such
        // header — caching those is the one thing that makes seeking cheap.
        let base = playlist.rsplit_once('/').expect("directory").0.to_string();
        let (head, _) = get(server.port, &format!("{base}/seg00000.m4s"), "");
        assert!(!head.contains("Cache-Control"), "{head}");
    }

    /// The one route that still answers straight from an ffmpeg pipe, and it had
    /// no test at all.
    ///
    /// The name no longer promises anything about stragglers: the check that
    /// looked for them is gone, for the reason spelled out at the end.
    #[test]
    fn a_subtitle_track_is_served_as_webvtt() {
        let (server, _dir) = hls_server(
            r#"stdout WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nCiao\n
exit 0"#,
        );
        let target = format!(
            "/subtitle?token={}&path=Movie%2FMovie.mkv&track=0",
            server.token
        );

        let (head, body) = get(server.port, &target, "");
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(head.contains("Content-Type: text/vtt"), "{head}");
        // A `<track>` from another origin is dropped without this, silently.
        assert!(head.contains("Access-Control-Allow-Origin: *"), "{head}");
        assert!(
            String::from_utf8_lossy(&body).starts_with("WEBVTT"),
            "{body:?}"
        );

        // No straggler check any more. It used to match processes by the path
        // of this test's own script, and every test now runs the same compiled
        // stand-in: the pattern would find the ones the other tests keep alive
        // on purpose, and fail here for their sake. What it guarded is
        // structural rather than incidental anyway, because `output()` waits
        // for the child before it returns the bytes.
    }

    #[test]
    fn a_subtitle_track_that_cannot_be_read_fails_instead_of_arriving_empty() {
        // The failure this closes: the response head went out before ffmpeg had
        // read a byte, so a track it refused arrived as an empty 200. The
        // element accepts that, the track switches on, and not one line ever
        // appears — which is exactly what "the subtitles do not work" looks
        // like from the outside.
        let (server, _dir) = hls_server(
            r#"stderr Stream map 0:s:0 matches no streams\n
exit 1"#,
        );
        let target = format!(
            "/subtitle?token={}&path=Movie%2FMovie.mkv&track=3",
            server.token
        );
        let (head, _) = get(server.port, &target, "");
        assert!(head.starts_with("HTTP/1.1 500"), "{head}");
    }

    #[test]
    fn a_subtitle_track_that_comes_back_empty_is_also_a_failure() {
        // ffmpeg exits 0 and writes nothing: a track that exists and holds no
        // cues in the range asked for. Served as an empty 200 it is the same
        // silent nothing.
        let (server, _dir) = hls_server("exit 0");
        let target = format!(
            "/subtitle?token={}&path=Movie%2FMovie.mkv&track=0",
            server.token
        );
        let (head, _) = get(server.port, &target, "");
        assert!(head.starts_with("HTTP/1.1 500"), "{head}");
    }

    #[test]
    fn only_a_bare_file_name_names_something_in_a_session() {
        assert!(paths::is_plain_name("index.m3u8"));
        assert!(paths::is_plain_name("seg00001.m4s"));
        assert!(paths::is_plain_name("init.mp4"));
        for bad in ["", ".", "..", "../x", "a/b", "a\\b", "%2e%2e", "a b", "~"] {
            assert!(!paths::is_plain_name(bad), "name: {bad}");
        }
    }

    #[test]
    fn an_hls_response_declares_the_playlist_and_segment_types() {
        assert_eq!(
            content_type_for(Path::new("/tmp/s/index.m3u8")),
            "application/vnd.apple.mpegurl"
        );
        assert_eq!(
            content_type_for(Path::new("/tmp/s/seg00000.m4s")),
            "video/mp4"
        );
        assert_eq!(content_type_for(Path::new("/tmp/s/init.mp4")), "video/mp4");
    }

    #[test]
    fn a_direct_file_keeps_its_own_url_rather_than_a_playlist() {
        // Direct is the one delivery that already plays, with ranges and native
        // seeking; putting it through a conversion would give up both.
        let (server, _dir) = direct_server();
        let url = server
            .stream_url("Movie/Movie.mp4", 0.0, Delivery::Direct, Some("h264"), 0)
            .expect("url");
        assert!(url.contains("/stream?"), "{url}");
        assert!(!url.contains("/hls/"), "{url}");
        assert!(
            locked(&server.session).is_none(),
            "no ffmpeg for a direct file"
        );
    }

    #[test]
    fn a_path_outside_the_media_root_starts_no_session() {
        let (server, _dir) = direct_server();
        assert!(server
            .stream_url("../../etc/passwd", 0.0, Delivery::Remux, None, 0)
            .is_err());
        assert!(locked(&server.session).is_none());
    }

    #[test]
    fn a_token_is_long_and_not_constant() {
        let first = mint_token();
        assert_eq!(first.len(), 32);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, mint_token(), "a fixed token would guard nothing");
    }
}
