//! Remote control over a Unix domain socket, newline-delimited JSON.
//!
//! Any local client (the MacroPad daemon, a script, `nc -U`) connects to
//! `~/.config/ministep/control.sock` and receives a full state snapshot on
//! connect, then a fresh snapshot whenever something changes (polled at
//! 20 Hz). Commands are one object per line:
//!
//! ```text
//! {"cmd": "PLAY_STOP"}
//! {"cmd": "BPM_UP", "value": 5}
//! {"cmd": "CURSOR", "value": -1}
//! {"cmd": "SAVE"}
//! {"cmd": "SAVE_PATTERN", "value": "bite_test"}
//! ```
//!
//! `cmd` is any [`CommandName`] name, or one of [`EXTRA_COMMANDS`]. Unknown or
//! malformed lines get an `{"type":"error"}` reply and are otherwise ignored.
//! All state mutations go through the thread-safe [`Engine`], so they are
//! ordered with MIDI and GUI input.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ministep_core::controller::{Command, CommandName};
use ministep_core::engine::Engine;
use ministep_core::notes::note_name;
use ministep_core::patterns::default_config_dir;
use ministep_core::state::LOOP_LENGTHS;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Map, Value, json};

pub const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Longest accepted command line in bytes, counting the trailing newline.
pub const MAX_LINE: usize = 4096;
/// How often the accept loop checks for shutdown.
const ACCEPT_INTERVAL: Duration = Duration::from_millis(20);
/// A client that cannot take a snapshot within this long is dropped.
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
/// Polls skipped via `Engine::version()` before a full comparison is forced anyway.
const FULL_CHECK_EVERY: u32 = 20;

/// Editor operations that are not controller commands.
pub const EXTRA_COMMANDS: [&str; 9] =
    ["SAVE", "LOAD", "SAVE_PATTERN", "LOAD_PATTERN", "CURSOR", "CURSOR_NOTE", "REPLACE", "DELETE", "LOOP_CYCLE"];

pub fn default_socket_path() -> PathBuf {
    default_config_dir().join("control.sock")
}

/// Serialisable view of everything a remote display might show.
pub fn snapshot(engine: &Engine) -> Value {
    let state = engine.state();
    let cursor_note = state.sequence.get(state.cursor.min(state.sequence.len().saturating_sub(1))).map(|step| {
        match step.note {
            None => "--".to_string(),
            Some(note) => note_name(note),
        }
    });
    let last = state.last_note_played.map(|(note, _)| note_name(note));
    let mut m = Map::new();
    m.insert("type".into(), json!("state"));
    m.insert("bpm".into(), json!(state.bpm));
    m.insert("division".into(), json!(state.step_division));
    m.insert("loop".into(), json!(state.loop_length));
    m.insert("playing".into(), json!(state.playing));
    m.insert("recording".into(), json!(state.recording));
    m.insert("playhead".into(), json!(state.playhead));
    m.insert("length".into(), json!(state.sequence.len()));
    m.insert("cursor".into(), json!(state.cursor));
    m.insert("cursor_note".into(), json!(cursor_note));
    m.insert("last".into(), json!(last));
    m.insert("transpose".into(), json!(state.transpose));
    m.insert("status".into(), json!(state.status_message));
    m.insert("input".into(), json!(state.selected_input));
    m.insert("output".into(), json!(state.selected_output));
    m.insert("drum_output".into(), json!(state.selected_drum_output));
    m.insert("track".into(), json!(state.active_track.as_str()));
    Value::Object(m)
}

/// Python's `int(value)` with a fallback: numbers truncate toward zero, bools
/// are 0/1, strings parse as base-10 integers (whitespace, sign and `_`
/// digit separators allowed); anything else yields `default`.
fn int_value(value: Option<&Value>, default: i64) -> i64 {
    match value {
        Some(Value::Bool(b)) => i64::from(*b),
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_u64().map(|u| i64::try_from(u).unwrap_or(i64::MAX)))
            .or_else(|| n.as_f64().map(|f| f.trunc() as i64))
            .unwrap_or(default),
        Some(Value::String(s)) => parse_python_int(s).unwrap_or(default),
        _ => default,
    }
}

fn parse_python_int(s: &str) -> Option<i64> {
    let s = s.trim();
    let (negative, digits) = match s.as_bytes().first()? {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return None;
    }
    let mut magnitude: i64 = 0;
    for c in digits.chars().filter(|&c| c != '_') {
        let d = c.to_digit(10)?;
        magnitude = magnitude.saturating_mul(10).saturating_add(i64::from(d));
    }
    Some(if negative { -magnitude } else { magnitude })
}

/// Numeric payload for a controller command (`bool` counts as 0/1, as in Python).
fn command_value(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::Bool(b)) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

/// Python `repr()` of a string, for error messages.
fn py_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || (0x7f..0xa0).contains(&(c as u32)) => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Apply one command message.
pub fn apply(engine: &Engine, message: &Value) -> Result<(), String> {
    let Some(cmd) = message.get("cmd").and_then(Value::as_str) else {
        return Err("missing cmd".into());
    };
    let value = message.get("value");
    if let Some(name) = CommandName::parse(cmd) {
        engine.execute(Command { name, value: command_value(value) });
        return Ok(());
    }
    match cmd {
        "SAVE" => engine.save().map_err(|e| format!("save failed: {e}")),
        "LOAD" => engine.load().map_err(|e| format!("load failed: {e}")),
        "SAVE_PATTERN" | "LOAD_PATTERN" => {
            let Some(name) = value.and_then(Value::as_str).filter(|n| !n.trim().is_empty()) else {
                return Err(format!("{cmd} needs a pattern name in value"));
            };
            let ok = if cmd == "SAVE_PATTERN" {
                engine.save_pattern(name, None)
            } else {
                engine.load_pattern(name, None)
            };
            if ok { Ok(()) } else { Err(engine.state().status_message.clone()) }
        }
        "CURSOR" => {
            let delta = int_value(value, 1);
            engine.edit(|s| {
                // Reduce first so a huge delta cannot overflow the cursor arithmetic.
                if !s.sequence.is_empty() {
                    s.move_cursor(delta.rem_euclid(s.sequence.len() as i64));
                }
            });
            Ok(())
        }
        "CURSOR_NOTE" => {
            let semitones = int_value(value, 1).clamp(-255, 255) as i32;
            engine.edit(|s| s.adjust_cursor_note(semitones));
            Ok(())
        }
        "REPLACE" => {
            engine.edit(|s| s.replace_cursor_with_last_note());
            Ok(())
        }
        "DELETE" => {
            engine.edit(|s| s.delete_cursor());
            Ok(())
        }
        "LOOP_CYCLE" => {
            let delta = int_value(value, 1).rem_euclid(LOOP_LENGTHS.len() as i64);
            engine.edit(|s| {
                s.cycle_loop_length(delta);
                s.status_message = match s.loop_length {
                    None => "Loop length: FULL".into(),
                    Some(length) => format!("Loop length: {length}"),
                };
            });
            Ok(())
        }
        _ => Err(format!("unknown cmd {}", py_repr(cmd))),
    }
}

/// JSON formatter matching Python's default `ensure_ascii=True`.
struct AsciiFormatter;

impl serde_json::ser::Formatter for AsciiFormatter {
    fn write_string_fragment<W: ?Sized + Write>(&mut self, writer: &mut W, fragment: &str) -> io::Result<()> {
        let mut start = 0;
        for (i, c) in fragment.char_indices() {
            if c.is_ascii() {
                continue;
            }
            writer.write_all(&fragment.as_bytes()[start..i])?;
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                write!(writer, "\\u{unit:04x}")?;
            }
            start = i + c.len_utf8();
        }
        writer.write_all(&fragment.as_bytes()[start..])
    }
}

/// Compact one-line encoding, like Python's `json.dumps(m, separators=(",", ":"))`.
pub fn encode(message: &Value) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    let mut ser = serde_json::Serializer::with_formatter(&mut out, AsciiFormatter);
    message.serialize(&mut ser).expect("serialising a Value cannot fail");
    out.push(b'\n');
    out
}

fn error_message(msg: &str) -> Vec<u8> {
    encode(&json!({"type": "error", "msg": msg}))
}

// --- server ----------------------------------------------------------------------------

#[derive(Default)]
struct Hub {
    clients: HashMap<u64, UnixStream>,
    last: Option<Value>,
}

impl Hub {
    fn send(&mut self, id: u64, data: &[u8]) {
        if let Some(mut stream) = self.clients.get(&id)
            && stream.write_all(data).is_err()
        {
            self.drop_client(id);
        }
    }

    fn drop_client(&mut self, id: u64) {
        if let Some(stream) = self.clients.remove(&id) {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

struct Inner {
    engine: Arc<Engine>,
    stop: AtomicBool,
    hub: Mutex<Hub>,
    readers: Mutex<Vec<JoinHandle<()>>>,
    next_id: AtomicU64,
}

impl Inner {
    /// Send the current snapshot to every client, unless it equals the last one sent.
    fn broadcast(&self, force: bool) {
        let mut hub = self.hub.lock();
        let current = snapshot(&self.engine);
        if !force && hub.last.as_ref() == Some(&current) {
            return;
        }
        let data = encode(&current);
        hub.last = Some(current);
        let ids: Vec<u64> = hub.clients.keys().copied().collect();
        for id in ids {
            hub.send(id, &data);
        }
    }

    fn poll_loop(&self) {
        let mut seen = None;
        let mut skipped = 0;
        while !self.stop.load(Ordering::Acquire) {
            thread::park_timeout(POLL_INTERVAL);
            if self.stop.load(Ordering::Acquire) {
                break;
            }
            // The version counter tells us cheaply whether anything changed; still do a
            // full comparison now and then in case state was touched without a bump.
            let version = self.engine.version();
            if seen == Some(version) && skipped < FULL_CHECK_EVERY {
                skipped += 1;
                continue;
            }
            seen = Some(version);
            skipped = 0;
            self.broadcast(false);
        }
    }

    fn accept_loop(self: &Arc<Self>, listener: &UnixListener) {
        while !self.stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((stream, _)) => self.add_client(stream),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(ACCEPT_INTERVAL),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => thread::sleep(ACCEPT_INTERVAL),
            }
        }
    }

    fn add_client(self: &Arc<Self>, stream: UnixStream) {
        // Accepted sockets inherit O_NONBLOCK from the listener on BSD/macOS.
        if stream.set_nonblocking(false).is_err() || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err() {
            return;
        }
        let Ok(reader) = stream.try_clone() else { return };
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut hub = self.hub.lock();
            hub.clients.insert(id, stream);
            hub.send(id, &encode(&snapshot(&self.engine)));
        }
        let inner = Arc::clone(self);
        let spawned = thread::Builder::new().name(format!("ministep-remote-{id}")).spawn(move || inner.serve(id, reader));
        let mut readers = self.readers.lock();
        readers.retain(|h| !h.is_finished());
        match spawned {
            Ok(handle) => readers.push(handle),
            Err(_) => self.hub.lock().drop_client(id),
        }
    }

    fn serve(&self, id: u64, stream: UnixStream) {
        let mut reader = BufReader::new(stream);
        loop {
            let line = match read_line(&mut reader) {
                Ok(Line::Eof) | Err(_) => break,
                Ok(Line::TooLong) => {
                    self.hub.lock().send(id, &error_message("line too long"));
                    continue;
                }
                Ok(Line::Data(line)) => line,
            };
            if self.stop.load(Ordering::Acquire) {
                break;
            }
            let message = match serde_json::from_slice::<Value>(&line) {
                Ok(Value::Object(m)) => Value::Object(m),
                Ok(_) => {
                    self.hub.lock().send(id, &error_message("expected object"));
                    continue;
                }
                Err(_) => {
                    self.hub.lock().send(id, &error_message("bad json"));
                    continue;
                }
            };
            if let Err(error) = apply(&self.engine, &message) {
                self.hub.lock().send(id, &error_message(&error));
            }
            self.broadcast(true);
        }
        self.hub.lock().drop_client(id);
    }
}

enum Line {
    Data(Vec<u8>),
    TooLong,
    Eof,
}

/// Read one `\n`-terminated line (a final unterminated line counts too). Lines longer
/// than [`MAX_LINE`] bytes including the newline are consumed and reported as too long.
fn read_line(reader: &mut impl BufRead) -> io::Result<Line> {
    let mut line = Vec::new();
    let mut total = 0usize;
    loop {
        let available = match reader.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            return Ok(match total {
                0 => Line::Eof,
                n if n > MAX_LINE => Line::TooLong,
                _ => Line::Data(line),
            });
        }
        let (chunk, done) = match available.iter().position(|&b| b == b'\n') {
            Some(i) => (&available[..=i], true),
            None => (available, false),
        };
        total += chunk.len();
        if total <= MAX_LINE {
            line.extend_from_slice(chunk);
        }
        let used = chunk.len();
        reader.consume(used);
        if done {
            return Ok(if total > MAX_LINE { Line::TooLong } else { Line::Data(line) });
        }
    }
}

/// Unix-socket remote control server. Dropping it (or [`RemoteServer::close`]) stops
/// all threads, disconnects clients and removes the socket file.
pub struct RemoteServer {
    inner: Arc<Inner>,
    path: PathBuf,
    accept: Option<JoinHandle<()>>,
    poll: Option<JoinHandle<()>>,
}

impl RemoteServer {
    pub fn start(engine: Arc<Engine>, path: PathBuf) -> io::Result<RemoteServer> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        match fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let inner = Arc::new(Inner {
            engine,
            stop: AtomicBool::new(false),
            hub: Mutex::new(Hub::default()),
            readers: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(0),
        });
        let accept_inner = Arc::clone(&inner);
        let accept = thread::Builder::new()
            .name("ministep-remote-accept".into())
            .spawn(move || accept_inner.accept_loop(&listener))?;
        let poll_inner = Arc::clone(&inner);
        let poll = thread::Builder::new().name("ministep-remote-poll".into()).spawn(move || poll_inner.poll_loop());
        let mut server = RemoteServer { inner, path, accept: Some(accept), poll: None };
        server.poll = Some(poll?);
        Ok(server)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn client_count(&self) -> usize {
        self.inner.hub.lock().clients.len()
    }

    /// Stop serving. Idempotent.
    pub fn close(&mut self) {
        self.inner.stop.store(true, Ordering::Release);
        if let Some(poll) = self.poll.take() {
            poll.thread().unpark();
            let _ = poll.join();
        }
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
            let _ = fs::remove_file(&self.path);
        }
        let clients: Vec<UnixStream> = self.inner.hub.lock().clients.drain().map(|(_, s)| s).collect();
        for stream in clients {
            let _ = stream.shutdown(Shutdown::Both);
        }
        let readers: Vec<JoinHandle<()>> = self.inner.readers.lock().drain(..).collect();
        for reader in readers {
            let _ = reader.join();
        }
    }
}

impl Drop for RemoteServer {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests;
