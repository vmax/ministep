//! Parity with the Python build: playback, drum dispatch, anchored grid, timing stats.
//! Ported from legacy/tests/test_sequencer.py, test_drums.py (playback section),
//! test_timing.py and test_clock.py.
//!
//! Python drove `Sequencer.advance_one()` against a fake clock; here the poll-driven
//! [`Transport`] runs against virtual time (see `common::Sim`).
//!
//! Convention difference (deliberate, see `transport.rs`): after a step plays, Python's
//! `state.playhead` / `sequencer.drum_step` point at the *next* step, while Rust's
//! `playhead` / `drum_playhead` show the step that is *sounding*. Assertions below are
//! written in the Rust convention; Python's value is always Rust's + 1 (mod length).

mod common;

use common::{Sim, approx, is_off, is_on};
use ministep_core::clock::{Grid, step_duration_seconds};
use ministep_core::drums::{DRUM_STEPS, DrumStep};
use ministep_core::state::{AppState, Step};
use ministep_core::timing::TimingStats;
use ministep_core::transport::{DRUM_NOTE_LENGTH, Transport};

/// Python's FakeClock handed out 1-second windows; 60 BPM quarter notes are the same.
fn one_second_steps(sequence: Vec<Step>) -> AppState {
    AppState { bpm: 60.0, step_division: 4, sequence, ..AppState::default() }
}

fn hit(s: &mut AppState, lane: usize, step: usize, velocity: u8) {
    s.drums.lanes[lane].steps[step] = DrumStep { active: true, velocity };
}

fn ons(sim: &Sim) -> Vec<(f64, Vec<u8>)> {
    sim.events.iter().filter(|e| is_on(e)).map(|e| (e.0, e.1.clone())).collect()
}

// --- test_sequencer.py -----------------------------------------------------------------
// playback_orders_note_on_then_off_with_gate_and_transpose, rest_only_waits_a_full_step
// and hold_ties_note_across_an_extra_full_step are already covered by tests/transport.rs.

#[test]
fn loop_length_wraps_playback_before_later_sequence_steps() {
    let mut st = one_second_steps((0..10).map(|i| Step::new(Some(60 + i), 100, 0.5)).collect());
    st.loop_length = Some(8);
    st.playhead = 7;
    let mut sim = Sim::new(st, false);
    sim.t.start(&mut sim.s, false); // keep the playhead where it is
    sim.run_steps(2);
    assert_eq!(ons(&sim).iter().map(|e| e.1[1]).collect::<Vec<_>>(), vec![67, 60]);
    assert_eq!(sim.s.playhead, 0); // Python: 1 (next step)
}

// --- test_drums.py: playback -------------------------------------------------------------
// melodic_playback_is_unchanged_without_drums and drum_bar_is_sixteen_steps_regardless_of_
// melodic_loop are already covered by tests/transport.rs.

#[test]
fn drum_playback_sends_note_on_then_short_note_off() {
    let mut st = AppState::default();
    hit(&mut st, 0, 0, 120);
    let mut sim = Sim::new(one_second_steps(vec![]), false);
    sim.s.drums = st.drums;
    sim.start();
    let t0 = sim.now;
    let deadline = sim.poll();
    assert_eq!(sim.events, vec![(t0, vec![0x99, 36, 120], "m")]);
    assert!(approx(deadline, t0 + DRUM_NOTE_LENGTH));
    sim.now = deadline;
    let next = sim.poll();
    assert_eq!(sim.events.last().unwrap().1, vec![0x89, 36, 0]);
    assert!(approx(next, t0 + 1.0), "then the step end");
    assert_eq!(sim.s.drum_playhead, 0); // Python drum_step: 1
}

#[test]
fn multiple_hits_on_one_step_fire_together() {
    let mut st = one_second_steps(vec![]);
    hit(&mut st, 0, 4, 100);
    hit(&mut st, 2, 4, 80);
    hit(&mut st, 3, 4, 60);
    st.drum_playhead = 4;
    let mut sim = Sim::new(st, false);
    sim.t.start(&mut sim.s, false);
    sim.run_until(sim.now + 0.5);
    let on: Vec<_> = sim.events.iter().filter(|e| is_on(e)).map(|e| e.1.clone()).collect();
    assert_eq!(on, vec![vec![0x99, 36, 100], vec![0x99, 42, 80], vec![0x99, 46, 60]]);
    let off: Vec<_> = sim.events.iter().filter(|e| is_off(e)).map(|e| e.1.clone()).collect();
    assert_eq!(off, vec![vec![0x89, 36, 0], vec![0x89, 42, 0], vec![0x89, 46, 0]]);
}

#[test]
fn melodic_and_drums_play_on_the_same_step() {
    let mut st = one_second_steps(vec![Step::new(Some(60), 100, 0.5)]);
    hit(&mut st, 1, 0, 100);
    let mut sim = Sim::new(st, false);
    sim.start();
    let t0 = sim.now;
    sim.run_until(t0 + 1.0);
    let on: Vec<_> = ons(&sim).into_iter().map(|e| e.1).collect();
    assert_eq!(on, vec![vec![0x90, 60, 100], vec![0x99, 38, 100]]);
    // Melodic gate 0.5 -> off at +0.5; drum off at +0.04; next step at +1.
    let off_times: Vec<f64> = sim.events.iter().filter(|e| is_off(e)).map(|e| e.0 - t0).collect();
    assert!(approx(off_times[0], 0.04) && approx(off_times[1], 0.5), "{off_times:?}");
    assert_eq!((sim.s.playhead, sim.s.drum_playhead), (0, 0)); // Python: (0, 1)
}

#[test]
fn drums_use_their_own_output_and_channel() {
    let mut st = one_second_steps(vec![Step::new(Some(60), 100, 0.5)]);
    st.output_channel = 3;
    st.drums.channel = 12;
    hit(&mut st, 0, 0, 100);
    let mut sim = Sim::new(st, true);
    sim.start();
    sim.run_until(sim.now + 1.0);
    let by_port = |port| sim.events.iter().filter(|e| e.2 == port).map(|e| e.1.clone()).collect::<Vec<_>>();
    assert_eq!(by_port("m"), vec![vec![0x92, 60, 100], vec![0x82, 60, 0]]);
    assert_eq!(by_port("d"), vec![vec![0x9B, 36, 100], vec![0x8B, 36, 0]]);
}

#[test]
fn drums_play_when_melodic_sequence_is_empty() {
    let mut st = one_second_steps(vec![]);
    hit(&mut st, 0, 0, 100);
    assert!(Transport::has_content(&st));
    let mut sim = Sim::new(st, false);
    sim.start();
    let t0 = sim.now;
    sim.run_until(t0 + 0.5);
    assert_eq!(ons(&sim), vec![(t0, vec![0x99, 36, 100])]);
    assert_eq!(sim.t.grid.next_start(), Some(t0 + 1.0));

    sim.s.drums.clear();
    assert!(!Transport::has_content(&sim.s));
    sim.run_until(t0 + 3.0);
    assert_eq!(ons(&sim).len(), 1, "nothing to play once the drums are cleared");
}

#[test]
fn stop_releases_hanging_drum_notes() {
    let mut st = one_second_steps(vec![]);
    hit(&mut st, 0, 0, 100);
    hit(&mut st, 1, 0, 100);
    let mut sim = Sim::new(st, false);
    sim.start();
    sim.poll(); // Stop arrives inside the drum gate.
    assert!(sim.events.iter().all(|e| !is_off(e)));
    sim.stop();
    let mut offs: Vec<_> = sim.events.iter().filter(|e| is_off(e)).map(|e| e.1.clone()).collect();
    offs.sort();
    assert_eq!(offs, vec![vec![0x89, 36, 0], vec![0x89, 38, 0]]);
    assert_eq!(sim.outs.melodic.as_ref().unwrap().lock().sounding(), 0);
    // Stopping twice is harmless.
    sim.stop();
    assert_eq!(sim.events.iter().filter(|e| is_off(e)).count(), 2);
}

#[test]
fn drum_note_offs_do_not_drift_the_grid() {
    let bpm = 174.0;
    let mut st = AppState { bpm, step_division: 16, sequence: vec![Step::new(Some(60), 100, 0.5); 16], ..AppState::default() };
    for lane in 0..4 {
        for step in 0..DRUM_STEPS {
            hit(&mut st, lane, step, 100);
        }
    }
    let mut sim = Sim::new(st, false);
    sim.overshoot = 0.0011;
    sim.start();
    let steps = 64;
    sim.run_steps(steps);
    sim.run_until(sim.now + 0.05); // let the last step's note-offs fall due (step is 86 ms)
    sim.stop();

    let duration = step_duration_seconds(bpm, 16);
    let melodic_ons: Vec<f64> = sim.events.iter().filter(|e| is_on(e) && e.1[1] == 60).map(|e| e.0).collect();
    assert_eq!(melodic_ons.len(), steps);
    let first = melodic_ons[0];
    let lateness: Vec<f64> = melodic_ons.iter().enumerate().map(|(i, t)| t - (first + i as f64 * duration)).collect();
    assert!(lateness.iter().cloned().fold(f64::MIN, f64::max) < 0.003, "{lateness:?}");
    assert!(lateness.iter().cloned().fold(f64::MAX, f64::min) >= 0.0);
    let drum_ons: Vec<f64> = sim.events.iter().filter(|e| is_on(e) && e.1[1] != 60).map(|e| e.0).collect();
    let drum_offs: Vec<f64> = sim.events.iter().filter(|e| is_off(e) && e.1[1] != 60).map(|e| e.0).collect();
    assert_eq!(drum_ons.len(), steps * 4);
    assert_eq!(drum_offs.len(), steps * 4);
    let gap = DRUM_NOTE_LENGTH;
    assert!(drum_ons.iter().zip(&drum_offs).all(|(on, off)| (gap - 0.001..=gap + 0.01).contains(&(off - on))));
}

// --- test_timing.py -----------------------------------------------------------------------

fn build(bpm: f64, steps: usize, overshoot: f64) -> Sim {
    let st = AppState {
        bpm,
        step_division: 16,
        default_gate: 0.5,
        sequence: (0..steps).map(|i| Step::new(Some(60 + (i % 12) as u8), 100, 0.5)).collect(),
        ..AppState::default()
    };
    let mut sim = Sim::new(st, false).with_stats();
    sim.overshoot = overshoot;
    sim
}

#[test]
fn sixteenths_do_not_accumulate_drift_over_one_minute() {
    for bpm in [120.0, 150.0, 180.0, 200.0, 240.0] {
        let mut sim = build(bpm, 16, 0.0011);
        sim.start();
        sim.run_until(sim.now + 60.0);
        let summary = sim.t.stats.as_ref().unwrap().summary().unwrap();
        let expected_steps = (60.0 / step_duration_seconds(bpm, 16)).round() as i64;
        assert!((summary.steps as i64 - expected_steps).abs() <= 1, "{bpm}: {} vs {expected_steps}", summary.steps);
        assert!(summary.drift_final.abs() < 0.003, "{bpm}");
        assert!(summary.drift_max_abs < 0.003, "{bpm}");
        assert!(summary.interval_mean_error.abs() < 0.0001, "{bpm}");
        assert_eq!(summary.skipped_steps, 0, "{bpm}");
    }
}

#[test]
fn step_deadlines_come_from_the_grid_not_the_wake_time() {
    let mut grid = Grid::default();
    let first = grid.next_step(1000.0, 120.0, 16); // 125 ms steps
    let second = grid.next_step(first.end() + 0.030, 120.0, 16); // woke 30 ms late
    assert!(approx(second.start, first.end()));
    assert!(approx(second.late, 0.030));
    assert_eq!(second.skipped, 0);
    let third = grid.next_step(second.end(), 120.0, 16); // on time again
    assert!(approx(third.start, first.start + 2.0 * 0.125));
    assert!(approx(third.late, 0.0));
}

#[test]
fn stall_of_several_steps_skips_slots_and_keeps_the_grid() {
    let mut grid = Grid::default();
    let first = grid.next_step(1000.0, 120.0, 16);
    let window = grid.next_step(first.end() + 0.125 * 2.0 + 0.010, 120.0, 16);
    assert_eq!(window.skipped, 2);
    assert!(approx(window.start, first.start + 3.0 * 0.125));
    assert!(approx(window.late, 0.010));
}

#[test]
fn sequencer_advances_playhead_past_skipped_slots() {
    let mut sim = build(120.0, 8, 0.0);
    sim.start();
    sim.run_steps(2); // steps 0 and 1
    assert_eq!(sim.s.playhead, 1);
    let off_deadline = sim.poll();
    sim.wake_at(off_deadline);
    let boundary = sim.poll(); // step 1's note-off; next is slot 2
    sim.now = boundary + 0.125 * 3.0 + 0.001; // stall: slots 2, 3, 4 are missed
    sim.poll();
    let notes: Vec<u8> = ons(&sim).iter().map(|e| e.1[1]).collect();
    assert_eq!(notes, vec![60, 61, 65]);
    assert_eq!(sim.s.playhead, 5); // Python: 6
    assert_eq!(sim.t.stats.as_ref().unwrap().skipped_steps, 3);
}

#[test]
fn tempo_change_reanchors_from_the_current_step_boundary() {
    let mut sim = build(120.0, 16, 0.0005);
    sim.start();
    sim.run_steps(4);
    let boundary = sim.t.grid.next_start().unwrap();
    sim.s.set_bpm(180.0);
    let before = sim.t.last_step_at;
    while sim.t.last_step_at == before {
        let d = sim.poll();
        if sim.t.last_step_at == before {
            sim.wake_at(d);
        }
    }
    assert!(approx(sim.t.last_step_at.unwrap(), boundary));
    assert!(approx(sim.t.grid.next_start().unwrap(), boundary + step_duration_seconds(180.0, 16)));
}

#[test]
fn gate_and_note_off_stay_on_the_grid_when_late() {
    let mut sim = build(120.0, 2, 0.0);
    sim.stalls.insert(3, 0.020); // wakes: 1 off0, 2 step 1, 3 off1 (20 ms late)
    sim.start();
    sim.run_steps(3);
    let on: Vec<f64> = sim.events.iter().filter(|e| is_on(e)).map(|e| e.0).collect();
    let off: Vec<f64> = sim.events.iter().filter(|e| is_off(e)).map(|e| e.0).collect();
    assert!(approx(off[0] - on[0], 0.0625));
    assert!(approx(on[1], on[0] + 0.125));
    assert!(approx(off[1] - on[1], 0.0625 + 0.020), "the late off is sent when we wake");
    assert!(approx(on[2], on[0] + 0.25), "but the grid did not move");
}

#[test]
fn repeated_float_addition_is_negligible_for_transport() {
    let duration = step_duration_seconds(240.0, 32);
    let origin = 3_000_000.0;
    let steps = (3600.0 / duration) as u64;
    let mut accumulated: f64 = origin;
    for _ in 0..steps {
        accumulated += duration;
    }
    assert!((accumulated - (origin + steps as f64 * duration)).abs() < 1e-6);
}

#[test]
fn timing_stats_summary_reports_drift_and_percentiles() {
    let mut stats = TimingStats::default();
    let duration = 0.1;
    for index in 0..100 {
        let late = if index % 10 != 0 { 0.001 } else { 0.010 };
        let scheduled = f64::from(index) * duration;
        stats.record(scheduled, scheduled + late, 0.0001, duration, 0);
    }
    let s = stats.summary().unwrap();
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-6 * b.abs().max(1e-12);
    assert_eq!(s.steps, 100);
    assert!(close(s.lateness_p50, 0.001), "{}", s.lateness_p50);
    assert!(close(s.lateness_max, 0.010));
    assert!(close(s.drift_final, 0.001));
    assert!(close(s.interval_mean_error, -0.009 / 99.0), "{}", s.interval_mean_error);
    assert!(close(s.interval_jitter_max, 0.009));
}

/// Wall-clock check, opt-in like the Python `MINISTEP_REALTIME_TESTS` test.
#[test]
#[ignore = "wall-clock timing: run with `cargo test -- --ignored`"]
fn realtime_sixteenths_at_180_bpm_hold_the_grid() {
    use ministep_core::engine::{Engine, EngineConfig};
    use ministep_core::output::testing::Recorder;
    use ministep_core::transport::Outputs;

    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("timing.log");
    let st = AppState { bpm: 180.0, step_division: 16, sequence: (0..16).map(|i| Step::new(Some(60 + i), 100, 0.5)).collect(), ..AppState::default() };
    let rec = Recorder::default();
    let config = EngineConfig {
        patterns: ministep_core::patterns::PatternStore::new(dir.path()),
        sequence_path: dir.path().join("seq.json"),
        timing_log: Some(log.clone()),
        ..Default::default()
    };
    let engine = Engine::new(st, Outputs { synth: None, melodic: Some(rec.output()), drums: None }, config);
    engine.play();
    std::thread::sleep(std::time::Duration::from_secs(10));
    engine.stop();
    let text = std::fs::read_to_string(&log).unwrap();
    let field = |key: &str| -> f64 {
        let rest = &text[text.find(key).unwrap_or_else(|| panic!("{key} in {text}")) + key.len()..];
        rest.split_whitespace().next().unwrap().trim_end_matches("ms").parse().unwrap()
    };
    assert_eq!(field("skipped="), 0.0, "{text}");
    assert!(field("final=").abs() < 10.0, "{text}");
    assert!(field("p95=") < 10.0, "{text}");
}

// --- test_clock.py ---------------------------------------------------------------------------

#[test]
fn step_duration_calculations() {
    let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
    assert!(close(step_duration_seconds(120.0, 4), 0.5));
    assert!(close(step_duration_seconds(120.0, 8), 0.25));
    assert!(close(step_duration_seconds(120.0, 16), 0.125));
    assert!(close(step_duration_seconds(128.0, 16), 60.0 / 128.0 / 4.0));
}
