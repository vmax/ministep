//! Parity with the Python build: `AppState`, `DrumTrack`, and snapshot (de)serialisation.
//! Ported from the Python build's tests/test_state.py, test_persistence.py, test_patterns.py
//! (serialisation section) and test_drums.py (model and persistence sections).

use ministep_core::drums::{DRUM_STEPS, DrumStep, DrumTrack};
use ministep_core::notes::{note_name, parse_note_name};
use ministep_core::state::{AppState, Step};
use serde_json::{Value, json};

fn rich_state() -> AppState {
    let mut s = AppState {
        bpm: 133.5,
        step_division: 8,
        loop_length: Some(16),
        default_gate: 0.35,
        default_velocity: 90,
        transpose: -3,
        octave: 1,
        output_channel: 4,
        ..AppState::default()
    };
    s.append_note(60, Some(111));
    s.append_hold();
    s.append_rest();
    s.append_note(67, Some(42));
    let last = s.sequence.last_mut().unwrap();
    last.accent = true;
    last.enabled = false;
    s
}

fn musical_view(s: &AppState) -> Value {
    let mut data = s.to_json(None);
    data.as_object_mut().unwrap().remove("version");
    data
}

fn round_trip(s: &AppState) -> AppState {
    let text = serde_json::to_string(&s.to_json(None)).unwrap();
    AppState::from_json(&serde_json::from_str(&text).unwrap()).unwrap()
}

fn hit(s: &mut AppState, lane: usize, step: usize, velocity: Option<u8>) {
    let d = &mut s.drums.lanes[lane].steps[step];
    d.active = true;
    if let Some(v) = velocity {
        d.velocity = v;
    }
}

// --- test_state.py -------------------------------------------------------------

#[test]
fn note_name_uses_ableton_live_octave_numbering() {
    assert_eq!(note_name(60), "C3");
    assert_eq!(note_name(61), "C#3");
    assert_eq!(note_name(0), "C-2");
}

#[test]
fn sequence_append_rest_and_undo() {
    let mut s = AppState { last_note_played: Some((60, 111)), ..AppState::default() };
    assert!(s.commit_last_note());
    s.append_rest();
    let pairs: Vec<_> = s.sequence.iter().map(|x| (x.note, x.velocity)).collect();
    assert_eq!(pairs, vec![(Some(60), 111), (None, 100)]);
    assert_eq!(s.undo(), Some(Step::new(None, 100, 0.5)));
    assert_eq!(s.sequence.iter().map(|x| x.note).collect::<Vec<_>>(), vec![Some(60)]);
}

#[test]
fn hold_extends_last_note_and_undo_restores_its_release() {
    let mut s = AppState::default();
    s.append_note(60, Some(111));
    assert!(s.append_hold().is_some());
    let v: Vec<_> = s.sequence.iter().map(|x| (x.note, x.gate, x.tie)).collect();
    assert_eq!(v, vec![(Some(60), 0.5, true), (Some(60), 1.0, false)]);
    s.undo();
    let v: Vec<_> = s.sequence.iter().map(|x| (x.note, x.tie)).collect();
    assert_eq!(v, vec![(Some(60), false)]);
}

#[test]
fn commit_requires_an_auditioned_note() {
    let mut s = AppState::default();
    assert!(!s.commit_last_note());
    assert!(s.sequence.is_empty());
    assert!(s.status_message.contains("Nothing to commit"));
}

#[test]
fn auditioned_note_is_exposed_before_it_is_committed() {
    let mut s = AppState::default();
    s.note_held(61, 107);
    assert_eq!(s.last_note_played, Some((61, 107)));
    assert!(s.status_message.contains("Auditioning C#3"), "{}", s.status_message);
    assert!(s.sequence.is_empty());
}

#[test]
fn arrow_editing_changes_selected_pitched_step() {
    let mut s = AppState { sequence: vec![Step::new(Some(60), 100, 0.5), Step::new(None, 100, 0.5)], ..AppState::default() };
    assert!(s.adjust_cursor_note(1));
    assert_eq!(s.sequence[0].note, Some(61));
    s.move_cursor(1);
    assert!(!s.adjust_cursor_note(1));
    assert_eq!(s.sequence[1].note, None);
}

#[test]
fn set_root_edits_notes_using_nearest_pitch_class_transposition() {
    let seq = [60, 63, 67].map(|n| Step::new(Some(n), 100, 0.5)).to_vec();
    let mut s = AppState { sequence: seq, ..AppState::default() };
    assert!(s.set_root_from_note(70)); // Bb
    assert_eq!(s.transpose, 0);
    assert_eq!(s.sequence.iter().map(|x| x.note).collect::<Vec<_>>(), vec![Some(58), Some(61), Some(65)]);
}

#[test]
fn loop_length_limits_playback_without_truncating_the_sequence() {
    let s = AppState { sequence: vec![Step::new(Some(60), 100, 0.5); 12], loop_length: Some(8), ..AppState::default() };
    assert_eq!(s.playback_length(), 8);
    assert_eq!(s.sequence.len(), 12);
}

// --- test_persistence.py ---------------------------------------------------------

#[test]
fn json_save_and_load_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("melody.json");
    let mut s =
        AppState { bpm: 99.0, step_division: 8, loop_length: Some(16), default_gate: 0.7, transpose: -2, ..AppState::default() };
    s.append_note(60, Some(111));
    s.append_rest();
    s.save(&path).unwrap();

    let mut loaded = AppState::default();
    loaded.load(&path).unwrap();
    assert_eq!(loaded.bpm, 99.0);
    assert_eq!(loaded.step_division, 8);
    assert_eq!(loaded.loop_length, Some(16));
    assert_eq!(loaded.default_gate, 0.7);
    assert_eq!(loaded.transpose, -2);
    let pairs: Vec<_> = loaded.sequence.iter().map(|x| (x.note, x.velocity)).collect();
    assert_eq!(pairs, vec![(Some(60), 111), (None, 100)]);
}

// --- test_patterns.py: serialisation -----------------------------------------------

#[test]
fn serialize_deserialize_roundtrip() {
    let s = rich_state();
    let restored = round_trip(&s);
    assert_eq!(musical_view(&restored), musical_view(&s));
    assert_eq!(restored.sequence, s.sequence);
}

#[test]
fn notes_and_rests_survive() {
    let r = round_trip(&rich_state());
    assert_eq!(r.sequence.iter().map(|x| x.note).collect::<Vec<_>>(), vec![Some(60), Some(60), None, Some(67)]);
}

#[test]
fn hold_state_survives() {
    let r = round_trip(&rich_state());
    let v: Vec<_> = r.sequence[..2].iter().map(|x| (x.tie, x.gate)).collect();
    assert_eq!(v, vec![(true, 0.35), (false, 1.0)]);
}

#[test]
fn velocity_survives() {
    let r = round_trip(&rich_state());
    assert_eq!(r.sequence.iter().map(|x| x.velocity).collect::<Vec<_>>(), vec![111, 111, 90, 42]);
    assert_eq!(r.default_velocity, 90);
}

#[test]
fn bpm_division_gate_survive() {
    let r = round_trip(&rich_state());
    assert_eq!((r.bpm, r.step_division, r.default_gate), (133.5, 8, 0.35));
}

#[test]
fn transpose_octave_root_survive() {
    let r = round_trip(&rich_state());
    assert_eq!((r.transpose, r.octave), (-3, 1));
    assert_eq!(r.sequence_root(), Some(60));
}

#[test]
fn loop_length_and_flags_survive() {
    let r = round_trip(&rich_state());
    assert_eq!(r.loop_length, Some(16));
    assert_eq!((r.sequence[3].accent, r.sequence[3].enabled), (true, false));
}

#[test]
fn transient_state_is_not_serialized() {
    let mut s = rich_state();
    s.playing = true;
    s.playhead = 2;
    s.cursor = 1;
    s.recording = true;
    s.selected_input = Some("MiniLab3".into());
    s.note_held(70, 99);
    let data = s.to_json(Some("x"));
    for key in ["playing", "playhead", "cursor", "recording", "selected_input", "last_note_played"] {
        assert!(data.get(key).is_none(), "{key}");
    }
}

#[test]
fn missing_optional_fields_get_defaults() {
    let r = AppState::from_json(&json!({"version": 1, "steps": [{"note": 62}]})).unwrap();
    assert_eq!(r.bpm, 128.0);
    assert_eq!(r.step_division, 16);
    assert_eq!(r.loop_length, None);
    assert_eq!(r.octave, 0);
    assert_eq!(r.sequence, vec![Step::new(Some(62), 100, 0.5)]);
}

#[test]
fn unknown_fields_are_ignored() {
    let mut data = rich_state().to_json(Some("future"));
    data["swing"] = json!(0.6);
    data["melodic"]["steps"][0]["probability"] = json!(0.5);
    let r = AppState::from_json(&data).unwrap();
    assert_eq!(r.sequence[0].note, Some(60));
}

#[test]
fn unsupported_version_is_a_readable_error() {
    let e = AppState::from_json(&json!({"version": 99, "steps": []})).unwrap_err();
    assert!(e.contains("Unsupported pattern version 99"), "{e}");
}

#[test]
fn structurally_broken_steps_are_readable_errors() {
    let e = AppState::from_json(&json!({"version": 1, "steps": [{"velocity": 3}]})).unwrap_err();
    assert!(e.contains("Invalid pattern data"), "{e}");
    let e = AppState::from_json(&json!({"version": 1, "steps": [{"note": 400}]})).unwrap_err();
    assert!(e.contains("Invalid pattern data"), "{e}");
    let e = AppState::from_json(&json!([1, 2])).unwrap_err();
    assert!(e.contains("JSON object"), "{e}");
}

// --- test_drums.py: model -------------------------------------------------------------

#[test]
fn default_drum_track_has_four_empty_lanes() {
    let d = DrumTrack::default();
    assert_eq!(d.lanes.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), vec!["Kick", "Snare", "Closed Hat", "Open Hat"]);
    assert!(d.lanes.iter().all(|l| l.steps.len() == DRUM_STEPS));
    assert!(!d.has_hits());
    assert_eq!(d.channel, 10);
    assert!(!AppState::default().drums.has_hits());
}

#[test]
fn default_lane_notes_follow_ableton_naming_and_gm_drum_map() {
    let d = DrumTrack::default();
    assert_eq!(d.lanes.iter().map(|l| note_name(l.note)).collect::<Vec<_>>(), vec!["C1", "D1", "F#1", "A#1"]);
    assert_eq!(d.lanes.iter().map(|l| l.note).collect::<Vec<_>>(), vec![36, 38, 42, 46]);
    assert_eq!(parse_note_name("C1"), Ok(36));
    assert_eq!(parse_note_name("Bb1"), Ok(46));
    assert!(parse_note_name("H2").is_err());
}

#[test]
fn toggle_hit_and_cursor_movement() {
    let mut d = DrumTrack::default();
    d.move_lane(1);
    d.move_step(3);
    assert_eq!((d.cursor_lane, d.cursor_step), (1, 3));
    assert!(d.toggle_hit());
    assert!(d.lanes[1].steps[3].active);
    let hits: Vec<_> = d.hits_at(3).map(|(l, s)| (l.clone(), *s)).collect();
    assert_eq!(hits, vec![(d.lanes[1].clone(), d.lanes[1].steps[3])]);
    assert!(!d.toggle_hit());
    assert!(!d.has_hits());
    d.move_step(-4);
    d.move_lane(-2);
    assert_eq!((d.cursor_lane, d.cursor_step), (3, 15));
}

/// Python also asserted that `DrumStep(velocity=0)` and a short `DrumLane` raise in
/// their constructors. In Rust a lane is a fixed `[DrumStep; 16]` (length enforced by
/// the type) and range checks live in `DrumTrack::from_json` (covered below).
#[test]
fn velocity_is_stored_per_hit_and_clamped() {
    let mut d = DrumTrack::default();
    assert_eq!(d.cursor().unwrap().velocity, 100);
    assert_eq!(d.adjust_velocity(-10), 90);
    assert_eq!(d.adjust_velocity(-200), 1);
    assert_eq!(d.adjust_velocity(500), 127);
    d.move_step(1);
    assert_eq!(d.cursor().unwrap().velocity, 100);
}

// --- test_drums.py: persistence ----------------------------------------------------------

fn beat_state() -> AppState {
    let mut s = AppState { bpm: 123.0, step_division: 16, ..AppState::default() };
    s.append_note(60, Some(111));
    s.append_rest();
    hit(&mut s, 0, 0, Some(127));
    hit(&mut s, 0, 8, Some(90));
    hit(&mut s, 1, 4, None);
    hit(&mut s, 2, 2, Some(64));
    s.drums.channel = 11;
    s
}

#[test]
fn pattern_v2_roundtrip() {
    let s = beat_state();
    let text = serde_json::to_string(&s.to_json(Some("bite_with_drums"))).unwrap();
    let data: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(data["version"], 2);
    assert_eq!(data["bpm"], 123.0);
    assert_eq!(data["step_division"], 16);
    assert_eq!(data["melodic"]["steps"][0]["note"], 60);
    assert_eq!(data["drums"]["channel"], 11);
    let notes: Vec<_> = data["drums"]["lanes"].as_array().unwrap().iter().map(|l| l["note"].clone()).collect();
    assert_eq!(notes, vec![json!("C1"), json!("D1"), json!("F#1"), json!("A#1")]);
    assert_eq!(data["drums"]["lanes"][0]["steps"][0], json!({"active": true, "velocity": 127}));
    assert!(data.get("selected_drum_output").is_none() && data.get("active_track").is_none());

    let restored = AppState::from_json(&data).unwrap();
    assert_eq!(restored.sequence, s.sequence);
    assert_eq!(restored.drums.lanes, s.drums.lanes);
    assert_eq!(restored.drums.channel, 11);
}

#[test]
fn loading_v1_pattern_creates_empty_drums() {
    let v1 = json!({
        "version": 1, "bpm": 99, "step_division": 8, "loop_length": 16, "default_gate": 0.7,
        "transpose": -2, "output_channel": 4,
        "steps": [{"note": 60, "velocity": 111}, {"note": null}],
    });
    let s = AppState::from_json(&v1).unwrap();
    assert_eq!((s.bpm, s.step_division, s.loop_length), (99.0, 8, Some(16)));
    assert_eq!((s.default_gate, s.transpose, s.output_channel), (0.7, -2, 4));
    let pairs: Vec<_> = s.sequence.iter().map(|x| (x.note, x.velocity)).collect();
    assert_eq!(pairs, vec![(Some(60), 111), (None, 100)]);
    assert_eq!(s.drums.lanes.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), vec!["Kick", "Snare", "Closed Hat", "Open Hat"]);
    assert!(!s.drums.has_hits());
    assert_eq!(s.drums.channel, 10);
}

#[test]
fn v1_and_v2_of_the_same_melody_play_identically() {
    let v1 = json!({"version": 1, "bpm": 140, "transpose": 3, "steps": [{"note": 60, "gate": 0.25}]});
    let from_v1 = AppState::from_json(&v1).unwrap();
    let from_v2 = AppState::from_json(&from_v1.to_json(None)).unwrap();
    assert_eq!(from_v2.sequence, from_v1.sequence);
    assert_eq!((from_v2.bpm, from_v2.transpose), (140.0, 3));
}

#[test]
fn v2_drum_lanes_tolerate_partial_and_numeric_data() {
    let data = json!({
        "version": 2,
        "melodic": {"steps": []},
        "drums": {"lanes": [{"name": "Clap", "note": 39, "steps": [{"active": true}]}]},
    });
    let s = AppState::from_json(&data).unwrap();
    assert_eq!(s.drums.lanes.len(), 1);
    assert_eq!(s.drums.lanes[0].note, 39);
    assert_eq!(s.drums.lanes[0].steps[0], DrumStep { active: true, velocity: 100 });
    assert_eq!(s.drums.lanes[0].steps.len(), DRUM_STEPS);
    let e = AppState::from_json(&json!({"version": 2, "drums": {"lanes": [{"name": "x"}]}})).unwrap_err();
    assert!(e.contains("Invalid pattern data"), "{e}");
    let bad_velocity = json!({"lanes": [{"note": "C1", "steps": [{"velocity": 0}]}]});
    let e = AppState::from_json(&json!({"version": 2, "drums": bad_velocity})).unwrap_err();
    assert!(e.contains("Invalid pattern data"), "{e}");
}
