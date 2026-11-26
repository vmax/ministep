//! Deterministic playback simulations against a fake clock.

use ministep_core::drums::DrumStep;
use ministep_core::output::testing::Recorder;
use ministep_core::state::{AppState, Step};
use ministep_core::transport::{DRUM_NOTE_LENGTH, Outputs, Transport};

struct Sim {
    t: Transport,
    s: AppState,
    outs: Outputs,
    mel: Recorder,
    drum: Recorder,
    now: f64,
}

type Ev = (f64, Vec<u8>, &'static str);

impl Sim {
    fn new(s: AppState, separate_drums: bool) -> Self {
        let mel = Recorder::default();
        let drum = Recorder::default();
        let outs = Outputs { melodic: Some(mel.output()), drums: separate_drums.then(|| drum.output()) };
        Self { t: Transport::default(), s, outs, mel, drum, now: 100.0 }
    }

    fn start(&mut self) {
        self.t.start(&mut self.s, true);
    }

    /// Run poll at every returned deadline until `end` (exclusive); events with timestamps.
    fn run(&mut self, end: f64) -> Vec<Ev> {
        let mut events = Vec::new();
        let fixed = self.now;
        let clock = move || fixed;
        loop {
            let deadline = self.t.poll(self.now, &mut self.s, &self.outs, &clock);
            for e in self.mel.take() {
                events.push((self.now, e, "m"));
            }
            for e in self.drum.take() {
                events.push((self.now, e, "d"));
            }
            if deadline >= end {
                self.now = end;
                return events;
            }
            assert!(deadline >= self.now, "deadline went backwards");
            self.now = deadline;
        }
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn state_with(steps: Vec<Step>) -> AppState {
    AppState { bpm: 60.0, step_division: 4, sequence: steps, ..AppState::default() } // 1 s steps
}

#[test]
fn note_on_then_off_with_gate_and_transpose() {
    let mut st = state_with(vec![Step::new(Some(60), 110, 0.5)]);
    st.transpose = 2;
    let mut sim = Sim::new(st, false);
    sim.start();
    let ev = sim.run(101.0);
    assert_eq!(ev.len(), 2);
    assert!(close(ev[0].0, 100.0) && ev[0].1 == vec![0x90, 62, 110]);
    assert!(close(ev[1].0, 100.5) && ev[1].1 == vec![0x80, 62, 0]);
    assert_eq!(sim.s.playhead, 0, "playhead shows the sounding step");
}

#[test]
fn rest_is_silent() {
    let mut sim = Sim::new(state_with(vec![Step::new(None, 100, 0.5)]), false);
    sim.start();
    assert!(sim.run(103.0).is_empty());
}

#[test]
fn hold_ties_across_extra_step() {
    let mut a = Step::new(Some(60), 100, 0.5);
    a.tie = true;
    let b = Step::new(Some(60), 100, 1.0);
    let mut sim = Sim::new(state_with(vec![a, b]), false);
    sim.start();
    let ev = sim.run(101.5);
    assert_eq!(ev, vec![(100.0, vec![0x90, 60, 100], "m")], "tied step must not retrigger");
    let ev2 = sim.run(102.5);
    // gate 1.0 off lands exactly on the next boundary, before the next on.
    assert!(close(ev2[0].0, 102.0) && ev2[0].1 == vec![0x80, 60, 0], "{ev2:?}");
    assert_eq!(ev2[1].1, vec![0x90, 60, 100]);
}

#[test]
fn loop_length_wraps_before_later_steps() {
    let steps = (0..10).map(|i| Step::new(Some(60 + i), 100, 0.5)).collect();
    let mut st = state_with(steps);
    st.loop_length = Some(8);
    let mut sim = Sim::new(st, false);
    sim.start();
    let ons: Vec<u8> = sim.run(110.0).into_iter().filter(|e| e.1[0] == 0x90).map(|e| e.1[1]).collect();
    assert_eq!(ons, vec![60, 61, 62, 63, 64, 65, 66, 67, 60, 61]);
}

#[test]
fn drums_fire_short_notes_on_their_own_port_and_channel() {
    let mut st = state_with(vec![]);
    st.drums.lanes[0].steps[0] = DrumStep { active: true, velocity: 120 };
    st.drums.lanes[2].steps[0] = DrumStep { active: true, velocity: 80 };
    let mut sim = Sim::new(st, true);
    sim.start();
    let ev = sim.run(100.5);
    assert_eq!(
        ev,
        vec![
            (100.0, vec![0x99, 36, 120], "d"),
            (100.0, vec![0x99, 42, 80], "d"),
            (100.0 + DRUM_NOTE_LENGTH, vec![0x89, 36, 0], "d"),
            (100.0 + DRUM_NOTE_LENGTH, vec![0x89, 42, 0], "d"),
        ]
    );
}

#[test]
fn drum_bar_is_sixteen_steps_regardless_of_melodic_loop() {
    let mut st = state_with((0..3).map(|i| Step::new(Some(60 + i), 100, 0.5)).collect());
    st.drums.lanes[0].steps[0] = DrumStep { active: true, velocity: 100 };
    let mut sim = Sim::new(st, false);
    sim.start();
    let kicks: Vec<f64> = sim.run(133.0).into_iter().filter(|e| e.1 == vec![0x99, 36, 100]).map(|e| e.0).collect();
    assert_eq!(kicks, vec![100.0, 116.0, 132.0]);
}

#[test]
fn stop_releases_hanging_notes() {
    let mut a = Step::new(Some(60), 100, 0.5);
    a.tie = true;
    let mut st = state_with(vec![a]);
    st.drums.lanes[1].steps[0] = DrumStep { active: true, velocity: 100 };
    let mut sim = Sim::new(st, false);
    sim.start();
    sim.run(100.01);
    sim.t.stop(&mut sim.s, &sim.outs);
    let offs = sim.mel.take();
    assert!(offs.contains(&vec![0x80, 60, 0]) && offs.contains(&vec![0x89, 38, 0]), "{offs:?}");
    assert_eq!(sim.outs.melodic.as_ref().unwrap().lock().sounding(), 0);
}

#[test]
fn late_polls_keep_the_grid_and_stalls_drop_slots() {
    let steps = (0..16).map(|i| Step::new(Some(60 + i), 100, 0.5)).collect();
    let mut sim = Sim::new(state_with(steps), false);
    sim.start();
    sim.run(100.1);
    // Wake 3.5 steps late: three slots are dropped, playhead stays in phase with time.
    sim.now = 104.5;
    let ev = sim.run(104.6);
    let on = ev.iter().find(|e| e.1[0] == 0x90).unwrap();
    assert_eq!(on.1[1], 64, "step 4 plays at t=104.5");
    assert_eq!(sim.s.playhead, 4);
    assert_eq!(sim.t.grid.next_start(), Some(105.0));
}

#[test]
fn hour_long_grid_does_not_drift() {
    let mut sim = Sim::new(AppState { bpm: 240.0, step_division: 32, sequence: vec![Step::new(None, 100, 0.5)], ..AppState::default() }, false);
    sim.start();
    sim.run(100.0 + 3600.0);
    let next = sim.t.grid.next_start().unwrap();
    let d = 60.0 / 240.0 * 4.0 / 32.0;
    let n = ((next - 100.0) / d).round();
    assert!((next - (100.0 + n * d)).abs() < 1e-6);
}
