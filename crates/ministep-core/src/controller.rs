//! Abstract commands and learned controller mappings.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::message::MidiMessage;
use crate::patterns::default_config_dir;

macro_rules! commands {
    ($($name:ident),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        pub enum CommandName { $($name),* }

        impl CommandName {
            pub const ALL: &'static [CommandName] = &[$(CommandName::$name),*];
            pub fn as_str(self) -> &'static str {
                match self { $(CommandName::$name => stringify!($name)),* }
            }
            pub fn parse(s: &str) -> Option<Self> {
                match s { $(stringify!($name) => Some(CommandName::$name),)* _ => None }
            }
        }
    };
}

commands!(
    COMMIT, HOLD, RECORD, RECORD_TOGGLE, REST, UNDO, CLEAR, PLAY, STOP, PLAY_STOP, RESTART,
    BPM_UP, BPM_DOWN, STEP_DIVISION_UP, STEP_DIVISION_DOWN, SET_BPM, SET_GATE, OCTAVE_UP,
    OCTAVE_DOWN, TRANSPOSE_UP, TRANSPOSE_DOWN, ARM_ROOT_CAPTURE, SET_TRANSPOSE, SET_OCTAVE,
);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Command {
    pub name: CommandName,
    pub value: Option<f64>,
}

impl Command {
    pub fn new(name: CommandName) -> Self {
        Self { name, value: None }
    }
    pub fn with(name: CommandName, value: f64) -> Self {
        Self { name, value: Some(value) }
    }
}

/// Commands that need a continuous control: (min, max, integer).
pub fn absolute_range(name: CommandName) -> Option<(f64, f64, bool)> {
    match name {
        CommandName::SET_BPM => Some((40.0, 240.0, true)),
        CommandName::SET_GATE => Some((0.05, 1.0, false)),
        CommandName::SET_TRANSPOSE => Some((-24.0, 24.0, true)),
        CommandName::SET_OCTAVE => Some((-3.0, 3.0, true)),
        _ => None,
    }
}

pub fn default_mapping_path() -> PathBuf {
    default_config_dir().join("minilab3-mapping.json")
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AbsoluteCc {
    pub control: u8,
    pub command: CommandName,
    pub minimum: f64,
    pub maximum: f64,
    pub integer: bool,
}

impl AbsoluteCc {
    pub fn translate(&self, value: u8) -> Command {
        let scaled = self.minimum + (self.maximum - self.minimum) * (f64::from(value) / 127.0);
        Command::with(self.command, if self.integer { scaled.round_ties_even() } else { scaled })
    }
}

/// Editable mapping for pad notes and CC controls (MIDI Learn).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mapping {
    pub note_commands: BTreeMap<(u8, u8), CommandName>,
    pub cc_commands: BTreeMap<u8, CommandName>,
    pub absolute_cc: Vec<AbsoluteCc>,
}

impl Mapping {
    pub fn command_for(&self, message: &MidiMessage) -> Option<Command> {
        match *message {
            MidiMessage::NoteOn { channel, note, velocity } if velocity > 0 => {
                self.note_commands.get(&(channel, note)).map(|&c| Command::new(c))
            }
            MidiMessage::ControlChange { control, value, .. } => {
                if let Some(&c) = self.cc_commands.get(&control) {
                    // Pads send 127 on press and 0 on release: fire once.
                    return (value > 0).then(|| Command::new(c));
                }
                self.absolute_cc.iter().find(|r| r.control == control).map(|r| r.translate(value))
            }
            _ => None,
        }
    }

    pub fn learn_cc(&mut self, control: u8, command: CommandName) {
        self.cc_commands.remove(&control);
        self.absolute_cc.retain(|r| r.control != control);
        if let Some((minimum, maximum, integer)) = absolute_range(command) {
            self.absolute_cc.push(AbsoluteCc { control, command, minimum, maximum, integer });
        } else {
            self.cc_commands.insert(control, command);
        }
    }

    pub fn learn_note(&mut self, channel: u8, note: u8, command: CommandName) -> Result<(), String> {
        if absolute_range(command).is_some() {
            return Err(format!("{} needs a continuous CC control", command.as_str()));
        }
        self.note_commands.insert((channel, note), command);
        Ok(())
    }

    pub fn to_json(&self) -> Value {
        json!({
            "version": 1,
            "cc_commands": self.cc_commands.iter().map(|(c, n)| json!({"control": c, "command": n.as_str()})).collect::<Vec<_>>(),
            "note_commands": self.note_commands.iter().map(|((ch, note), n)| json!({"channel": ch, "note": note, "command": n.as_str()})).collect::<Vec<_>>(),
            "absolute_cc": self.absolute_cc.iter().map(|r| json!({
                "control": r.control, "command": r.command.as_str(),
                "minimum": r.minimum, "maximum": r.maximum, "integer": r.integer,
            })).collect::<Vec<_>>(),
        })
    }

    pub fn from_json(data: &Value) -> Result<Self, String> {
        if data.get("version").and_then(Value::as_i64).unwrap_or(1) != 1 {
            return Err("Unsupported MIDI mapping version".into());
        }
        let items = |k: &str| data.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
        let int = |v: &Value, k: &str| -> Result<u8, String> {
            v.get(k).and_then(Value::as_i64).map(|n| n.clamp(0, 127) as u8).ok_or(format!("mapping item needs '{k}'"))
        };
        let cmd = |v: &Value| -> Result<CommandName, String> {
            let s = v.get("command").and_then(Value::as_str).unwrap_or_default();
            CommandName::parse(s).ok_or(format!("unknown command {s:?}"))
        };
        let mut m = Self::default();
        for item in items("cc_commands") {
            m.cc_commands.insert(int(&item, "control")?, cmd(&item)?);
        }
        for item in items("note_commands") {
            m.note_commands.insert((int(&item, "channel")?, int(&item, "note")?), cmd(&item)?);
        }
        for item in items("absolute_cc") {
            let f = |k: &str| item.get(k).and_then(Value::as_f64).ok_or(format!("absolute_cc needs '{k}'"));
            m.absolute_cc.push(AbsoluteCc {
                control: int(&item, "control")?,
                command: cmd(&item)?,
                minimum: f("minimum")?,
                maximum: f("maximum")?,
                integer: item.get("integer").and_then(Value::as_bool).unwrap_or(false),
            });
        }
        Ok(m)
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::from_json(&serde_json::from_str(&text).map_err(|e| e.to_string())?)
    }

    /// Missing file → empty mapping.
    pub fn load_or_default(path: &Path) -> Result<Self, String> {
        if path.exists() { Self::load(path) } else { Ok(Self::default()) }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        crate::patterns::write_json_atomic(path, &self.to_json()).map_err(|e| e.0)
    }
}
