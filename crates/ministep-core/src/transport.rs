//! Playback logic as a pure, poll-driven state machine.
//!
//! [`Transport::poll`] is called with the current time; it emits every MIDI
//! event that is due and returns the next deadline. The real-time thread in
//! [`crate::engine`] sleeps until that deadline; tests drive it with a fake
//! clock and get bit-exact, deterministic schedules.
//!
//! Both tracks advance on the same grid step, so they can never drift apart.
//! Within one poll, due note-offs are always sent before the step's note-ons.

use crate::clock::{Grid, StepWindow};
use crate::drums::DRUM_STEPS;
use crate::output::{Owner, SharedOutput};
use crate::state::AppState;
use crate::timing::TimingStats;

/// Drum hits are fixed-length triggers, independent of the melodic gate.
pub const DRUM_NOTE_LENGTH: f64 = 0.040;
/// Poll interval while playing with nothing to play.
const IDLE_POLL: f64 = 0.05;
/// Deadline returned while stopped (the engine is woken explicitly on play).
const STOPPED_POLL: f64 = 1.0;

#[derive(Clone, Default)]
pub struct Outputs {
    pub melodic: Option<SharedOutput>,
    /// `None`: drums share the melodic port.
    pub drums: Option<SharedOutput>,
}

impl Outputs {
    pub fn drum_sink(&self) -> Option<&SharedOutput> {
        self.drums.as_ref().or(self.melodic.as_ref())
    }

    pub fn all_notes_off(&self) {
        if let Some(m) = &self.melodic {
            m.lock().all_notes_off();
        }
        if let Some(d) = &self.drums {
            let same = self.melodic.as_ref().is_some_and(|m| std::sync::Arc::ptr_eq(m, d));
            if !same {
                d.lock().all_notes_off();
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PendingOff {
    at: f64,
    drums: bool,
    note: u8,
    channel: u8,
}

#[derive(Debug, Default)]
pub struct Transport {
    pub grid: Grid,
    /// Note sounding through a tie, with its (zero-based) channel.
    active_tie: Option<(u8, u8)>,
    pending: Vec<PendingOff>,
    melodic_advance: bool,
    drum_advance: bool,
    pub stats: Option<TimingStats>,
    /// Time of the last emitted step, for UI flashing.
    pub last_step_at: Option<f64>,
}

impl Transport {
    pub fn new(stats: Option<TimingStats>) -> Self {
        Self { stats, ..Self::default() }
    }

    pub fn has_content(state: &AppState) -> bool {
        !state.sequence.is_empty() || state.drums.has_hits()
    }

    /// Prepare for playback. `restart` rewinds both tracks.
    pub fn start(&mut self, state: &mut AppState, restart: bool) {
        if restart {
            state.playhead = 0;
            state.drum_playhead = 0;
            self.melodic_advance = false;
            self.drum_advance = false;
        }
        if !state.playing || restart {
            // A fresh run must not inherit a stale grid (it would look like a stall).
            self.grid.reset();
        }
        state.playing = true;
    }

    /// Stop and release everything this transport owns.
    pub fn stop(&mut self, state: &mut AppState, outs: &Outputs) {
        state.playing = false;
        self.grid.reset();
        self.release_tie(outs);
        let pending = std::mem::take(&mut self.pending);
        for off in pending {
            Self::send_off(outs, off);
        }
        self.melodic_advance = false;
        self.drum_advance = false;
    }

    /// Emit everything due at `now`; return the next deadline.
    pub fn poll(&mut self, now: f64, state: &mut AppState, outs: &Outputs, clock: &dyn Fn() -> f64) -> f64 {
        self.flush_due(now, outs);
        if !state.playing {
            return self.next_deadline(now + STOPPED_POLL);
        }
        if !Self::has_content(state) {
            self.release_tie(outs);
            self.grid.reset();
            return self.next_deadline(now + IDLE_POLL);
        }
        if self.grid.next_start().is_none_or(|t| now >= t) {
            self.advance(now, state, outs, clock);
        }
        let next = self.grid.next_start().unwrap_or(now + IDLE_POLL);
        self.next_deadline(next)
    }

    fn next_deadline(&self, fallback: f64) -> f64 {
        self.pending.iter().map(|p| p.at).fold(fallback, f64::min)
    }

    fn flush_due(&mut self, now: f64, outs: &Outputs) {
        if self.pending.iter().all(|p| p.at > now) {
            return;
        }
        let mut due: Vec<PendingOff> = Vec::new();
        self.pending.retain(|p| {
            if p.at <= now {
                due.push(*p);
                false
            } else {
                true
            }
        });
        due.sort_by(|a, b| a.at.total_cmp(&b.at));
        for off in due {
            Self::send_off(outs, off);
        }
    }

    fn send_off(outs: &Outputs, off: PendingOff) {
        let (sink, owner) = if off.drums {
            (outs.drum_sink(), Owner::Drums)
        } else {
            (outs.melodic.as_ref(), Owner::Sequencer)
        };
        if let Some(sink) = sink {
            sink.lock().note_off(off.note, off.channel, owner);
        }
    }

    fn release_tie(&mut self, outs: &Outputs) {
        if let Some((note, channel)) = self.active_tie.take()
            && let Some(m) = &outs.melodic {
                m.lock().note_off(note, channel, Owner::Sequencer);
            }
    }

    /// Play exactly one grid step on both tracks.
    fn advance(&mut self, now: f64, state: &mut AppState, outs: &Outputs, clock: &dyn Fn() -> f64) {
        let window = self.grid.next_step(now, state.bpm, state.step_division);
        let skipped = window.skipped as usize;
        // After a stall the grid drops whole slots; move both positions past them.
        let drum_step = usize::from(self.drum_advance) + skipped;
        state.drum_playhead = (state.drum_playhead + drum_step) % DRUM_STEPS;
        self.drum_advance = true;

        if !state.sequence.is_empty() {
            let loop_length = state.playback_length().max(1);
            let step_by = usize::from(self.melodic_advance) + skipped;
            state.playhead = (state.playhead + step_by) % loop_length;
            self.melodic_advance = true;
            self.play_melodic(&window, state, outs);
        }
        self.fire_drums(&window, state, outs);
        self.last_step_at = Some(window.start);
        if let Some(stats) = &mut self.stats {
            stats.record(window.start, now, clock() - now, window.duration, window.skipped);
        }
    }

    fn play_melodic(&mut self, window: &StepWindow, state: &AppState, outs: &Outputs) {
        let step = state.sequence[state.playhead];
        let note = state.playback_note(&step);
        let channel = state.output_channel.saturating_sub(1);
        if self.active_tie.is_some_and(|(n, _)| Some(n) != note) {
            self.release_tie(outs);
        }
        let continuing = note.is_some() && self.active_tie.map(|(n, _)| n) == note;
        let Some(note) = note else { return };
        if !continuing
            && let Some(m) = &outs.melodic {
                m.lock().note_on(note, step.velocity, channel, Owner::Sequencer);
            }
        if step.tie {
            let tie_channel = self.active_tie.map_or(channel, |(_, c)| c);
            self.active_tie = Some((note, tie_channel));
        } else {
            let off_channel = self.active_tie.take().map_or(channel, |(_, c)| c);
            self.pending.push(PendingOff { at: window.start + window.duration * step.gate, drums: false, note, channel: off_channel });
        }
    }

    fn fire_drums(&mut self, window: &StepWindow, state: &AppState, outs: &Outputs) {
        let Some(sink) = outs.drum_sink() else { return };
        let channel = state.drums.channel.saturating_sub(1);
        let mut sink = sink.lock();
        for (lane, hit) in state.drums.hits_at(state.drum_playhead) {
            sink.note_on(lane.note, hit.velocity, channel, Owner::Drums);
            self.pending.push(PendingOff { at: window.start + DRUM_NOTE_LENGTH, drums: true, note: lane.note, channel });
        }
    }
}
