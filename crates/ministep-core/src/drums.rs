//! Drum track model: fixed 16-step lanes that fire short MIDI notes.
//!
//! A drum hit has no pitch, tie, or gate; it is an on/off flag with a velocity.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::jsonx::{as_i64, get_i64, truthy};
use crate::notes::{note_name, parse_note_name};
use crate::voice::VoiceParams;

pub const DRUM_STEPS: usize = 16;
pub const DEFAULT_DRUM_VELOCITY: u8 = 100;
/// One-based, as shown to the user.
pub const DEFAULT_DRUM_CHANNEL: u8 = 10;
/// General MIDI drum map = Ableton Drum Rack default pads. C1 = 36.
pub const DEFAULT_DRUM_LANES: [(&str, u8); 4] =
    [("Kick", 36), ("Snare", 38), ("Closed Hat", 42), ("Open Hat", 46)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrumStep {
    pub active: bool,
    pub velocity: u8,
}

impl Default for DrumStep {
    fn default() -> Self {
        Self { active: false, velocity: DEFAULT_DRUM_VELOCITY }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrumLane {
    pub name: String,
    pub note: u8,
    pub steps: [DrumStep; DRUM_STEPS],
    /// Built-in synth voice. Saved only when it differs from the note's default.
    pub voice: VoiceParams,
}

impl DrumLane {
    pub fn new(name: impl Into<String>, note: u8) -> Self {
        let note = note.min(127);
        Self { name: name.into(), note, steps: [DrumStep::default(); DRUM_STEPS], voice: VoiceParams::for_note(note) }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrumTrack {
    pub lanes: Vec<DrumLane>,
    /// One-based MIDI channel.
    pub channel: u8,
    /// Edit cursor: UI state, not persisted.
    pub cursor_lane: usize,
    pub cursor_step: usize,
    /// Auditioned, not yet committed hits: lane index -> velocity.
    pub audition: BTreeMap<usize, u8>,
}

impl Default for DrumTrack {
    fn default() -> Self {
        Self::with_lanes(DEFAULT_DRUM_LANES.iter().map(|&(n, note)| DrumLane::new(n, note)).collect())
    }
}

impl DrumTrack {
    pub fn with_lanes(lanes: Vec<DrumLane>) -> Self {
        Self { lanes, channel: DEFAULT_DRUM_CHANNEL, cursor_lane: 0, cursor_step: 0, audition: BTreeMap::new() }
    }

    pub fn has_hits(&self) -> bool {
        self.lanes.iter().any(|l| l.steps.iter().any(|s| s.active))
    }

    /// Active `(lane, step)` pairs on grid position `index`.
    pub fn hits_at(&self, index: usize) -> impl Iterator<Item = (&DrumLane, &DrumStep)> {
        let pos = index % DRUM_STEPS;
        self.lanes.iter().filter(move |l| l.steps[pos].active).map(move |l| (l, &l.steps[pos]))
    }

    pub fn lane_for_note(&self, note: u8) -> Option<usize> {
        self.lanes.iter().position(|l| l.note == note)
    }

    /// Remember a played drum note for the next commit; `None` if no lane owns it.
    pub fn audition_note(&mut self, note: u8, velocity: u8) -> Option<usize> {
        let index = self.lane_for_note(note)?;
        self.audition.insert(index, velocity.clamp(1, 127));
        self.cursor_lane = index;
        Some(index)
    }

    /// Write every auditioned hit into the cursor step, then advance. Returns committed lane names.
    pub fn commit_audition(&mut self) -> Vec<String> {
        if self.audition.is_empty() {
            return Vec::new();
        }
        let mut names = Vec::new();
        for (&index, &velocity) in &self.audition {
            if let Some(lane) = self.lanes.get_mut(index) {
                lane.steps[self.cursor_step] = DrumStep { active: true, velocity };
                names.push(lane.name.clone());
            }
        }
        self.audition.clear();
        self.move_step(1);
        names
    }

    pub fn clear_audition(&mut self) {
        self.audition.clear();
    }

    /// Drum "rest": leave the cursor step, drop the audition, advance.
    pub fn skip_step(&mut self) {
        self.audition.clear();
        self.move_step(1);
    }

    pub fn cursor(&self) -> Option<&DrumStep> {
        self.lanes.get(self.cursor_lane).map(|l| &l.steps[self.cursor_step])
    }

    pub fn cursor_mut(&mut self) -> Option<&mut DrumStep> {
        let step = self.cursor_step;
        self.lanes.get_mut(self.cursor_lane).map(|l| &mut l.steps[step])
    }

    pub fn move_step(&mut self, delta: i64) {
        self.cursor_step = (self.cursor_step as i64 + delta).rem_euclid(DRUM_STEPS as i64) as usize;
    }

    pub fn move_lane(&mut self, delta: i64) {
        if !self.lanes.is_empty() {
            self.cursor_lane = (self.cursor_lane as i64 + delta).rem_euclid(self.lanes.len() as i64) as usize;
        }
    }

    /// Flip the hit under the cursor; return its new `active` value.
    pub fn toggle_hit(&mut self) -> bool {
        self.cursor_mut().map(|s| {
            s.active = !s.active;
            s.active
        }).unwrap_or(false)
    }

    pub fn clear_hit(&mut self) {
        if let Some(s) = self.cursor_mut() {
            s.active = false;
        }
    }

    pub fn adjust_velocity(&mut self, delta: i32) -> u8 {
        self.cursor_mut().map(|s| {
            s.velocity = (i32::from(s.velocity) + delta).clamp(1, 127) as u8;
            s.velocity
        }).unwrap_or(DEFAULT_DRUM_VELOCITY)
    }

    pub fn clear(&mut self) {
        for lane in &mut self.lanes {
            lane.steps = [DrumStep::default(); DRUM_STEPS];
        }
    }

    /// Musical content only: lanes, notes (as names), hits, velocities, channel.
    pub fn to_json(&self) -> Map<String, Value> {
        let lanes: Vec<Value> = self
            .lanes
            .iter()
            .map(|lane| {
                let mut v = json!({
                    "name": lane.name,
                    "note": note_name(lane.note),
                    "steps": lane.steps.iter().map(|s| json!({"active": s.active, "velocity": s.velocity})).collect::<Vec<_>>(),
                });
                // Optional: files from builds without the synth (and the Python
                // build) stay byte-identical when the voice is untouched.
                if lane.voice != VoiceParams::for_note(lane.note) {
                    v["synth"] = lane.voice.to_json();
                }
                v
            })
            .collect();
        let mut map = Map::new();
        map.insert("channel".into(), json!(self.channel));
        map.insert("lanes".into(), Value::Array(lanes));
        map
    }

    /// Notes may be names or numbers, short lanes are padded, missing velocity is 100.
    pub fn from_json(raw: &Value) -> Result<Self, String> {
        let obj = raw.as_object().ok_or("drums must be an object")?;
        let raw_lanes = match obj.get("lanes") {
            None => Vec::new(),
            Some(Value::Array(a)) => a.clone(),
            Some(_) => return Err("drum lanes must be a list".into()),
        };
        let mut lanes = Vec::new();
        for raw_lane in &raw_lanes {
            let lane_obj = raw_lane.as_object().filter(|o| o.contains_key("note")).ok_or("each drum lane needs a 'note'")?;
            let note = match &lane_obj["note"] {
                Value::String(s) => parse_note_name(s)?,
                other => {
                    let n = as_i64(other, "note")?;
                    if !(0..=127).contains(&n) {
                        return Err("MIDI note must be between 0 and 127".into());
                    }
                    n as u8
                }
            };
            let raw_steps = match lane_obj.get("steps") {
                None => Vec::new(),
                Some(Value::Array(a)) if a.len() <= DRUM_STEPS => a.clone(),
                Some(_) => return Err(format!("drum lane steps must be a list of at most {DRUM_STEPS}")),
            };
            let mut steps = [DrumStep::default(); DRUM_STEPS];
            for (slot, step) in steps.iter_mut().zip(&raw_steps) {
                if let Some(o) = step.as_object() {
                    let velocity = get_i64(o, "velocity", i64::from(DEFAULT_DRUM_VELOCITY))?;
                    if !(1..=127).contains(&velocity) {
                        return Err("drum velocity must be between 1 and 127".into());
                    }
                    *slot = DrumStep { active: o.get("active").is_some_and(truthy), velocity: velocity as u8 };
                }
            }
            let name = match lane_obj.get("name") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Null) | None => note_name(note),
                Some(other) => other.to_string(),
            };
            let voice = lane_obj.get("synth").map_or(VoiceParams::for_note(note), |v| VoiceParams::from_json(v, note));
            lanes.push(DrumLane { name, note, steps, voice });
        }
        let mut track = Self::with_lanes(lanes);
        track.channel = get_i64(obj, "channel", i64::from(DEFAULT_DRUM_CHANNEL))?.clamp(1, 16) as u8;
        Ok(track)
    }

    /// Take `other`'s lanes and channel; reset cursor and audition.
    pub fn replace_content(&mut self, other: DrumTrack) {
        self.lanes = other.lanes;
        self.channel = other.channel;
        self.cursor_lane = 0;
        self.cursor_step = 0;
        self.audition.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audition_commit_advances() {
        let mut t = DrumTrack::default();
        assert_eq!(t.audition_note(36, 110), Some(0));
        assert_eq!(t.audition_note(42, 0), Some(2));
        assert_eq!(t.audition_note(61, 90), None);
        assert_eq!(t.commit_audition(), vec!["Kick".to_string(), "Closed Hat".to_string()]);
        assert_eq!(t.lanes[0].steps[0], DrumStep { active: true, velocity: 110 });
        assert_eq!(t.lanes[2].steps[0], DrumStep { active: true, velocity: 1 });
        assert_eq!(t.cursor_step, 1);
        assert!(t.commit_audition().is_empty());
        t.skip_step();
        assert_eq!(t.cursor_step, 2);
        t.move_step(-3);
        assert_eq!(t.cursor_step, 15);
    }

    #[test]
    fn json_round_trip_and_padding() {
        let mut t = DrumTrack::default();
        t.lanes[1].steps[4] = DrumStep { active: true, velocity: 120 };
        t.channel = 3;
        let back = DrumTrack::from_json(&Value::Object(t.to_json())).unwrap();
        assert_eq!(back.lanes, t.lanes);
        assert_eq!(back.channel, 3);
        let raw = json!({"lanes": [{"note": 36, "steps": [{"active": true}]}]});
        let short = DrumTrack::from_json(&raw).unwrap();
        assert_eq!(short.lanes[0].name, "C1");
        assert!(short.lanes[0].steps[0].active);
        assert_eq!(short.lanes[0].steps[0].velocity, 100);
        assert!(!short.lanes[0].steps[15].active);
        assert_eq!(short.channel, 10);
        assert!(DrumTrack::from_json(&json!({"lanes": [{"name": "x"}]})).is_err());
        assert!(DrumTrack::from_json(&json!([])).is_err());
    }
}
