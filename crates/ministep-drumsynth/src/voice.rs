//! The drum voices. Each is a tiny analogue-style model in the spirit of the
//! classic drum machines: no samples, every hit computed from its knobs.

use ministep_core::voice::{VoiceModel, VoiceParams, decay_seconds};

use crate::dsp::{Decay, Noise, Phase, Svf, tune_ratio};

/// 808 hi-hat oscillator frequencies (Hz).
const METAL: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];

pub struct Voice {
    pub params: VoiceParams,
    gain: f32,
    sr: f32,
    t: u32,
    amp: Decay,
    pitch_env: Decay,
    noise_env: Decay,
    phases: [Phase; 6],
    noise: Noise,
    f1: Svf,
    f2: Svf,
    /// Fast fade-out after a choke (1.0 = not choking).
    choke: f32,
    choking: bool,
    done: bool,
}

impl Voice {
    pub fn new(params: VoiceParams, velocity: u8, sr: f32, seed: u32) -> Self {
        let p = params;
        let vel = f32::from(velocity.clamp(1, 127)) / 127.0;
        let gain = p.level * p.level * (0.35 + 0.65 * vel);
        let decay = decay_seconds(p.model, p.decay);
        let (amp, pitch_env, noise_env, f1, f2) = match p.model {
            VoiceModel::Kick => (
                Decay::new(decay, sr),
                Decay::new(0.012 + 0.05 * (1.0 - p.snap), sr),
                Decay::new(0.004, sr),
                Svf::new(3000.0, 0.7, sr),
                Svf::new(20.0, 0.7, sr),
            ),
            VoiceModel::Snare => (
                Decay::new(decay * 0.6, sr),
                Decay::new(0.02, sr),
                Decay::new(decay, sr),
                Svf::new(1500.0 + 6500.0 * p.tone, 0.8, sr),
                Svf::new(7000.0, 0.6, sr),
            ),
            VoiceModel::ClosedHat | VoiceModel::OpenHat | VoiceModel::Cymbal => {
                let centre = if p.model == VoiceModel::Cymbal { 5000.0 } else { 7500.0 };
                (
                    Decay::new(decay, sr),
                    Decay::new(1.0, sr),
                    Decay::new(decay, sr),
                    Svf::new(centre * (0.6 + 0.8 * p.tone), 1.2, sr),
                    Svf::new(4000.0 + 4000.0 * p.tone, 0.7, sr),
                )
            }
            VoiceModel::Clap => (
                Decay::new(decay, sr),
                Decay::new(1.0, sr),
                Decay::new(0.008, sr),
                Svf::new(700.0 + 1600.0 * p.tone, 2.2, sr),
                Svf::new(400.0, 0.7, sr),
            ),
            VoiceModel::Tom => (
                Decay::new(decay, sr),
                Decay::new(0.03 + 0.25 * p.snap, sr),
                Decay::new(0.01, sr),
                Svf::new(600.0 + 4000.0 * p.tone, 0.7, sr),
                Svf::new(20.0, 0.7, sr),
            ),
            VoiceModel::Rim => (
                Decay::new(decay, sr),
                Decay::new(0.01, sr),
                Decay::new(0.003, sr),
                Svf::new(400.0, 0.7, sr),
                Svf::new(2500.0 + 3000.0 * p.tone, 1.5, sr),
            ),
            VoiceModel::Cowbell => (
                Decay::new(decay, sr),
                Decay::new(0.02, sr),
                Decay::new(1.0, sr),
                Svf::new(1200.0 + 2500.0 * p.tone, 1.6, sr),
                Svf::new(300.0, 0.7, sr),
            ),
        };
        Self {
            params,
            gain,
            sr,
            t: 0,
            amp,
            pitch_env,
            noise_env,
            phases: [Phase::default(); 6],
            noise: Noise::new(seed),
            f1,
            f2,
            choke: 1.0,
            choking: false,
            done: false,
        }
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Fade out over ~5 ms (hat choke, voice stealing).
    pub fn choke(&mut self) {
        self.choking = true;
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        if self.done {
            return 0.0;
        }
        let p = self.params;
        let sr = self.sr;
        let tune = tune_ratio(p.tune);
        let a = self.amp.tick();
        let x = match p.model {
            VoiceModel::Kick => {
                let base = 48.0 * tune;
                let sweep = 1.0 + (2.0 + 7.0 * p.snap) * self.pitch_env.tick();
                let body = self.phases[0].sine(base * sweep, sr);
                let click = self.noise.tick() * self.noise_env.tick() * p.snap;
                let click = self.f1.process(click).low;
                let drive = 1.0 + 5.0 * p.tone;
                ((body * a + click * 0.6) * drive).tanh() / drive.tanh().max(0.5)
            }
            VoiceModel::Snare => {
                let pe = 1.0 + 0.5 * self.pitch_env.tick();
                let body = 0.6 * self.phases[0].sine(185.0 * tune * pe, sr) + 0.4 * self.phases[1].sine(330.0 * tune * pe, sr);
                let n = self.noise.tick();
                let n = self.f1.process(n).high;
                let ne = self.noise_env.tick();
                let snap = p.snap;
                body * a * (1.0 - 0.6 * snap) + n * ne * (0.3 + 0.9 * snap)
            }
            VoiceModel::ClosedHat | VoiceModel::OpenHat | VoiceModel::Cymbal => {
                let mut metal = 0.0;
                for (i, f) in METAL.iter().enumerate() {
                    metal += self.phases[i].square(f * tune * if p.model == VoiceModel::Cymbal { 0.8 } else { 1.0 }, sr);
                }
                let mix = metal / 6.0 * p.snap + self.noise.tick() * (1.0 - p.snap) * 0.8;
                let bp = self.f1.process(mix).band;
                let hp = self.f2.process(bp).high;
                hp * a * 2.2
            }
            VoiceModel::Clap => {
                // Three quick bursts, then the tail.
                let spacing = (0.006 + 0.012 * p.snap) * sr;
                let tf = self.t as f32;
                let burst = if tf < spacing * 3.0 {
                    let local = tf % spacing;
                    (-local / (0.0025 * sr)).exp()
                } else {
                    a
                };
                let n = self.f1.process(self.noise.tick()).band;
                let n = self.f2.process(n).high;
                n * burst * 2.5
            }
            VoiceModel::Tom => {
                let base = 110.0 * tune;
                let sweep = 1.0 + 0.8 * self.pitch_env.tick();
                let body = self.phases[0].sine(base * sweep, sr);
                let n = self.f1.process(self.noise.tick()).low * self.noise_env.tick() * 0.3;
                body * a + n
            }
            VoiceModel::Rim => {
                let body = 0.6 * self.phases[0].sine(1700.0 * tune, sr) + 0.5 * self.phases[1].sine(480.0 * tune, sr);
                let click = self.noise.tick() * self.noise_env.tick() * p.snap;
                let x = self.f2.process(body * a + click).band;
                self.f1.process(x).high * 2.0
            }
            VoiceModel::Cowbell => {
                let detune = 1.0 + 0.08 * (p.snap - 0.3);
                let s = 0.5 * self.phases[0].square(540.0 * tune, sr) + 0.5 * self.phases[1].square(800.0 * tune * detune, sr);
                let bp = self.f1.process(s).band;
                let hp = self.f2.process(bp).high;
                // Two-stage: a sharp attack decay into the set tail.
                let attack = self.pitch_env.tick();
                hp * (0.6 * a + 0.4 * attack) * 1.6
            }
        };
        self.t = self.t.wrapping_add(1);
        if self.choking {
            self.choke *= (-1.0 / (0.005 * sr)).exp();
        }
        if a < 1e-4 || self.choke < 1e-4 {
            self.done = true;
        }
        x * self.gain * self.choke
    }
}

/// Render one hit offline (tests, GUI waveform preview).
pub fn render(params: VoiceParams, velocity: u8, sr: f32, max_seconds: f32) -> Vec<f32> {
    let mut v = Voice::new(params, velocity, sr, 0x9E37_79B9);
    let n = (max_seconds * sr) as usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        if v.is_done() {
            break;
        }
        out.push(v.tick());
    }
    out
}
