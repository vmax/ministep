//! Sequence data and application state, independent from MIDI and the UI.

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::drums::DrumTrack;
use crate::jsonx::{as_i64, get_f64, get_i64, truthy};
use crate::notes::note_name;
use crate::patterns::{PatternError, read_json, write_json_atomic};

/// One sequencer step. `note == None` is a rest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    pub note: Option<u8>,
    pub velocity: u8,
    pub gate: f64,
    pub enabled: bool,
    pub tie: bool,
    pub accent: bool,
}

impl Step {
    pub fn new(note: Option<u8>, velocity: u8, gate: f64) -> Self {
        Self { note, velocity, gate, enabled: true, tie: false, accent: false }
    }

    fn to_json(self) -> Value {
        json!({
            "note": self.note,
            "velocity": self.velocity,
            "gate": self.gate,
            "enabled": self.enabled,
            "tie": self.tie,
            "accent": self.accent,
        })
    }

    fn from_json(raw: &Value) -> Result<Self, String> {
        let obj = raw.as_object().ok_or("each step must be an object")?;
        let note = match obj.get("note") {
            None => return Err("step is missing 'note'".into()),
            Some(Value::Null) => None,
            Some(v) => {
                let n = as_i64(v, "note")?;
                if !(0..=127).contains(&n) {
                    return Err("MIDI note must be between 0 and 127".into());
                }
                Some(n as u8)
            }
        };
        let velocity = get_i64(obj, "velocity", 100)?;
        if !(0..=127).contains(&velocity) {
            return Err("MIDI velocity must be between 0 and 127".into());
        }
        let gate = get_f64(obj, "gate", 0.5)?;
        if !(0.0..=1.0).contains(&gate) {
            return Err("gate must be between 0.0 and 1.0".into());
        }
        let flag = |k: &str, d: bool| obj.get(k).map_or(d, truthy);
        Ok(Self {
            note,
            velocity: velocity as u8,
            gate,
            enabled: flag("enabled", true),
            tie: flag("tie", false),
            accent: flag("accent", false),
        })
    }
}

pub const LOOP_LENGTHS: [Option<usize>; 5] = [None, Some(8), Some(16), Some(32), Some(64)];
pub const PLAYBACK_DIVISIONS: [u32; 4] = [4, 8, 16, 32];
const VALID_DIVISIONS: [u32; 7] = [1, 2, 4, 8, 16, 32, 64];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Track {
    #[default]
    Melodic,
    Drums,
}

impl Track {
    pub fn as_str(self) -> &'static str {
        match self {
            Track::Melodic => "melodic",
            Track::Drums => "drums",
        }
    }
}

pub const FORMAT_VERSION: i64 = 2;
pub const TRACK_FORMAT_VERSION: i64 = 1;

/// Mutable state owned by the engine.
#[derive(Debug, Clone)]
pub struct AppState {
    pub bpm: f64,
    pub step_division: u32,
    pub sequence: Vec<Step>,
    /// `None` loops through the complete sequence.
    pub loop_length: Option<usize>,
    pub playhead: usize,
    /// Drum position: its own 16-step counter. Runtime only.
    pub drum_playhead: usize,
    pub playing: bool,
    pub recording: bool,
    pub last_note_played: Option<(u8, u8)>,
    pub held_notes: HashMap<u8, u32>,
    pub output_channel: u8,
    pub default_gate: f64,
    pub default_velocity: u8,
    pub transpose: i32,
    pub octave: i32,
    pub selected_input: Option<String>,
    pub selected_output: Option<String>,
    pub selected_drum_output: Option<String>,
    pub cursor: usize,
    pub root_capture_armed: bool,
    pub status_message: String,
    pub drums: DrumTrack,
    pub active_track: Track,
    /// Where drum hits go: built-in synth, MIDI, or both. Session routing, not saved.
    pub drum_route: crate::voice::DrumRoute,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            bpm: 128.0,
            step_division: 16,
            sequence: Vec::new(),
            loop_length: None,
            playhead: 0,
            drum_playhead: 0,
            playing: false,
            recording: false,
            last_note_played: None,
            held_notes: HashMap::new(),
            output_channel: 1,
            default_gate: 0.5,
            default_velocity: 100,
            transpose: 0,
            octave: 0,
            selected_input: None,
            selected_output: None,
            selected_drum_output: None,
            cursor: 0,
            root_capture_armed: false,
            status_message: "Ready: audition a note, then commit it.".into(),
            drums: DrumTrack::default(),
            active_track: Track::Melodic,
            drum_route: crate::voice::DrumRoute::default(),
        }
    }
}

fn clamp_note(n: i32) -> u8 {
    n.clamp(0, 127) as u8
}

impl AppState {
    pub fn set_bpm(&mut self, bpm: f64) {
        self.bpm = if bpm.is_nan() { self.bpm } else { bpm.clamp(1.0, 999.0) };
    }

    pub fn set_step_division(&mut self, division: u32) -> Result<(), String> {
        if !VALID_DIVISIONS.contains(&division) {
            return Err("step_division must be a power-of-two note division".into());
        }
        self.step_division = division;
        Ok(())
    }

    pub fn set_loop_length(&mut self, length: Option<usize>) -> Result<(), String> {
        if !LOOP_LENGTHS.contains(&length) {
            return Err("loop_length must be None, 8, 16, 32, or 64".into());
        }
        self.loop_length = length;
        Ok(())
    }

    pub fn cycle_loop_length(&mut self, delta: i64) {
        let i = LOOP_LENGTHS.iter().position(|&l| l == self.loop_length).unwrap_or(0) as i64;
        self.loop_length = LOOP_LENGTHS[(i + delta).rem_euclid(LOOP_LENGTHS.len() as i64) as usize];
    }

    pub fn cycle_step_division(&mut self, direction: i64) {
        let i = PLAYBACK_DIVISIONS.iter().position(|&d| d == self.step_division).unwrap_or(2) as i64;
        let division = PLAYBACK_DIVISIONS[(i + direction).rem_euclid(PLAYBACK_DIVISIONS.len() as i64) as usize];
        self.step_division = division;
        self.status_message = format!("Step division set to 1/{division}.");
    }

    /// Active loop span, never longer than the sequence itself.
    pub fn playback_length(&self) -> usize {
        match self.loop_length {
            None => self.sequence.len(),
            Some(l) => l.min(self.sequence.len()),
        }
    }

    /// Global gate; existing steps follow it.
    pub fn set_default_gate(&mut self, gate: f64, apply_existing: bool) {
        self.default_gate = gate.clamp(0.0, 1.0);
        if apply_existing {
            for step in &mut self.sequence {
                step.gate = self.default_gate;
            }
        }
    }

    pub fn append_note(&mut self, note: u8, velocity: Option<u8>) -> Step {
        let step = Step::new(Some(note.min(127)), velocity.unwrap_or(self.default_velocity).min(127), self.default_gate);
        self.sequence.push(step);
        self.cursor = self.sequence.len() - 1;
        step
    }

    pub fn commit_last_note(&mut self) -> bool {
        let Some((note, velocity)) = self.last_note_played else {
            self.status_message = "Nothing to commit: audition a MIDI note first.".into();
            return false;
        };
        self.append_note(note, Some(velocity));
        self.status_message = format!("Committed {} as step {}.", note_name(note), self.sequence.len());
        true
    }

    pub fn append_rest(&mut self) -> Step {
        let step = Step::new(None, self.default_velocity, self.default_gate);
        self.sequence.push(step);
        self.cursor = self.sequence.len() - 1;
        self.status_message = format!("Added rest as step {}.", self.sequence.len());
        step
    }

    /// Extend the preceding pitched step by one full grid step (tie + gate 1.0 copy).
    pub fn append_hold(&mut self) -> Option<Step> {
        let Some(previous) = self.sequence.last_mut().filter(|s| s.note.is_some()) else {
            self.status_message = "Hold needs a note immediately before it.".into();
            return None;
        };
        previous.tie = true;
        let step = Step { tie: false, gate: 1.0, ..*previous };
        self.sequence.push(step);
        self.cursor = self.sequence.len() - 1;
        self.status_message = format!("Held {} for one more step.", note_name(step.note.unwrap_or(0)));
        Some(step)
    }

    pub fn undo(&mut self) -> Option<Step> {
        let Some(step) = self.sequence.pop() else {
            self.status_message = "Nothing to undo.".into();
            return None;
        };
        if let (Some(previous), Some(_)) = (self.sequence.last_mut(), step.note)
            && previous.note == step.note && previous.tie {
                previous.tie = false;
            }
        self.clamp_positions();
        self.status_message = "Removed last step.".into();
        Some(step)
    }

    fn clamp_positions(&mut self) {
        let last = self.sequence.len().saturating_sub(1);
        self.cursor = self.cursor.min(last);
        self.playhead = self.playhead.min(last);
    }

    pub fn clear(&mut self) {
        self.sequence.clear();
        self.cursor = 0;
        self.playhead = 0;
        self.status_message = "Sequence cleared.".into();
    }

    pub fn replace_cursor_with_last_note(&mut self) -> bool {
        if self.sequence.is_empty() {
            self.status_message = "There is no step to replace.".into();
            return false;
        }
        let Some((note, velocity)) = self.last_note_played else {
            self.status_message = "Audition a MIDI note before replacing a step.".into();
            return false;
        };
        let old = &mut self.sequence[self.cursor];
        old.note = Some(note);
        old.velocity = velocity;
        self.status_message = format!("Replaced step {} with {}.", self.cursor + 1, note_name(note));
        true
    }

    pub fn delete_cursor(&mut self) -> Option<Step> {
        if self.sequence.is_empty() {
            self.status_message = "There is no step to delete.".into();
            return None;
        }
        let step = self.sequence.remove(self.cursor);
        self.clamp_positions();
        self.status_message = "Deleted selected step.".into();
        Some(step)
    }

    pub fn move_cursor(&mut self, delta: i64) {
        if !self.sequence.is_empty() {
            self.cursor = (self.cursor as i64 + delta).rem_euclid(self.sequence.len() as i64) as usize;
        }
    }

    pub fn adjust_cursor_note(&mut self, semitones: i32) -> bool {
        if self.sequence.is_empty() {
            self.status_message = "There is no step to edit.".into();
            return false;
        }
        let cursor = self.cursor;
        let step = &mut self.sequence[cursor];
        let Some(note) = step.note else {
            self.status_message = "A rest has no pitch to change. Use E to replace it.".into();
            return false;
        };
        let new = clamp_note(i32::from(note) + semitones);
        step.note = Some(new);
        self.status_message = format!("Step {}: {}.", cursor + 1, note_name(new));
        true
    }

    /// The first pitched step is the sequence root reference.
    pub fn sequence_root(&self) -> Option<u8> {
        self.sequence.iter().find_map(|s| s.note)
    }

    /// Destructively move pitched steps so the first has the target's pitch class,
    /// by the nearest signed interval; bakes in and clears live transpose.
    pub fn set_root_from_note(&mut self, target_note: u8) -> bool {
        let Some(root) = self.sequence_root() else {
            self.status_message = "Cannot set root: the sequence has no notes.".into();
            return false;
        };
        let audible_root = (i32::from(root) + self.transpose).clamp(0, 127);
        let mut semitones = (i32::from(target_note) % 12 - audible_root % 12).rem_euclid(12);
        if semitones > 6 {
            semitones -= 12;
        }
        let total_shift = self.transpose + semitones;
        for step in &mut self.sequence {
            if let Some(n) = step.note {
                step.note = Some(clamp_note(i32::from(n) + total_shift));
            }
        }
        self.transpose = 0;
        self.status_message = format!("Root changed to {} ({:+} semitones).", note_name(target_note), total_shift);
        true
    }

    pub fn toggle_track(&mut self) -> Track {
        self.active_track = match self.active_track {
            Track::Melodic => Track::Drums,
            Track::Drums => Track::Melodic,
        };
        self.status_message = format!("Editing {} track.", self.active_track.as_str());
        self.active_track
    }

    pub fn note_held(&mut self, note: u8, velocity: u8) {
        *self.held_notes.entry(note).or_insert(0) += 1;
        self.last_note_played = Some((note, velocity));
        self.status_message = format!("Auditioning {} (velocity {velocity}).", note_name(note));
    }

    pub fn note_released(&mut self, note: u8) {
        match self.held_notes.get(&note).copied() {
            Some(c) if c > 1 => {
                self.held_notes.insert(note, c - 1);
            }
            _ => {
                self.held_notes.remove(&note);
            }
        }
    }

    /// Note as it will sound during playback (transpose + octave applied).
    pub fn playback_note(&self, step: &Step) -> Option<u8> {
        if !step.enabled {
            return None;
        }
        step.note.map(|n| clamp_note(i32::from(n) + self.transpose + self.octave * 12))
    }

    // --- persistence --------------------------------------------------------

    /// The melodic track's musical state: no transport, ports, or UI positions.
    pub fn melodic_to_json(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("loop_length".into(), json!(self.loop_length));
        m.insert("default_gate".into(), json!(self.default_gate));
        m.insert("default_velocity".into(), json!(self.default_velocity));
        m.insert("transpose".into(), json!(self.transpose));
        m.insert("octave".into(), json!(self.octave));
        m.insert("output_channel".into(), json!(self.output_channel));
        m.insert("steps".into(), Value::Array(self.sequence.iter().map(|s| s.to_json()).collect()));
        m
    }

    /// Both tracks plus tempo (snapshot format v2).
    pub fn to_json(&self, name: Option<&str>) -> Value {
        let mut m = Map::new();
        m.insert("version".into(), json!(FORMAT_VERSION));
        if let Some(name) = name {
            m.insert("name".into(), json!(name));
        }
        m.insert("bpm".into(), json!(self.bpm));
        m.insert("step_division".into(), json!(self.step_division));
        m.insert("melodic".into(), Value::Object(self.melodic_to_json()));
        m.insert("drums".into(), Value::Object(self.drums.to_json()));
        Value::Object(m)
    }

    /// Build a state from snapshot JSON (version 1 or 2).
    pub fn from_json(data: &Value) -> Result<Self, String> {
        let obj = data.as_object().ok_or("Pattern file must contain a JSON object")?;
        let version = obj.get("version").map_or(Ok(1), |v| as_i64(v, "version"));
        let (melodic, drums) = match version {
            Ok(1) => (data.clone(), None),
            Ok(FORMAT_VERSION) => (obj.get("melodic").cloned().unwrap_or_else(|| json!({})), obj.get("drums")),
            _ => {
                return Err(format!(
                    "Unsupported pattern version {} (this MiniStep reads versions 1 and {FORMAT_VERSION})",
                    obj.get("version").map_or("None".to_string(), Value::to_string)
                ));
            }
        };
        let build = || -> Result<Self, String> {
            let mut state = Self::default();
            state.set_bpm(get_f64(obj, "bpm", 128.0)?);
            state.set_step_division(u32::try_from(get_i64(obj, "step_division", 16)?).map_err(|e| e.to_string())?)?;
            state.apply_melodic(&melodic)?;
            if let Some(d) = drums.filter(|d| !d.is_null()) {
                state.drums = DrumTrack::from_json(d)?;
            }
            Ok(state)
        };
        build().map_err(|e| format!("Invalid pattern data: {e}"))
    }

    /// Replace the melodic track from a section; parsed fully before anything changes.
    pub fn apply_melodic(&mut self, section: &Value) -> Result<(), String> {
        let obj = section.as_object().ok_or("melodic pattern must be an object")?;
        let loop_length = match obj.get("loop_length") {
            None | Some(Value::Null) => None,
            Some(v) => Some(as_i64(v, "loop_length")?),
        };
        let loop_length = match loop_length {
            None => None,
            Some(l) if [8, 16, 32, 64].contains(&l) => Some(l as usize),
            Some(_) => return Err("loop_length must be None, 8, 16, 32, or 64".into()),
        };
        let default_gate = get_f64(obj, "default_gate", 0.5)?;
        let default_velocity = get_i64(obj, "default_velocity", 100)?;
        let transpose = get_i64(obj, "transpose", 0)?;
        let octave = get_i64(obj, "octave", 0)?;
        let output_channel = get_i64(obj, "output_channel", 1)?;
        let steps = match obj.get("steps") {
            None => Vec::new(),
            Some(Value::Array(a)) => a.iter().map(Step::from_json).collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err("steps must be a list".into()),
        };
        self.sequence = steps;
        self.loop_length = loop_length;
        self.default_gate = default_gate;
        self.default_velocity = default_velocity.clamp(0, 127) as u8;
        self.transpose = transpose.clamp(-127, 127) as i32;
        self.octave = octave.clamp(-8, 8) as i32;
        self.output_channel = output_channel.clamp(1, 16) as u8;
        self.cursor = 0;
        self.playhead = 0;
        Ok(())
    }

    pub fn apply_drums(&mut self, section: &Value) -> Result<(), String> {
        let track = DrumTrack::from_json(section)?;
        self.drums.replace_content(track);
        Ok(())
    }

    /// Replace musical content with `loaded`; ports and active track survive.
    pub fn adopt(&mut self, loaded: AppState) {
        let ports = (self.selected_input.take(), self.selected_output.take(), self.selected_drum_output.take());
        let active = self.active_track;
        let route = self.drum_route;
        *self = loaded;
        self.drum_route = route;
        (self.selected_input, self.selected_output, self.selected_drum_output) = ports;
        self.active_track = active;
    }

    pub fn save(&mut self, path: &Path) -> Result<(), PatternError> {
        write_json_atomic(path, &self.to_json(None))?;
        self.status_message = format!("Saved {} steps to {}.", self.sequence.len(), path.display());
        Ok(())
    }

    pub fn load(&mut self, path: &Path) -> Result<(), PatternError> {
        let loaded = Self::from_json(&read_json(path)?).map_err(PatternError)?;
        self.adopt(loaded);
        self.status_message = format!("Loaded {} steps from {}.", self.sequence.len(), path.display());
        Ok(())
    }
}

