//! Voice scheduling and mixing, independent of the audio backend.
//!
//! Hits arrive stamped with monotonic time. The mixer keeps a smoothed mapping
//! from its own sample counter to monotonic time and starts every hit on the
//! exact frame `at + latency`. Callback timing jitter therefore never reaches
//! the sound; only the constant `latency` does.

use crossbeam_channel::Receiver;
use ministep_core::voice::VoiceParams;

use crate::voice::Voice;

pub const MAX_VOICES: usize = 48;

#[derive(Debug, Clone, Copy)]
pub struct Hit {
    pub at: f64,
    pub params: VoiceParams,
    pub velocity: u8,
}

pub struct Mixer {
    pub sample_rate: f32,
    /// Fixed scheduling delay (seconds) added to every hit.
    pub latency: f64,
    pub master: f32,
    voices: Vec<Voice>,
    pending: Vec<(u64, Hit)>,
    /// Hits not yet placed on a frame (placed after the next clock sync).
    incoming: Vec<Hit>,
    frame: u64,
    /// Monotonic time of frame 0 (smoothed).
    origin: Option<f64>,
    syncs: u32,
    seed: u32,
    /// Hits that arrived after their frame had already been rendered.
    pub late_hits: u64,
}

impl Mixer {
    pub fn new(sample_rate: f32, latency: f64) -> Self {
        Self {
            sample_rate,
            latency,
            master: 0.9,
            voices: Vec::with_capacity(MAX_VOICES),
            pending: Vec::with_capacity(256),
            incoming: Vec::with_capacity(256),
            frame: 0,
            origin: None,
            syncs: 0,
            seed: 1,
            late_hits: 0,
        }
    }

    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }

    /// Update the frame↔time mapping from a callback that starts at `now`.
    fn sync_clock(&mut self, now: f64) {
        let measured = now - self.frame as f64 / f64::from(self.sample_rate);
        // Running mean at first (fast, unbiased), then a slow average that
        // only follows the drift between the audio and system clocks.
        self.syncs = self.syncs.saturating_add(1);
        let alpha = (1.0 / f64::from(self.syncs)).max(0.0005);
        self.origin = Some(match self.origin {
            // A jump (device restart, sleep) re-anchors immediately.
            Some(o) if (measured - o).abs() < 0.05 => o + alpha * (measured - o),
            _ => {
                self.syncs = 1;
                measured
            }
        });
    }

    /// Queue a hit; it is placed on a frame at the start of the next block.
    pub fn schedule(&mut self, hit: Hit) {
        if self.incoming.len() < self.incoming.capacity() {
            self.incoming.push(hit);
        }
    }

    fn place(&mut self, hit: Hit) {
        let origin = self.origin.unwrap_or(hit.at);
        let target = ((hit.at + self.latency - origin) * f64::from(self.sample_rate)).round();
        let frame = if target < self.frame as f64 {
            self.late_hits += 1;
            self.frame
        } else {
            target as u64
        };
        if self.pending.len() < self.pending.capacity() {
            self.pending.push((frame, hit));
        }
    }

    fn start(&mut self, hit: Hit) {
        if let Some(group) = hit.params.model.choke_group() {
            for v in &mut self.voices {
                if v.params.model.choke_group() == Some(group) {
                    v.choke();
                }
            }
        }
        if self.voices.len() >= MAX_VOICES {
            self.voices.remove(0);
        }
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.voices.push(Voice::new(hit.params, hit.velocity, self.sample_rate, self.seed));
    }

    /// Render one interleaved block that starts playing at monotonic `now`.
    pub fn process(&mut self, out: &mut [f32], channels: usize, now: f64, hits: Option<&Receiver<Hit>>) {
        self.sync_clock(now);
        if let Some(rx) = hits {
            while let Ok(hit) = rx.try_recv() {
                self.schedule(hit);
            }
        }
        for i in 0..self.incoming.len() {
            let hit = self.incoming[i];
            self.place(hit);
        }
        self.incoming.clear();
        let frames = out.len() / channels.max(1);
        self.pending.sort_unstable_by_key(|(f, _)| *f);
        let mut next = 0;
        for i in 0..frames {
            let f = self.frame + i as u64;
            while next < self.pending.len() && self.pending[next].0 <= f {
                let hit = self.pending[next].1;
                self.start(hit);
                next += 1;
            }
            let mut sum = 0.0;
            for v in &mut self.voices {
                sum += v.tick();
            }
            let s = (sum * self.master).tanh();
            for c in 0..channels {
                out[i * channels + c] = s;
            }
        }
        self.pending.drain(..next);
        self.voices.retain(|v| !v.is_done());
        self.frame += frames as u64;
    }
}
