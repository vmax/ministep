//! Round trip through a real Unix socket.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ministep_core::engine::{Engine, EngineConfig};
use ministep_core::patterns::PatternStore;
use ministep_core::state::AppState;
use ministep_core::transport::Outputs;
use ministep_remote::{MAX_LINE, RemoteServer};
use serde_json::Value;

fn make_engine(dir: &Path) -> Arc<Engine> {
    Engine::new(
        AppState::default(),
        Outputs::default(),
        EngineConfig {
            patterns: PatternStore::new(dir),
            sequence_path: dir.join("sequence.json"),
            realtime: false,
            ..Default::default()
        },
    )
}

fn read_json(reader: &mut impl BufRead) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).expect("read line");
    assert!(line.ends_with('\n'), "unterminated line {line:?}");
    serde_json::from_str(&line).expect("valid json")
}

fn wait_until(mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn server_round_trip() {
    // macOS caps AF_UNIX paths at 104 bytes; the default temp dir is longer.
    let dir = tempfile::Builder::new().prefix("ms").tempdir_in("/tmp").unwrap();
    let sock = dir.path().join("control.sock");
    std::fs::write(&sock, b"stale").unwrap();
    let engine = make_engine(dir.path());
    let mut server = RemoteServer::start(Arc::clone(&engine), sock.clone()).unwrap();
    assert_eq!(std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777, 0o600);

    let stream = UnixStream::connect(&sock).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    let mut raw = String::new();
    reader.read_line(&mut raw).unwrap();
    assert!(raw.starts_with("{\"type\":\"state\",\"bpm\":128.0,"), "{raw}");
    let first: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(first["length"], 0);

    writer.write_all(b"{\"cmd\":\"REST\"}\n").unwrap();
    let state = read_json(&mut reader);
    assert_eq!(state["length"], 1);

    writer.write_all(b"not json\n").unwrap();
    let err = read_json(&mut reader);
    assert_eq!(err["type"], "error");
    assert_eq!(err["msg"], "bad json");

    writer.write_all(b"[1]\n").unwrap();
    assert_eq!(read_json(&mut reader)["msg"], "expected object");

    let mut long = vec![b' '; MAX_LINE];
    long.push(b'\n');
    writer.write_all(&long).unwrap();
    assert_eq!(read_json(&mut reader)["msg"], "line too long");

    // An apply error is reported, then the snapshot is broadcast anyway.
    writer.write_all(b"{\"cmd\":\"DANCE\"}\n").unwrap();
    assert_eq!(read_json(&mut reader)["msg"], "unknown cmd 'DANCE'");
    assert_eq!(read_json(&mut reader)["type"], "state");

    // A change made elsewhere (GUI, MIDI) is pushed by the poll loop.
    engine.edit(|s| s.set_bpm(90.0));
    let mut pushed = read_json(&mut reader);
    while pushed["bpm"] != 90.0 {
        pushed = read_json(&mut reader);
    }
    assert_eq!(pushed["bpm"], 90.0);

    // A second client gets broadcasts triggered by the first.
    let other = UnixStream::connect(&sock).unwrap();
    other.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut other_reader = BufReader::new(other);
    assert_eq!(read_json(&mut other_reader)["length"], 1);
    assert!(wait_until(|| server.client_count() == 2));
    writer.write_all(b"{\"cmd\":\"REST\"}\n").unwrap();
    assert_eq!(read_json(&mut reader)["length"], 2);
    assert_eq!(read_json(&mut other_reader)["length"], 2);

    drop(writer);
    drop(reader);
    assert!(wait_until(|| server.client_count() == 1));

    server.close();
    assert!(!sock.exists());
    // The remaining client sees EOF once the server closes.
    let mut rest = String::new();
    assert_eq!(other_reader.read_line(&mut rest).unwrap(), 0);
}

#[test]
fn drop_removes_socket() {
    let dir = tempfile::Builder::new().prefix("ms").tempdir_in("/tmp").unwrap();
    let sock = dir.path().join("nested").join("control.sock");
    let engine = make_engine(dir.path());
    let server = RemoteServer::start(engine, sock.clone()).unwrap();
    assert!(sock.exists());
    drop(server);
    assert!(!sock.exists());
}
