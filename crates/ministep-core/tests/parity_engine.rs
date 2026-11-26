//! Parity with the Python build: the runtime (Rust `Engine`) — record mode, MIDI learn,
//! controller mappings, main-encoder menu, synth profiles, drum audition over MIDI.
//! Ported from the Python build's tests/test_record_mode.py, test_midi_learn.py, test_controller.py,
//! test_main_encoder_menu.py, test_profiles.py, test_clock.py, test_minilab_mapping.py
//! and the MIDI/runtime sections of test_drums.py.

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;

use common::{EngineOpts, TestEngine, bare_engine, cc, engine, note_off, note_on};
use ministep_core::controller::AbsoluteCc;
use ministep_core::drums::DrumStep;
use ministep_core::engine::EngineEvent;
use ministep_core::patterns::{PatternKind, PatternStore};
use ministep_core::profiles::{load_profile, parse_profile};
use ministep_core::state::{AppState, Step, Track};
use ministep_core::{Command, CommandName as C, Mapping, MidiMessage};

fn notes(s: &AppState) -> Vec<Option<u8>> {
    s.sequence.iter().map(|x| x.note).collect()
}

fn note_vel(s: &AppState) -> Vec<(Option<u8>, u8)> {
    s.sequence.iter().map(|x| (x.note, x.velocity)).collect()
}

// --- test_record_mode.py ------------------------------------------------------------------

#[test]
fn record_mode_commits_each_physical_note_without_using_wall_clock() {
    let dir = tempfile::tempdir().unwrap();
    let e = bare_engine(AppState::default(), dir.path());
    e.execute(Command::new(C::RECORD_TOGGLE));
    assert!(e.state().recording);
    assert!(!e.state().playing);
    e.handle_midi(note_on(60, 91));
    std::thread::sleep(std::time::Duration::from_millis(1));
    e.handle_midi(note_on(63, 102));
    std::thread::sleep(std::time::Duration::from_millis(2));
    e.handle_midi(note_on(67, 113));
    assert_eq!(note_vel(&e.state()), vec![(Some(60), 91), (Some(63), 102), (Some(67), 113)]);
    // (Python also checked steps carry no timestamp: `Step` has no such field.)
}

#[test]
fn note_off_does_not_record_and_record_off_restores_explicit_commit() {
    let dir = tempfile::tempdir().unwrap();
    let e = bare_engine(AppState::default(), dir.path());
    e.execute(Command::with(C::RECORD, 1.0));
    e.handle_midi(note_on(60, 90));
    e.handle_midi(note_off(60));
    assert_eq!(notes(&e.state()), vec![Some(60)]);

    e.execute(Command::new(C::RECORD_TOGGLE));
    e.handle_midi(note_on(64, 101));
    assert_eq!(notes(&e.state()), vec![Some(60)]);
    e.execute(Command::new(C::COMMIT));
    assert_eq!(note_vel(&e.state()), vec![(Some(60), 90), (Some(64), 101)]);
}

#[test]
fn armed_root_uses_next_midi_key_without_recording_it() {
    let dir = tempfile::tempdir().unwrap();
    let st = AppState {
        sequence: vec![Step::new(Some(60), 100, 0.5), Step::new(Some(63), 100, 0.5)],
        recording: true,
        ..AppState::default()
    };
    let e = bare_engine(st, dir.path());
    e.execute(Command::new(C::ARM_ROOT_CAPTURE));
    e.handle_midi(note_on(62, 100));
    let s = e.state();
    assert!(!s.root_capture_armed);
    assert_eq!(s.transpose, 0);
    assert_eq!(notes(&s), vec![Some(62), Some(65)]);
}

// --- test_midi_learn.py ---------------------------------------------------------------------

fn learn_engine(dir: &std::path::Path) -> (TestEngine, PathBuf) {
    let te = engine(AppState::default(), dir, EngineOpts { melodic: false, mapping: Some(Mapping::default()), ..Default::default() });
    (te, dir.join("mapping.json"))
}

#[test]
fn midi_learn_maps_button_cc_once_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let (te, path) = learn_engine(dir.path());
    let e = &te.engine;
    assert!(e.arm_midi_learn(C::RECORD_TOGGLE));
    e.handle_midi(cc(74, 127));
    assert!(path.exists());
    {
        let control = e.control();
        let mapping = control.mapping.as_ref().unwrap();
        assert!(mapping.command_for(&cc(74, 127)).is_some());
        assert!(mapping.command_for(&cc(74, 0)).is_none());
    }
    let restored = Mapping::load(&path).unwrap();
    assert_eq!(restored.command_for(&cc(74, 127)).unwrap().name, C::RECORD_TOGGLE);
}

#[test]
fn midi_learn_uses_absolute_range_for_bpm() {
    let mut mapping = Mapping::default();
    mapping.learn_cc(16, C::SET_BPM);
    let command = mapping.command_for(&cc(16, 127)).unwrap();
    assert_eq!(command.name, C::SET_BPM);
    assert_eq!(command.value, Some(240.0));
}

#[test]
fn midi_learn_maps_a_pad_note_only_on_its_midi_channel() {
    let dir = tempfile::tempdir().unwrap();
    let (te, path) = learn_engine(dir.path());
    assert!(te.engine.arm_midi_learn(C::PLAY_STOP));
    te.engine.handle_midi(MidiMessage::NoteOn { channel: 9, note: 40, velocity: 100 });
    let restored = Mapping::load(&path).unwrap();
    assert_eq!(restored.command_for(&MidiMessage::NoteOn { channel: 9, note: 40, velocity: 100 }), Some(Command::new(C::PLAY_STOP)));
    assert_eq!(restored.command_for(&MidiMessage::NoteOn { channel: 0, note: 40, velocity: 100 }), None);
}

// --- test_controller.py ------------------------------------------------------------------------

#[test]
fn pad_note_and_absolute_cc_translate_to_commands() {
    let mapping = Mapping {
        note_commands: BTreeMap::from([((9, 36), C::COMMIT)]),
        cc_commands: BTreeMap::new(),
        absolute_cc: vec![AbsoluteCc { control: 16, command: C::SET_BPM, minimum: 40.0, maximum: 240.0, integer: true }],
    };
    let on = |channel, velocity| MidiMessage::NoteOn { channel, note: 36, velocity };
    assert_eq!(mapping.command_for(&on(9, 100)), Some(Command::new(C::COMMIT)));
    assert_eq!(mapping.command_for(&on(0, 100)), None);
    assert_eq!(mapping.command_for(&on(9, 0)), None);
    assert_eq!(mapping.command_for(&cc(16, 127)), Some(Command::with(C::SET_BPM, 240.0)));
}

// --- test_minilab_mapping.py ----------------------------------------------------------------------

/// Python's `MINILAB3_MAPPING` is deliberately empty; in Rust that is `Mapping::default()`.
#[test]
fn starter_minilab_mapping_does_not_consume_musical_notes_or_ccs() {
    let mapping = Mapping::default();
    assert_eq!(mapping.command_for(&note_on(37, 100)), None);
    assert_eq!(mapping.command_for(&cc(16, 127)), None);
}

// --- test_main_encoder_menu.py ---------------------------------------------------------------------

fn screen(a: &str, b: &str) -> Option<(String, String)> {
    Some((a.into(), b.into()))
}

#[test]
fn main_encoder_browses_and_edits_the_menu() {
    let dir = tempfile::tempdir().unwrap();
    let e = bare_engine(AppState { bpm: 128.0, default_gate: 0.5, ..AppState::default() }, dir.path());
    assert!(e.handle_main_encoder(&cc(118, 127)));
    assert_eq!(e.menu_screen(), screen("MENU > BPM", "128 BPM"));
    assert!(e.handle_main_encoder(&cc(28, 65)));
    assert_eq!(e.menu_screen(), screen("MENU > GATE", "0.50"));

    assert!(e.handle_main_encoder(&cc(118, 127)));
    assert_eq!(e.menu_screen(), screen("EDIT > GATE", "0.50"));
    assert!(e.handle_main_encoder(&cc(28, 66)));
    assert_eq!(e.state().default_gate, 0.6);
    assert_eq!(e.menu_screen(), screen("EDIT > GATE", "0.60"));
}

#[test]
fn shift_click_closes_the_main_encoder_menu() {
    let dir = tempfile::tempdir().unwrap();
    let e = bare_engine(AppState::default(), dir.path());
    e.handle_main_encoder(&cc(115, 127));
    assert!(e.handle_main_encoder(&cc(119, 127)));
    assert_eq!(e.menu_screen(), None);
}

#[test]
fn main_encoder_menu_changes_loop_length() {
    let dir = tempfile::tempdir().unwrap();
    let e = bare_engine(AppState { loop_length: None, ..AppState::default() }, dir.path());
    e.handle_main_encoder(&cc(118, 127));
    e.handle_main_encoder(&cc(28, 68));
    assert_eq!(e.menu_screen(), screen("MENU > LOOP LENGTH", "FULL"));
    e.handle_main_encoder(&cc(118, 127));
    e.handle_main_encoder(&cc(28, 65));
    assert_eq!(e.state().loop_length, Some(8));
    assert_eq!(e.menu_screen(), screen("EDIT > LOOP LENGTH", "8 STEPS"));
}

#[test]
fn main_encoder_menu_selects_a_control_profile_page() {
    let dir = tempfile::tempdir().unwrap();
    let profile = parse_profile(
        "name: T\npages:\n  FILTER:\n    knobs:\n      1: { cc: 20, label: CUTOFF }\n  OSC:\n    knobs:\n      1: { cc: 82, label: OSC1 }\n",
    )
    .unwrap();
    let te = engine(AppState::default(), dir.path(), EngineOpts { melodic: false, profile: Some(profile), ..Default::default() });
    let e = &te.engine;
    e.handle_main_encoder(&cc(118, 127));
    e.handle_main_encoder(&cc(28, 69));
    assert_eq!(e.menu_screen(), screen("MENU > CONTROL PAGE", "FILTER"));
    e.handle_main_encoder(&cc(118, 127));
    e.handle_main_encoder(&cc(28, 65));
    assert_eq!(e.control().profile.as_ref().unwrap().page_index, 1);
    assert_eq!(e.menu_screen(), screen("EDIT > CONTROL PAGE", "OSC"));
}

// --- test_profiles.py --------------------------------------------------------------------------------

/// Python tested `ProfileRouter.handle_message`; in Rust knob routing lives in
/// `Engine::handle_midi`, so the router is exercised through the engine.
#[test]
fn profile_routes_knobs_by_active_page() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.yaml");
    std::fs::write(
        &path,
        "name: Test Synth\nchannel: 2\npages:\n  FILTER:\n    knobs:\n      1: { cc: 20, label: CUTOFF }\n  OSC:\n    knobs:\n      1: { cc: 82, label: OSC1 }\n",
    )
    .unwrap();
    let profile = load_profile(&path).unwrap();
    let te = engine(AppState::default(), dir.path(), EngineOpts { profile: Some(profile), ..Default::default() });
    let e = &te.engine;
    let events = e.events();

    e.handle_midi(cc(74, 96));
    assert_eq!(te.mel.take(), vec![vec![0xB1, 20, 96]]);
    let routed = events.try_iter().find(|ev| matches!(ev, EngineEvent::Knob { .. }));
    assert_eq!(routed, Some(EngineEvent::Knob { page: "FILTER".into(), label: "CUTOFF".into(), value: 96 }));

    e.control().profile.as_mut().unwrap().page_index = 1; // select_page(1)
    e.handle_midi(cc(74, 31));
    assert_eq!(te.mel.take(), vec![vec![0xB1, 82, 31]]);
    events.try_iter().for_each(drop);
    // Knob 2 is a knob message but unbound on OSC: nothing is routed (or passed through).
    e.handle_midi(cc(71, 31));
    assert!(te.mel.take().is_empty());
    assert!(!events.try_iter().any(|ev| matches!(ev, EngineEvent::Knob { .. })));
}

#[test]
fn profile_rejects_invalid_knob_cc() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.yaml");
    std::fs::write(&path, "name: Broken\npages:\n  FILTER:\n    knobs:\n      1: { cc: 128, label: CUTOFF }\n").unwrap();
    let err = load_profile(&path).unwrap_err();
    assert!(err.0.contains("between 0 and 127"), "{err}");
}

#[test]
fn bundled_miniverse_profile_loads() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../profiles/miniverse.yaml");
    let profile = load_profile(&path).unwrap();
    assert_eq!(profile.name, "Miniverse");
    assert_eq!(profile.page_names(), vec!["CONTROLLERS", "OSCILLATOR BANK", "MIXER", "MODIFIERS", "OUTPUT"]);
}

// --- test_clock.py -------------------------------------------------------------------------------------

#[test]
fn step_division_cycles_through_musical_values() {
    let dir = tempfile::tempdir().unwrap();
    let e = bare_engine(AppState { step_division: 16, ..AppState::default() }, dir.path());
    e.execute(Command::new(C::STEP_DIVISION_UP));
    assert_eq!(e.state().step_division, 32);
    e.execute(Command::new(C::STEP_DIVISION_UP));
    assert_eq!(e.state().step_division, 4);
    e.execute(Command::new(C::STEP_DIVISION_DOWN));
    assert_eq!(e.state().step_division, 32);
}

// --- test_drums.py: runtime and MIDI audition ----------------------------------------------------------

fn drum_engine(dir: &std::path::Path) -> TestEngine {
    let st = AppState { active_track: Track::Drums, ..AppState::default() };
    engine(st, dir, EngineOpts { drums: true, ..Default::default() })
}

#[test]
fn runtime_stop_sends_all_notes_off_to_separate_drum_output() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::default();
    st.drums.lanes[0].steps[0].active = true;
    let te = engine(st, dir.path(), EngineOpts { drums: true, ..Default::default() });
    te.engine.play(); // only drums have content
    assert!(te.engine.state().playing);
    te.engine.stop();
    let all_off = |rec: &ministep_core::output::testing::Recorder| rec.take().iter().any(|m| m[..2] == [0xB0, 123]);
    assert!(all_off(&te.mel) && all_off(&te.drum));
    assert!(!te.engine.state().playing);
}

#[test]
fn drum_midi_note_auditions_its_lane_on_the_drum_output() {
    for (note, lane, name) in [(36, 0, "Kick"), (38, 1, "Snare"), (42, 2, "Closed Hat"), (46, 3, "Open Hat")] {
        let dir = tempfile::tempdir().unwrap();
        let te = drum_engine(dir.path());
        te.engine.handle_midi(note_on(note, 93));
        {
            let s = te.engine.state();
            assert_eq!(s.drums.audition, BTreeMap::from([(lane, 93)]));
            assert_eq!(s.drums.cursor_lane, lane);
            assert!(s.status_message.contains(name) && s.status_message.contains("93"), "{}", s.status_message);
            assert!(!s.drums.has_hits());
            assert_eq!(s.last_note_played, None);
            assert!(s.held_notes.is_empty());
        }
        assert_eq!(te.drum.take(), vec![vec![0x99, note, 93]]);
        assert!(te.mel.take().is_empty());
        te.engine.handle_midi(note_off(note));
        assert_eq!(te.drum.take(), vec![vec![0x89, note, 0]]);
    }
}

#[test]
fn unknown_midi_note_creates_no_drum_hit_and_passes_through() {
    let dir = tempfile::tempdir().unwrap();
    let te = drum_engine(dir.path());
    te.engine.handle_midi(note_on(60, 80));
    assert!(te.engine.state().drums.audition.is_empty());
    assert!(te.drum.take().is_empty());
    assert_eq!(te.mel.take(), vec![vec![0x90, 60, 80]]);
    assert_eq!(te.engine.state().last_note_played, Some((60, 80)));
    assert!(!te.engine.commit());
    assert!(!te.engine.state().drums.has_hits());
}

#[test]
fn commit_writes_auditioned_hit_with_midi_velocity_and_advances_step() {
    let dir = tempfile::tempdir().unwrap();
    let te = drum_engine(dir.path());
    te.engine.handle_midi(note_on(42, 93));
    assert!(te.engine.commit());
    let s = te.engine.state();
    assert_eq!(s.drums.lanes[2].steps[0], DrumStep { active: true, velocity: 93 });
    assert_eq!(s.drums.cursor_step, 1);
    assert!(s.drums.audition.is_empty());
    assert_eq!(s.status_message, "Committed Closed Hat on step 1.");
}

#[test]
fn kick_plus_hat_audition_commits_both_and_repeat_updates_velocity() {
    let dir = tempfile::tempdir().unwrap();
    let te = drum_engine(dir.path());
    te.engine.handle_midi(note_on(36, 120));
    te.engine.handle_midi(note_on(42, 70));
    te.engine.handle_midi(note_on(42, 95));
    assert_eq!(te.engine.state().drums.audition, BTreeMap::from([(0, 120), (2, 95)]));
    assert!(te.engine.commit());
    let s = te.engine.state();
    assert_eq!(s.drums.lanes[0].steps[0], DrumStep { active: true, velocity: 120 });
    assert_eq!(s.drums.lanes[2].steps[0], DrumStep { active: true, velocity: 95 });
    assert!(!s.drums.lanes[1].steps[0].active);
    assert_eq!(s.drums.cursor_step, 1);
    assert!(s.drums.audition.is_empty());
    assert_eq!(s.status_message, "Committed Kick + Closed Hat on step 1.");
}

#[test]
fn rest_skips_a_drum_step_and_drops_the_audition() {
    let dir = tempfile::tempdir().unwrap();
    let te = drum_engine(dir.path());
    te.engine.handle_midi(note_on(38, 100));
    te.engine.execute(Command::new(C::REST));
    let s = te.engine.state();
    assert!(s.drums.audition.is_empty());
    assert_eq!(s.drums.cursor_step, 1);
    assert!(!s.drums.has_hits());
    assert!(s.sequence.is_empty());
}

#[test]
fn midi_recorded_velocities_survive_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let te = drum_engine(dir.path());
    let e = &te.engine;
    e.handle_midi(note_on(36, 127));
    e.commit();
    e.handle_midi(note_on(46, 33));
    e.handle_midi(note_on(36, 64));
    e.commit();
    assert!(e.save_pattern("live", None));
    assert!(dir.path().join("patterns/drums/live.json").is_file());
    let mut loaded = AppState::default();
    PatternStore::new(dir.path().join("patterns")).load(PatternKind::Drums, &mut loaded, "live").unwrap();
    assert_eq!(loaded.drums.lanes[0].steps[0].velocity, 127);
    assert_eq!(loaded.drums.lanes[0].steps[1].velocity, 64);
    assert_eq!(loaded.drums.lanes[3].steps[1].velocity, 33);
    assert!(loaded.drums.audition.is_empty());
}

#[test]
fn melodic_midi_workflow_is_unchanged_when_melodic_track_is_active() {
    let dir = tempfile::tempdir().unwrap();
    let te = drum_engine(dir.path());
    te.engine.edit(|s| s.active_track = Track::Melodic);
    te.engine.handle_midi(note_on(36, 90));
    assert_eq!(te.mel.take(), vec![vec![0x90, 36, 90]]);
    assert!(te.drum.take().is_empty());
    assert!(te.engine.state().drums.audition.is_empty());
    assert_eq!(te.engine.state().last_note_played, Some((36, 90)));
    assert!(te.engine.commit());
    assert_eq!(notes(&te.engine.state()), vec![Some(36)]);
    assert!(!te.engine.state().drums.has_hits());
}
