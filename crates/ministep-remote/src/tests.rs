use std::path::Path;
use std::sync::Arc;

use ministep_core::engine::{Engine, EngineConfig};
use ministep_core::message::MidiMessage;
use ministep_core::patterns::PatternStore;
use ministep_core::state::{AppState, Track};
use ministep_core::transport::Outputs;
use serde_json::{Value, json};

use super::*;

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

fn run(engine: &Engine, message: Value) -> Result<(), String> {
    apply(engine, &message)
}

#[test]
fn snapshot_shape() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = make_engine(tmp.path());
    engine.handle_midi(MidiMessage::NoteOn { channel: 0, note: 63, velocity: 100 });
    engine.edit(|s| s.commit_last_note());
    let snap = snapshot(&engine);
    assert_eq!(snap["type"], "state");
    assert_eq!(snap["bpm"], json!(128.0));
    assert!(snap["bpm"].is_f64());
    assert_eq!(snap["division"], 16);
    assert!(snap["loop"].is_null());
    assert_eq!(snap["length"], 1);
    assert_eq!(snap["cursor_note"], "D#3");
    assert_eq!(snap["last"], "D#3");
    assert_eq!(snap["playing"], false);
    assert_eq!(snap["recording"], false);
    assert_eq!(snap["track"], "melodic");
    let keys: Vec<&str> = snap.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "type", "bpm", "division", "loop", "playing", "recording", "playhead", "length", "cursor",
            "cursor_note", "last", "transpose", "status", "input", "output", "drum_output", "track"
        ]
    );
}

#[test]
fn snapshot_empty_rest_and_drums() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = make_engine(tmp.path());
    let snap = snapshot(&engine);
    assert!(snap["cursor_note"].is_null());
    assert!(snap["last"].is_null());
    assert!(snap["input"].is_null() && snap["output"].is_null() && snap["drum_output"].is_null());
    engine.edit(|s| {
        s.append_rest();
        s.loop_length = Some(8);
        s.active_track = Track::Drums;
    });
    let snap = snapshot(&engine);
    assert_eq!(snap["cursor_note"], "--");
    assert_eq!(snap["loop"], 8);
    assert_eq!(snap["track"], "drums");
}

#[test]
fn encode_is_compact_and_ascii() {
    let encoded = encode(&json!({"type": "state", "bpm": 128.0, "loop": null, "status": "é ♪ 🎹\n"}));
    assert_eq!(
        String::from_utf8(encoded).unwrap(),
        "{\"type\":\"state\",\"bpm\":128.0,\"loop\":null,\"status\":\"\\u00e9 \\u266a \\ud83c\\udfb9\\n\"}\n"
    );
}

#[test]
fn apply_controller_commands_and_extras() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = make_engine(tmp.path());
    assert_eq!(run(&engine, json!({"cmd": "BPM_UP", "value": 5})), Ok(()));
    assert_eq!(engine.state().bpm, 133.0);
    assert_eq!(run(&engine, json!({"cmd": "RECORD_TOGGLE"})), Ok(()));
    assert!(engine.state().recording);
    assert_eq!(run(&engine, json!({"cmd": "REST"})), Ok(()));
    assert_eq!(run(&engine, json!({"cmd": "REST"})), Ok(()));
    assert_eq!(run(&engine, json!({"cmd": "CURSOR", "value": -1})), Ok(()));
    assert_eq!(engine.state().cursor, 0);
    assert_eq!(run(&engine, json!({"cmd": "DELETE"})), Ok(()));
    assert_eq!(engine.state().sequence.len(), 1);
    assert_eq!(run(&engine, json!({"cmd": "LOOP_CYCLE", "value": 1})), Ok(()));
    assert_eq!(engine.state().loop_length, Some(8));
    assert_eq!(engine.state().status_message, "Loop length: 8");
    assert_eq!(run(&engine, json!({"cmd": "SAVE"})), Ok(()));
    assert!(tmp.path().join("sequence.json").exists());
    assert_eq!(run(&engine, json!({"cmd": "CLEAR"})), Ok(()));
    assert_eq!(engine.state().sequence.len(), 0);
    assert_eq!(run(&engine, json!({"cmd": "LOAD"})), Ok(()));
    assert_eq!(engine.state().sequence.len(), 1);
}

#[test]
fn apply_rejects_bad_messages() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = make_engine(tmp.path());
    assert_eq!(run(&engine, json!({})), Err("missing cmd".into()));
    assert_eq!(run(&engine, json!({"cmd": 3})), Err("missing cmd".into()));
    assert_eq!(run(&engine, json!({"cmd": "DANCE"})), Err("unknown cmd 'DANCE'".into()));
    assert_eq!(run(&engine, json!({"cmd": "it's"})), Err("unknown cmd \"it's\"".into()));
    let err = run(&engine, json!({"cmd": "LOAD"})).unwrap_err();
    assert!(err.starts_with("load failed"), "{err}");
    assert_eq!(
        run(&engine, json!({"cmd": "SAVE_PATTERN", "value": "  "})),
        Err("SAVE_PATTERN needs a pattern name in value".into())
    );
    assert_eq!(
        run(&engine, json!({"cmd": "LOAD_PATTERN", "value": 7})),
        Err("LOAD_PATTERN needs a pattern name in value".into())
    );
    let err = run(&engine, json!({"cmd": "LOAD_PATTERN", "value": "nope"})).unwrap_err();
    assert_eq!(err, engine.state().status_message);
    assert!(err.starts_with("Failed to load"), "{err}");
}

#[test]
fn apply_patterns_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = make_engine(tmp.path());
    engine.edit(|s| {
        s.append_note(60, None);
        s.append_note(64, None);
    });
    assert_eq!(run(&engine, json!({"cmd": "SAVE_PATTERN", "value": "bite_test"})), Ok(()));
    assert_eq!(run(&engine, json!({"cmd": "CLEAR"})), Ok(()));
    assert_eq!(run(&engine, json!({"cmd": "LOAD_PATTERN", "value": "bite_test"})), Ok(()));
    assert_eq!(engine.state().sequence.len(), 2);
}

#[test]
fn apply_editor_extras() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = make_engine(tmp.path());
    engine.edit(|s| {
        s.append_note(60, None);
        s.append_rest();
    });
    // String values coerce like Python int(); junk falls back to the default.
    assert_eq!(run(&engine, json!({"cmd": "CURSOR", "value": " -1 "})), Ok(()));
    assert_eq!(engine.state().cursor, 0);
    assert_eq!(run(&engine, json!({"cmd": "CURSOR_NOTE", "value": 2.9})), Ok(()));
    assert_eq!(engine.state().sequence[0].note, Some(62));
    assert_eq!(run(&engine, json!({"cmd": "CURSOR_NOTE", "value": "x"})), Ok(()));
    assert_eq!(engine.state().sequence[0].note, Some(63));
    assert_eq!(run(&engine, json!({"cmd": "CURSOR", "value": i64::MAX})), Ok(()));
    assert_eq!(run(&engine, json!({"cmd": "REPLACE"})), Ok(()));
    assert_eq!(engine.state().status_message, "Audition a MIDI note before replacing a step.");
    assert_eq!(run(&engine, json!({"cmd": "LOOP_CYCLE", "value": -1})), Ok(()));
    assert_eq!(engine.state().loop_length, Some(64));
    assert_eq!(run(&engine, json!({"cmd": "LOOP_CYCLE"})), Ok(()));
    assert_eq!(engine.state().loop_length, None);
    assert_eq!(engine.state().status_message, "Loop length: FULL");
    assert_eq!(run(&engine, json!({"cmd": "SET_BPM", "value": 90})), Ok(()));
    assert_eq!(engine.state().bpm, 90.0);
    assert_eq!(run(&engine, json!({"cmd": "RECORD", "value": false})), Ok(()));
    assert!(!engine.state().recording);
}

#[test]
fn python_int_coercion() {
    assert_eq!(int_value(Some(&json!("1_000")), 0), 1000);
    assert_eq!(int_value(Some(&json!("+7")), 0), 7);
    assert_eq!(int_value(Some(&json!("1.5")), 9), 9);
    assert_eq!(int_value(Some(&json!("_1")), 9), 9);
    assert_eq!(int_value(Some(&json!(-2.7)), 0), -2);
    assert_eq!(int_value(Some(&json!(true)), 0), 1);
    assert_eq!(int_value(Some(&json!(null)), 4), 4);
    assert_eq!(int_value(Some(&json!([1])), 4), 4);
    assert_eq!(int_value(None, 1), 1);
}
