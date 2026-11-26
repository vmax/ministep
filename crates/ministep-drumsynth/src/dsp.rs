//! Small DSP building blocks. Everything is per-sample and allocation-free.

use std::f32::consts::{PI, TAU};

/// xorshift32 white noise in -1..1.
#[derive(Clone, Copy)]
pub struct Noise(u32);

impl Noise {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// Topology-preserving state-variable filter (Simper/Cytomic).
#[derive(Clone, Copy, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

pub struct SvfOut {
    pub low: f32,
    pub band: f32,
    pub high: f32,
}

impl Svf {
    pub fn new(cutoff: f32, q: f32, sr: f32) -> Self {
        let mut f = Self::default();
        f.set(cutoff, q, sr);
        f
    }

    pub fn set(&mut self, cutoff: f32, q: f32, sr: f32) {
        let g = (PI * cutoff.clamp(10.0, sr * 0.49) / sr).tan();
        self.k = 1.0 / q.max(0.05);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> SvfOut {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        SvfOut { low: v2, band: v1, high: x - self.k * v1 - v2 }
    }
}

/// Exponential decay: multiplies by a constant each sample (-60 dB at `seconds`).
#[derive(Clone, Copy)]
pub struct Decay {
    pub value: f32,
    coef: f32,
}

impl Decay {
    pub fn new(seconds: f32, sr: f32) -> Self {
        Self { value: 1.0, coef: (-6.907_755 / (seconds.max(0.001) * sr)).exp() }
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        let v = self.value;
        self.value *= self.coef;
        v
    }
}

/// Naive phase accumulator; band-limiting is unnecessary for these short,
/// heavily filtered metallic tones.
#[derive(Clone, Copy, Default)]
pub struct Phase(pub f32);

impl Phase {
    #[inline]
    pub fn step(&mut self, freq: f32, sr: f32) -> f32 {
        let p = self.0;
        self.0 = (self.0 + freq / sr).fract();
        p
    }

    #[inline]
    pub fn sine(&mut self, freq: f32, sr: f32) -> f32 {
        (self.step(freq, sr) * TAU).sin()
    }

    #[inline]
    pub fn square(&mut self, freq: f32, sr: f32) -> f32 {
        if self.step(freq, sr) < 0.5 { 1.0 } else { -1.0 }
    }
}

/// `tune` 0..1 → frequency ratio, ±1 octave.
pub fn tune_ratio(tune: f32) -> f32 {
    2f32.powf((tune - 0.5) * 2.0)
}
