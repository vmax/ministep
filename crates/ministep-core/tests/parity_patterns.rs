//! Parity with the Python build: named pattern store and engine pattern wiring.
//! Ported from legacy/tests/test_patterns.py (names, store, legacy files, runtime)
//! and the persistence/runtime parts of legacy/tests/test_drums.py.

mod common;

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

use common::{EngineOpts, engine};
use ministep_core::drums::{DRUM_STEPS, DrumStep};
use ministep_core::patterns::{PatternKind, PatternStore, sanitize_pattern_name};
use ministep_core::state::{AppState, Track};
use serde_json::{Value, json};

const MELODIC: PatternKind = PatternKind::Melodic;
const DRUMS: PatternKind = PatternKind::Drums;

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

fn melodic_view(s: &AppState) -> Value {
    Value::Object(s.melodic_to_json())
}

fn drum_view(s: &AppState) -> Value {
    Value::Object(s.drums.to_json())
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn beat_state() -> AppState {
    let mut s = AppState { bpm: 123.0, step_division: 16, ..AppState::default() };
    s.append_note(60, Some(111));
    s.append_rest();
    for (lane, step, velocity) in [(0, 0, 127), (0, 8, 90), (1, 4, 100), (2, 2, 64)] {
        s.drums.lanes[lane].steps[step] = DrumStep { active: true, velocity };
    }
    s.drums.channel = 11;
    s
}

// --- names and paths ---------------------------------------------------------------

#[test]
fn sanitize_pattern_name_cases() {
    for (raw, expected) in [
        ("bite_test", "bite_test"),
        ("Egypt 01", "Egypt_01"),
        ("  spaced  out ", "spaced_out"),
        ("../../etc/passwd", "etc_passwd"),
        ("..\\..\\win", "win"),
        ("/abs/path", "abs_path"),
        (".hidden", "hidden"),
        ("name.json", "name"),
        ("ünïcödé!!x", "n_c_d_x"),
    ] {
        assert_eq!(sanitize_pattern_name(raw).unwrap(), expected, "{raw:?}");
    }
}

#[test]
fn sanitize_rejects_empty_names() {
    for raw in ["", "   ", "..", "///", "!!!"] {
        assert!(sanitize_pattern_name(raw).is_err(), "{raw:?}");
    }
}

#[test]
fn path_traversal_stays_inside_directory() {
    let dir = tempfile::tempdir().unwrap();
    let store = PatternStore::new(dir.path().join("patterns"));
    for kind in PatternKind::ALL {
        for raw in ["../escape", "../../etc/passwd", "a/../../b", "/tmp/evil"] {
            let path = store.path_for(kind, raw).unwrap();
            assert_eq!(path.parent().unwrap(), dir.path().join("patterns").join(kind.as_str()));
            assert_eq!(path.extension().unwrap(), "json");
        }
    }
}

// --- store -----------------------------------------------------------------------------

#[test]
fn melodic_save_writes_only_into_melodic_directory() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("nested/patterns");
    let store = PatternStore::new(&root);
    assert_eq!(store.save(MELODIC, &rich_state(), "bite test").unwrap(), "bite_test");
    let path = root.join("melodic/bite_test.json");
    let data = read(&path);
    assert_eq!(data["version"], 1);
    assert_eq!(data["type"], "melodic");
    assert_eq!(data["name"], "bite test");
    for key in ["bpm", "step_division", "drums"] {
        assert!(data.get(key).is_none(), "{key}");
    }
    assert_eq!(data["steps"][0]["note"], 60);
    assert_eq!(data["transpose"], -3);
    assert_eq!(store.list_names(MELODIC), vec!["bite_test"]);
    assert!(store.list_names(DRUMS).is_empty());
    assert!(!root.join("drums").exists());
    let leftovers: Vec<_> =
        fs::read_dir(path.parent().unwrap()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with('.')).collect();
    assert!(leftovers.is_empty());
}

#[test]
fn drum_save_writes_only_into_drums_directory() {
    let dir = tempfile::tempdir().unwrap();
    let store = PatternStore::new(dir.path());
    let mut s = rich_state();
    s.drums.lanes[0].steps[0] = DrumStep { active: true, velocity: 110 };
    assert_eq!(store.save(DRUMS, &s, "electro 01").unwrap(), "electro_01");
    let data = read(&dir.path().join("drums/electro_01.json"));
    assert_eq!((data["version"].clone(), data["type"].clone(), data["name"].clone()), (json!(1), json!("drums"), json!("electro 01")));
    for key in ["bpm", "steps", "transpose"] {
        assert!(data.get(key).is_none(), "{key}");
    }
    let mut steps = vec![json!({"active": true, "velocity": 110})];
    steps.extend(std::iter::repeat_n(json!({"active": false, "velocity": 100}), 15));
    assert_eq!(data["lanes"][0], json!({"name": "Kick", "note": "C1", "steps": steps}));
    assert_eq!(store.list_names(DRUMS), vec!["electro_01"]);
    assert!(store.list_names(MELODIC).is_empty());
    assert!(!dir.path().join("melodic").exists());
}

#[test]
fn loading_melodic_leaves_drums_and_tempo_alone() {
    let dir = tempfile::tempdir().unwrap();
    let store = PatternStore::new(dir.path());
    store.save(MELODIC, &rich_state(), "melody").unwrap();
    let mut s = AppState { bpm: 99.0, step_division: 8, selected_drum_output: Some("IAC".into()), ..AppState::default() };
    s.drums.lanes[1].steps[3] = DrumStep { active: true, velocity: 77 };
    s.drums.channel = 12;
    s.drums.lanes[0].note = 35;
    let before_drums = drum_view(&s);
    s.active_track = Track::Drums;

    assert_eq!(store.load(MELODIC, &mut s, "melody").unwrap(), "melody");
    assert_eq!(melodic_view(&s), melodic_view(&rich_state()));
    assert_eq!(drum_view(&s), before_drums);
    assert_eq!((s.bpm, s.step_division), (99.0, 8));
    assert_eq!(s.selected_drum_output.as_deref(), Some("IAC"));
    assert_eq!(s.active_track, Track::Drums);
    assert_eq!((s.playhead, s.cursor), (0, 0));
}

#[test]
fn loading_drums_leaves_melodic_state_alone() {
    let dir = tempfile::tempdir().unwrap();
    let store = PatternStore::new(dir.path());
    let mut beat = AppState::default();
    beat.drums.lanes[2].steps[0] = DrumStep { active: true, velocity: 45 };
    beat.drums.channel = 11;
    store.save(DRUMS, &beat, "beat").unwrap();

    let mut s = rich_state();
    s.set_bpm(77.0);
    s.drums.cursor_step = 9;
    s.drums.audition.insert(0, 100);
    let before = melodic_view(&s);
    assert_eq!(store.load(DRUMS, &mut s, "beat").unwrap(), "beat");
    assert_eq!(melodic_view(&s), before);
    assert_eq!(s.sequence, rich_state().sequence);
    assert_eq!(s.bpm, 77.0);
    assert_eq!(drum_view(&s), drum_view(&beat));
    assert_eq!(s.drums.cursor_step, 0);
    assert!(s.drums.audition.is_empty());
}

/// Python injected an `os.replace` failure ("No space left on device"); here the
/// failure is a read-only directory, which exercises the same guarantee: the
/// existing pattern stays intact and no temp file is left behind.
#[test]
fn overwrite_is_atomic_for_both_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let store = PatternStore::new(dir.path());
    let mut first = AppState { transpose: 1, ..AppState::default() };
    first.append_note(60, None);
    first.drums.lanes[0].steps[0].active = true;
    let mut second = AppState { transpose: 2, ..AppState::default() };
    second.append_note(72, None);
    second.drums.lanes[3].steps[15].active = true;
    let targets = [(MELODIC, dir.path().join("melodic/p.json")), (DRUMS, dir.path().join("drums/p.json"))];

    for (kind, path) in &targets {
        store.save(*kind, &first, "p").unwrap();
        let inode_before = fs::metadata(path).unwrap().ino();
        store.save(*kind, &second, "p").unwrap();
        let data = read(path);
        if *kind == MELODIC {
            assert_eq!(data["transpose"], 2);
            assert_eq!(data["steps"][0]["note"], 72);
        } else {
            assert_eq!(data["lanes"][3]["steps"][15]["active"], true);
            assert_eq!(data["lanes"][0]["steps"][0]["active"], false);
        }
        assert_ne!(fs::metadata(path).unwrap().ino(), inode_before);
    }

    if nix_is_root() {
        return;
    }
    for (kind, path) in &targets {
        let parent = path.parent().unwrap();
        fs::set_permissions(parent, fs::Permissions::from_mode(0o555)).unwrap();
        let err = store.save(*kind, &first, "p").unwrap_err();
        fs::set_permissions(parent, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(err.0.contains("Cannot write"), "{err}");
        assert_eq!(read(path)["name"], "p");
        let names: Vec<_> = fs::read_dir(parent).unwrap().flatten().map(|e| e.file_name().into_string().unwrap()).collect();
        assert_eq!(names, vec!["p.json"]);
    }
}

fn nix_is_root() -> bool {
    // A file in a 0o555 directory is still creatable by root.
    let probe = tempfile::tempdir().unwrap();
    fs::set_permissions(probe.path(), fs::Permissions::from_mode(0o555)).unwrap();
    let root = fs::write(probe.path().join("x"), "x").is_ok();
    fs::set_permissions(probe.path(), fs::Permissions::from_mode(0o755)).unwrap();
    root
}

#[test]
fn corrupted_files_are_pattern_errors_for_both_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let store = PatternStore::new(dir.path());
    fs::create_dir(dir.path().join("melodic")).unwrap();
    fs::create_dir(dir.path().join("drums")).unwrap();
    fs::write(dir.path().join("melodic/bad.json"), "{not json").unwrap();
    fs::write(dir.path().join("drums/bad.json"), "[1, 2]").unwrap();
    fs::write(dir.path().join("drums/broken.json"), json!({"version": 1, "type": "drums", "lanes": [{"name": "x"}]}).to_string())
        .unwrap();
    let mut s = rich_state();
    let before = (melodic_view(&s), drum_view(&s));
    let e = store.load(MELODIC, &mut s, "bad").unwrap_err();
    assert!(e.0.contains("not valid JSON"), "{e}");
    let e = store.load(DRUMS, &mut s, "bad").unwrap_err();
    assert!(e.0.contains("JSON object"), "{e}");
    let e = store.load(DRUMS, &mut s, "broken").unwrap_err();
    assert!(e.0.contains("needs a 'note'"), "{e}");
    assert_eq!((melodic_view(&s), drum_view(&s)), before);
}

#[test]
fn wrong_kind_and_unsupported_version_are_readable_errors() {
    let dir = tempfile::tempdir().unwrap();
    let store = PatternStore::new(dir.path());
    store.save(DRUMS, &AppState::default(), "beat").unwrap();
    store.save(MELODIC, &AppState::default(), "tune").unwrap();
    fs::write(dir.path().join("drums/future.json"), json!({"version": 9, "type": "drums", "lanes": []}).to_string()).unwrap();
    fs::write(dir.path().join("melodic/alien.json"), json!({"version": 1, "type": "bass"}).to_string()).unwrap();
    let mut s = AppState::default();
    fs::copy(dir.path().join("drums/beat.json"), dir.path().join("melodic/beat.json")).unwrap();
    let e = store.load(MELODIC, &mut s, "beat").unwrap_err();
    assert!(e.0.contains("is a drum pattern, not a melodic pattern"), "{e}");
    let e = store.load(DRUMS, &mut s, "future").unwrap_err();
    assert!(e.0.contains("unsupported drum pattern version 9"), "{e}");
    let e = store.load(MELODIC, &mut s, "alien").unwrap_err();
    assert!(e.0.contains("unknown pattern type 'bass'"), "{e}");
    // Python: list_names("bass") raises "Unknown pattern kind"; Rust: the kind does not parse.
    assert_eq!(PatternKind::parse("bass"), None);
}

#[test]
fn missing_pattern_and_missing_directory() {
    let dir = tempfile::tempdir().unwrap();
    let store = PatternStore::new(dir.path().join("nope"));
    assert!(store.list_names(MELODIC).is_empty() && store.list_names(DRUMS).is_empty());
    for kind in PatternKind::ALL {
        let e = store.load(kind, &mut AppState::default(), "ghost").unwrap_err();
        assert!(e.0.contains("Pattern not found: ghost"), "{e}");
    }
}

#[test]
fn permission_denied_is_a_pattern_error() {
    if nix_is_root() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let locked = dir.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
    let results: Vec<_> = PatternKind::ALL.iter().map(|&k| PatternStore::new(&locked).save(k, &AppState::default(), "x")).collect();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    for r in results {
        let e = r.unwrap_err();
        assert!(e.0.contains("Cannot write"), "{e}");
    }
}

#[test]
fn delete_and_rename() {
    let dir = tempfile::tempdir().unwrap();
    let store = PatternStore::new(dir.path());
    store.save(MELODIC, &rich_state(), "old").unwrap();
    store.save(DRUMS, &rich_state(), "old").unwrap();
    assert_eq!(store.rename(MELODIC, "old", "new name").unwrap(), "new_name");
    assert_eq!(store.list_names(MELODIC), vec!["new_name"]);
    assert_eq!(store.list_names(DRUMS), vec!["old"]);
    assert_eq!(read(&dir.path().join("melodic/new_name.json"))["name"], "new name");
    assert!(store.rename(MELODIC, "old", "x").unwrap_err().0.contains("not found"));
    store.delete(MELODIC, "new name").unwrap();
    assert!(store.list_names(MELODIC).is_empty());
    assert!(store.delete(MELODIC, "new name").unwrap_err().0.contains("not found"));
    assert_eq!(store.list_names(DRUMS), vec!["old"]);
}

// --- legacy combined files -----------------------------------------------------------

fn legacy_combined(root: &Path) -> AppState {
    let mut s = rich_state();
    s.drums.lanes[0].steps[0] = DrumStep { active: true, velocity: 123 };
    s.drums.channel = 13;
    fs::write(root.join("combo.json"), serde_json::to_string(&s.to_json(Some("combo"))).unwrap()).unwrap();
    s
}

#[test]
fn old_combined_pattern_loads_only_melodic_in_melodic_mode() {
    let dir = tempfile::tempdir().unwrap();
    let store = PatternStore::new(dir.path());
    let old = legacy_combined(dir.path());
    assert_eq!(store.list_names(MELODIC), vec!["combo"]);
    assert_eq!(store.list_names(DRUMS), vec!["combo"]);
    let mut s = AppState { bpm: 60.0, ..AppState::default() };
    s.drums.lanes[3].steps[7].active = true;
    let before_drums = drum_view(&s);
    store.load(MELODIC, &mut s, "combo").unwrap();
    assert_eq!(melodic_view(&s), melodic_view(&old));
    assert_eq!(drum_view(&s), before_drums);
    assert_eq!(s.bpm, 60.0);
}

#[test]
fn old_combined_pattern_loads_only_drums_in_drums_mode() {
    let dir = tempfile::tempdir().unwrap();
    let store = PatternStore::new(dir.path());
    let old = legacy_combined(dir.path());
    let mut s = AppState { bpm: 60.0, ..AppState::default() };
    s.append_note(40, None);
    let before = melodic_view(&s);
    store.load(DRUMS, &mut s, "combo").unwrap();
    assert_eq!(drum_view(&s), drum_view(&old));
    assert_eq!(melodic_view(&s), before);
    assert_eq!(s.bpm, 60.0);
}

#[test]
fn old_flat_v1_file_is_melodic_only() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("flat.json"), json!({"version": 1, "bpm": 140, "transpose": 3, "steps": [{"note": 61}]}).to_string())
        .unwrap();
    let store = PatternStore::new(dir.path());
    let mut s = AppState::default();
    store.load(MELODIC, &mut s, "flat").unwrap();
    assert_eq!(s.sequence.iter().map(|x| x.note).collect::<Vec<_>>(), vec![Some(61)]);
    assert_eq!(s.transpose, 3);
    assert!(store.load(DRUMS, &mut s, "flat").unwrap_err().0.contains("no drum section"));
}

#[test]
fn saving_after_legacy_load_writes_split_files_and_keeps_the_old_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = PatternStore::new(dir.path());
    legacy_combined(dir.path());
    let mut s = AppState::default();
    store.load(MELODIC, &mut s, "combo").unwrap();
    store.save(MELODIC, &s, "combo").unwrap();
    store.load(DRUMS, &mut s, "combo").unwrap();
    store.save(DRUMS, &s, "combo").unwrap();
    assert!(dir.path().join("combo.json").is_file());
    assert_eq!(read(&dir.path().join("melodic/combo.json"))["type"], "melodic");
    assert_eq!(read(&dir.path().join("drums/combo.json"))["type"], "drums");
    assert_eq!(store.list_names(MELODIC), vec!["combo"]);
    assert!(store.exists(DRUMS, "combo"));
}

// --- test_drums.py: store --------------------------------------------------------------

#[test]
fn store_save_and_load_restores_drum_steps() {
    let dir = tempfile::tempdir().unwrap();
    let store = PatternStore::new(dir.path());
    let s = beat_state();
    store.save(DRUMS, &s, "beat").unwrap();
    let on_disk = read(&dir.path().join("drums/beat.json"));
    assert_eq!(on_disk["version"], 1);
    assert_eq!(on_disk["type"], "drums");
    let mut loaded = AppState::default();
    store.load(DRUMS, &mut loaded, "beat").unwrap();
    assert_eq!(loaded.drums.lanes, s.drums.lanes);
    assert_eq!(loaded.drums.lanes[0].steps[0].velocity, 127);
    assert_eq!(loaded.drums.lanes[2].steps[2].velocity, 64);
    assert_eq!(loaded.drums.lanes[3].steps, [DrumStep::default(); DRUM_STEPS]);
}

// --- runtime (engine) integration ----------------------------------------------------------

#[test]
fn runtime_save_modify_load_restores_melodic_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = rich_state();
    s.selected_input = Some("IN".into());
    s.selected_output = Some("OUT".into());
    let expected = melodic_view(&s);
    let e = engine(s, dir.path(), EngineOpts { melodic: false, ..Default::default() }).engine;

    assert!(e.save_pattern("egypt_01", None));
    assert_eq!(e.state().status_message, "Saved melodic pattern: egypt_01");

    e.edit(|s| {
        s.clear();
        s.set_bpm(60.0);
        s.transpose = 5;
        s.octave = -2;
        s.set_loop_length(Some(8)).unwrap();
        s.append_note(40, None);
        s.drums.lanes[0].steps[0].active = true;
    });
    assert_ne!(melodic_view(&e.state()), expected);

    assert!(e.load_pattern("egypt_01", None));
    let s = e.state();
    assert_eq!(s.status_message, "Loaded melodic pattern: egypt_01");
    assert_eq!(melodic_view(&s), expected);
    assert_eq!((s.selected_input.as_deref(), s.selected_output.as_deref()), (Some("IN"), Some("OUT")));
    assert!(!s.playing);
    assert_eq!(s.playhead, 0);
    assert_eq!(s.bpm, 60.0);
    assert!(s.drums.lanes[0].steps[0].active);
}

#[test]
fn runtime_failed_load_keeps_current_state() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("patterns/melodic")).unwrap();
    fs::write(dir.path().join("patterns/melodic/bad.json"), "nope").unwrap();
    let s = rich_state();
    let before = (melodic_view(&s), drum_view(&s));
    let e = engine(s, dir.path(), EngineOpts { melodic: false, ..Default::default() }).engine;
    assert!(!e.load_pattern("bad", None));
    assert!(e.state().status_message.starts_with("Failed to load: bad.json"), "{}", e.state().status_message);
    assert!(!e.load_pattern("missing", None));
    assert_eq!(e.state().status_message, "Failed to load: Pattern not found: missing");
    assert!(!e.save_pattern("...", None));
    assert!(e.state().status_message.starts_with("Failed to save:"));
    let s = e.state();
    assert_eq!((melodic_view(&s), drum_view(&s)), before);
}

#[test]
fn runtime_load_pattern_keeps_drum_port_and_active_track() {
    let dir = tempfile::tempdir().unwrap();
    PatternStore::new(dir.path().join("patterns")).save(DRUMS, &beat_state(), "beat").unwrap();
    let s = AppState {
        selected_output: Some("Synth".into()),
        selected_drum_output: Some("IAC Bus".into()),
        active_track: Track::Drums,
        ..AppState::default()
    };
    let e = engine(s, dir.path(), EngineOpts { melodic: false, ..Default::default() }).engine;
    assert!(e.load_pattern("beat", None));
    let s = e.state();
    assert!(s.drums.has_hits());
    assert_eq!((s.selected_output.as_deref(), s.selected_drum_output.as_deref()), (Some("Synth"), Some("IAC Bus")));
    assert_eq!(s.active_track, Track::Drums);
}
