//! Parameters for the built-in drum synth, one set per drum lane.
//!
//! Pure data: the DSP lives in `ministep-drumsynth`. All knobs are 0..1 so the
//! GUI, presets and files treat them uniformly; `tune` 0.5 is the model's
//! natural pitch.

use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VoiceModel {
    Kick,
    Snare,
    ClosedHat,
    OpenHat,
    Clap,
    Tom,
    Rim,
    Cowbell,
    Cymbal,
}

impl VoiceModel {
    pub const ALL: [VoiceModel; 9] = [
        Self::Kick,
        Self::Snare,
        Self::ClosedHat,
        Self::OpenHat,
        Self::Clap,
        Self::Tom,
        Self::Rim,
        Self::Cowbell,
        Self::Cymbal,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kick => "kick",
            Self::Snare => "snare",
            Self::ClosedHat => "closed_hat",
            Self::OpenHat => "open_hat",
            Self::Clap => "clap",
            Self::Tom => "tom",
            Self::Rim => "rim",
            Self::Cowbell => "cowbell",
            Self::Cymbal => "cymbal",
        }
    }

    /// Short label for chips.
    pub fn label(self) -> &'static str {
        match self {
            Self::Kick => "KICK",
            Self::Snare => "SNARE",
            Self::ClosedHat => "HAT",
            Self::OpenHat => "OPEN",
            Self::Clap => "CLAP",
            Self::Tom => "TOM",
            Self::Rim => "RIM",
            Self::Cowbell => "COW",
            Self::Cymbal => "CYM",
        }
    }

    /// Name of the fourth, model-specific knob.
    pub fn snap_label(self) -> &'static str {
        match self {
            Self::Kick => "PUNCH",
            Self::Snare | Self::Rim => "SNAP",
            Self::ClosedHat | Self::OpenHat | Self::Cymbal => "METAL",
            Self::Clap => "SPREAD",
            Self::Tom => "SWEEP",
            Self::Cowbell => "DETUNE",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_str() == s)
    }

    /// The General MIDI drum map, as Ableton's Drum Rack lays it out.
    pub fn for_note(note: u8) -> Self {
        match note {
            35 | 36 => Self::Kick,
            37 => Self::Rim,
            38 | 40 => Self::Snare,
            39 => Self::Clap,
            42 | 44 => Self::ClosedHat,
            46 => Self::OpenHat,
            41 | 43 | 45 | 47 | 48 | 50 => Self::Tom,
            56 => Self::Cowbell,
            49 | 51 | 52 | 53 | 55 | 57 | 59 => Self::Cymbal,
            _ => Self::Tom,
        }
    }

    /// Closed hats cut open hats, like on a drum machine.
    pub fn choke_group(self) -> Option<u8> {
        matches!(self, Self::ClosedHat | Self::OpenHat).then_some(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceParams {
    pub model: VoiceModel,
    pub tune: f32,
    pub decay: f32,
    pub tone: f32,
    pub snap: f32,
    pub level: f32,
}

impl VoiceParams {
    pub fn new(model: VoiceModel) -> Self {
        let (decay, tone, snap) = match model {
            VoiceModel::Kick => (0.45, 0.35, 0.5),
            VoiceModel::Snare => (0.35, 0.5, 0.55),
            VoiceModel::ClosedHat => (0.2, 0.6, 0.5),
            VoiceModel::OpenHat => (0.55, 0.6, 0.5),
            VoiceModel::Clap => (0.4, 0.5, 0.5),
            VoiceModel::Tom => (0.45, 0.4, 0.5),
            VoiceModel::Rim => (0.2, 0.6, 0.5),
            VoiceModel::Cowbell => (0.4, 0.5, 0.3),
            VoiceModel::Cymbal => (0.7, 0.6, 0.5),
        };
        Self { model, tune: 0.5, decay, tone, snap, level: 0.8 }
    }

    pub fn for_note(note: u8) -> Self {
        Self::new(VoiceModel::for_note(note))
    }

    /// Knob access by index: 0 tune, 1 decay, 2 tone, 3 snap, 4 level.
    pub const KNOBS: [&'static str; 5] = ["TUNE", "DECAY", "TONE", "", "LEVEL"];

    pub fn knob(&self, i: usize) -> f32 {
        [self.tune, self.decay, self.tone, self.snap, self.level][i]
    }

    pub fn set_knob(&mut self, i: usize, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match i {
            0 => self.tune = v,
            1 => self.decay = v,
            2 => self.tone = v,
            3 => self.snap = v,
            _ => self.level = v,
        }
    }

    pub fn knob_label(&self, i: usize) -> &'static str {
        if i == 3 { self.model.snap_label() } else { Self::KNOBS[i] }
    }

    /// Human-readable knob value for the GUI.
    pub fn knob_display(&self, i: usize) -> String {
        match i {
            0 => format!("{:+.1} st", (self.tune - 0.5) * 24.0),
            1 => format!("{:.0} ms", decay_seconds(self.model, self.decay) * 1000.0),
            _ => format!("{:.0} %", self.knob(i) * 100.0),
        }
    }

    fn round3(v: f32) -> f64 {
        (f64::from(v) * 1000.0).round() / 1000.0
    }

    pub fn to_json(&self) -> Value {
        json!({
            "model": self.model.as_str(),
            "tune": Self::round3(self.tune),
            "decay": Self::round3(self.decay),
            "tone": Self::round3(self.tone),
            "snap": Self::round3(self.snap),
            "level": Self::round3(self.level),
        })
    }

    /// Missing keys take the model's defaults; unknown models fall back to the note's.
    pub fn from_json(raw: &Value, note: u8) -> Self {
        let Some(obj) = raw.as_object() else { return Self::for_note(note) };
        let model = obj.get("model").and_then(Value::as_str).and_then(VoiceModel::parse).unwrap_or(VoiceModel::for_note(note));
        let mut p = Self::new(model);
        let get = |o: &Map<String, Value>, k: &str| o.get(k).and_then(Value::as_f64).map(|v| (v as f32).clamp(0.0, 1.0));
        if let Some(v) = get(obj, "tune") {
            p.tune = v;
        }
        if let Some(v) = get(obj, "decay") {
            p.decay = v;
        }
        if let Some(v) = get(obj, "tone") {
            p.tone = v;
        }
        if let Some(v) = get(obj, "snap") {
            p.snap = v;
        }
        if let Some(v) = get(obj, "level") {
            p.level = v;
        }
        p
    }
}

/// Amplitude decay time for a knob position (exponential, model-scaled).
pub fn decay_seconds(model: VoiceModel, decay: f32) -> f32 {
    let (lo, hi): (f32, f32) = match model {
        VoiceModel::Kick => (0.08, 1.6),
        VoiceModel::Snare => (0.05, 0.6),
        VoiceModel::ClosedHat => (0.015, 0.25),
        VoiceModel::OpenHat => (0.12, 1.2),
        VoiceModel::Clap => (0.06, 0.8),
        VoiceModel::Tom => (0.08, 1.2),
        VoiceModel::Rim => (0.01, 0.15),
        VoiceModel::Cowbell => (0.05, 0.8),
        VoiceModel::Cymbal => (0.3, 3.0),
    };
    lo * (hi / lo).powf(decay.clamp(0.0, 1.0))
}

/// Receives drum hits for the built-in synth (implemented by the audio engine).
pub trait DrumVoiceSink: Send + Sync {
    /// Play `voice` at monotonic time `at` (seconds, see [`crate::clock::monotonic`]).
    fn trigger(&self, at: f64, voice: VoiceParams, velocity: u8);
}

/// Where the drum track goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DrumRoute {
    /// Built-in synth only.
    Synth,
    /// MIDI notes only (the Python build's behaviour, and the core default;
    /// the app switches to the synth when no drum MIDI port is chosen).
    #[default]
    Midi,
    Both,
}

impl DrumRoute {
    pub fn synth(self) -> bool {
        matches!(self, Self::Synth | Self::Both)
    }
    pub fn midi(self) -> bool {
        matches!(self, Self::Midi | Self::Both)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Synth => "synth",
            Self::Midi => "midi",
            Self::Both => "both",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Synth, Self::Midi, Self::Both].into_iter().find(|r| r.as_str() == s)
    }
}
