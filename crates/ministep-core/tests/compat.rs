//! Files written by the Python build must load, and re-saving must be byte-identical.

use std::path::PathBuf;

use ministep_core::patterns::{PatternKind, PatternStore, read_json};
use ministep_core::state::AppState;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn snapshot_v2_round_trips_byte_identical() {
    let path = fixtures().join("snapshot_v2.json");
    let mut s = AppState::default();
    s.load(&path).unwrap();
    assert_eq!(s.bpm, 133.5);
    assert_eq!(s.step_division, 8);
    assert_eq!(s.sequence.len(), 5);
    assert!(s.sequence[2].tie && s.sequence[3].gate == 1.0);
    assert!(s.sequence[1].accent && s.sequence[1].note.is_none());
    assert_eq!(s.drums.channel, 11);
    assert_eq!(s.drums.lanes[0].steps[0].velocity, 117);
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("sequence.json");
    s.save(&out).unwrap();
    assert_eq!(std::fs::read_to_string(&out).unwrap(), std::fs::read_to_string(&path).unwrap());
}

#[test]
fn named_patterns_round_trip_byte_identical() {
    let src = PatternStore::new(fixtures().join("patterns"));
    assert_eq!(src.list_names(PatternKind::Melodic), vec!["Egypt_01", "old_flat"]);
    assert_eq!(src.list_names(PatternKind::Drums), vec!["Electro_beat", "old_flat"]);
    let mut s = AppState::default();
    src.load(PatternKind::Melodic, &mut s, "Egypt 01").unwrap();
    src.load(PatternKind::Drums, &mut s, "Electro beat").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dst = PatternStore::new(dir.path());
    dst.save(PatternKind::Melodic, &s, "Egypt 01").unwrap();
    dst.save(PatternKind::Drums, &s, "Electro beat").unwrap();
    for rel in ["melodic/Egypt_01.json", "drums/Electro_beat.json"] {
        assert_eq!(
            std::fs::read_to_string(dir.path().join(rel)).unwrap(),
            std::fs::read_to_string(fixtures().join("patterns").join(rel)).unwrap(),
            "{rel}"
        );
    }
}

#[test]
fn legacy_flat_v1_loads_melodic_only() {
    let store = PatternStore::new(fixtures().join("patterns"));
    let mut s = AppState::default();
    store.load(PatternKind::Melodic, &mut s, "old_flat").unwrap();
    assert_eq!(s.sequence.len(), 2);
    assert_eq!(s.sequence[1].velocity, 100);
    let err = store.load(PatternKind::Drums, &mut s, "old_flat").unwrap_err();
    assert!(err.0.contains("no drum section"), "{err}");
    // Wrong type is refused and leaves state untouched.
    let before = s.sequence.clone();
    let err = store.load(PatternKind::Melodic, &mut s, "Electro beat").unwrap_err();
    assert!(err.0.contains("not found") || err.0.contains("drum pattern"), "{err}");
    assert_eq!(s.sequence, before);
    let v1 = AppState::from_json(&read_json(&fixtures().join("patterns/old_flat.json")).unwrap()).unwrap();
    assert_eq!(v1.bpm, 120.0);
    assert!(!v1.drums.has_hits());
}

#[test]
fn drum_voice_params_round_trip_and_stay_optional() {
    use ministep_core::drums::DrumTrack;
    use ministep_core::voice::{VoiceModel, VoiceParams};
    let mut t = DrumTrack::default();
    // Untouched voices are not written, so older readers see the same file.
    assert!(!serde_json::to_string(&t.to_json()).unwrap().contains("synth"));
    t.lanes[1].voice.model = VoiceModel::Clap;
    t.lanes[1].voice.decay = 0.25;
    let back = DrumTrack::from_json(&serde_json::Value::Object(t.to_json())).unwrap();
    assert_eq!(back.lanes[1].voice.model, VoiceModel::Clap);
    assert!((back.lanes[1].voice.decay - 0.25).abs() < 1e-6);
    assert_eq!(back.lanes[0].voice, VoiceParams::for_note(36));
}
