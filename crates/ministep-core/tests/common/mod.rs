//! Shared helpers for the Python-parity tests (`parity_*.rs`).
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use ministep_core::engine::{Engine, EngineConfig};
use ministep_core::output::testing::Recorder;
use ministep_core::patterns::PatternStore;
use ministep_core::state::AppState;
use ministep_core::timing::TimingStats;
use ministep_core::transport::{Outputs, Transport};
use ministep_core::{Mapping, MidiMessage};

/// One emitted MIDI message: (time, raw bytes, "m" melodic port / "d" drum port).
pub type Ev = (f64, Vec<u8>, &'static str);

pub fn is_on(e: &Ev) -> bool {
    e.1[0] & 0xF0 == 0x90
}

pub fn is_off(e: &Ev) -> bool {
    e.1[0] & 0xF0 == 0x80
}

pub fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// Tiny deterministic PRNG (xorshift64*) standing in for Python's `random.Random`.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let x = (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64;
        lo + (hi - lo) * x
    }
}

/// Poll-driven transport against virtual time.
///
/// Mirrors the Python `VirtualTime` fake: each wake-up lands `overshoot * U(0.8, 1.4)`
/// after the requested deadline, and `stalls[n]` adds extra delay to the n-th wake-up.
pub struct Sim {
    pub t: Transport,
    pub s: AppState,
    pub outs: Outputs,
    pub mel: Recorder,
    pub drum: Recorder,
    pub now: f64,
    pub overshoot: f64,
    pub rng: Rng,
    pub wakes: usize,
    pub stalls: HashMap<usize, f64>,
    pub events: Vec<Ev>,
}

impl Sim {
    pub fn new(s: AppState, separate_drums: bool) -> Self {
        let mel = Recorder::default();
        let drum = Recorder::default();
        let outs = Outputs { melodic: Some(mel.output()), drums: separate_drums.then(|| drum.output()) };
        Self {
            t: Transport::default(),
            s,
            outs,
            mel,
            drum,
            now: 1000.0,
            overshoot: 0.0,
            rng: Rng::new(1),
            wakes: 0,
            stalls: HashMap::new(),
            events: Vec::new(),
        }
    }

    pub fn with_stats(mut self) -> Self {
        self.t = Transport::new(Some(TimingStats::default()));
        self
    }

    pub fn start(&mut self) {
        self.t.start(&mut self.s, true);
    }

    /// Poll once at `self.now`; collect what it sent; return the next deadline.
    pub fn poll(&mut self) -> f64 {
        let fixed = self.now;
        let clock = move || fixed;
        let deadline = self.t.poll(self.now, &mut self.s, &self.outs, &clock);
        self.collect();
        deadline
    }

    pub fn collect(&mut self) {
        for e in self.mel.take() {
            self.events.push((self.now, e, "m"));
        }
        for e in self.drum.take() {
            self.events.push((self.now, e, "d"));
        }
    }

    /// Sleep until `deadline` the way the real thread would, with modelled overshoot/stalls.
    pub fn wake_at(&mut self, deadline: f64) {
        self.wakes += 1;
        let jitter = if self.overshoot > 0.0 { self.overshoot * self.rng.uniform(0.8, 1.4) } else { 0.0 };
        self.now = self.now.max(deadline) + jitter + self.stalls.remove(&self.wakes).unwrap_or(0.0);
    }

    /// Poll at every deadline until virtual time reaches `end`.
    pub fn run_until(&mut self, end: f64) {
        loop {
            let deadline = self.poll();
            if deadline >= end {
                self.now = end;
                return;
            }
            self.wake_at(deadline);
        }
    }

    /// Poll until `n` grid steps have been emitted (counted by `last_step_at` changes).
    pub fn run_steps(&mut self, n: usize) {
        let mut seen = 0;
        while seen < n {
            let before = self.t.last_step_at;
            let deadline = self.poll();
            if self.t.last_step_at != before {
                seen += 1;
                if seen == n {
                    return;
                }
            }
            self.wake_at(deadline);
        }
    }

    pub fn stop(&mut self) {
        self.t.stop(&mut self.s, &self.outs);
        self.collect();
    }
}

/// An engine with recording outputs, pattern store and mapping file under `dir`.
pub struct TestEngine {
    pub engine: Arc<Engine>,
    pub mel: Recorder,
    pub drum: Recorder,
}

pub struct EngineOpts {
    pub melodic: bool,
    pub drums: bool,
    pub mapping: Option<Mapping>,
    pub profile: Option<ministep_core::profiles::ControlProfile>,
}

impl Default for EngineOpts {
    fn default() -> Self {
        Self { melodic: true, drums: false, mapping: None, profile: None }
    }
}

pub fn engine(state: AppState, dir: &Path, opts: EngineOpts) -> TestEngine {
    let mel = Recorder::default();
    let drum = Recorder::default();
    let outs = Outputs { melodic: opts.melodic.then(|| mel.output()), drums: opts.drums.then(|| drum.output()) };
    let profile = opts.profile.map(|p| ministep_core::engine::ProfileRouter::new(p, mel.output()));
    let config = EngineConfig {
        patterns: PatternStore::new(dir.join("patterns")),
        sequence_path: dir.join("sequence.json"),
        mapping: opts.mapping,
        mapping_path: Some(dir.join("mapping.json")),
        encoder_menu: true,
        profile,
        realtime: false,
        ..Default::default()
    };
    TestEngine { engine: Engine::new(state, outs, config), mel, drum }
}

/// An engine with no outputs at all (Python `MiniStepRuntime(state, output=None)`).
pub fn bare_engine(state: AppState, dir: &Path) -> Arc<Engine> {
    engine(state, dir, EngineOpts { melodic: false, ..Default::default() }).engine
}

pub fn note_on(note: u8, velocity: u8) -> MidiMessage {
    MidiMessage::NoteOn { channel: 0, note, velocity }
}

pub fn note_off(note: u8) -> MidiMessage {
    MidiMessage::NoteOff { channel: 0, note, velocity: 0 }
}

pub fn cc(control: u8, value: u8) -> MidiMessage {
    MidiMessage::ControlChange { channel: 0, control, value }
}
