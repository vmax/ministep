//! The transport thread must never wait for the editor's state lock.

use std::time::Duration;

use ministep_core::engine::{Engine, EngineConfig};
use ministep_core::output::testing::Recorder;
use ministep_core::patterns::PatternStore;
use ministep_core::state::{AppState, Step};
use ministep_core::transport::Outputs;

#[test]
fn playback_continues_while_the_state_lock_is_held() {
    let dir = tempfile::tempdir().unwrap();
    let rec = Recorder::default();
    let state = AppState {
        bpm: 240.0,
        step_division: 16, // 62.5 ms steps
        sequence: (0..16).map(|i| Step::new(Some(60 + i), 100, 0.5)).collect(),
        ..AppState::default()
    };
    let engine = Engine::new(
        state,
        Outputs { melodic: Some(rec.output()), drums: None, synth: None },
        EngineConfig {
            patterns: PatternStore::new(dir.path()),
            sequence_path: dir.path().join("sequence.json"),
            realtime: false,
            ..EngineConfig::default()
        },
    );
    engine.play();
    std::thread::sleep(Duration::from_millis(30));
    rec.take();
    {
        // A slow GUI frame / remote client holding the lock for half a second.
        let _held = engine.state();
        std::thread::sleep(Duration::from_millis(500));
    }
    let ons = rec.take().into_iter().filter(|m| m[0] == 0x90).count();
    engine.stop();
    // 500 ms / 62.5 ms = 8 steps; allow scheduler slack.
    assert!(ons >= 7, "only {ons} notes while the state lock was held");
    assert!(!engine.state().playing);
}

#[test]
fn edits_reach_the_transport_and_playhead_is_live() {
    let dir = tempfile::tempdir().unwrap();
    let rec = Recorder::default();
    let state = AppState { bpm: 240.0, sequence: vec![Step::new(Some(60), 100, 0.5)], ..AppState::default() };
    let engine = Engine::new(
        state,
        Outputs { melodic: Some(rec.output()), drums: None, synth: None },
        EngineConfig { patterns: PatternStore::new(dir.path()), sequence_path: dir.path().join("s.json"), realtime: false, ..EngineConfig::default() },
    );
    engine.play();
    std::thread::sleep(Duration::from_millis(20));
    engine.edit(|s| s.transpose = 12);
    std::thread::sleep(Duration::from_millis(150));
    engine.edit(|s| s.sequence.push(Step::new(Some(64), 100, 0.5)));
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(15));
        seen.insert(engine.state().playhead);
    }
    engine.stop();
    let notes: Vec<u8> = rec.take().into_iter().filter(|m| m[0] == 0x90).map(|m| m[1]).collect();
    assert!(notes.contains(&72), "transpose edit not applied: {notes:?}");
    assert!(notes.contains(&76), "appended step not played: {notes:?}");
    assert!(seen.contains(&0) && seen.contains(&1), "playhead not live: {seen:?}");
}
