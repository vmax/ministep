//! Parsed short MIDI messages (channels are zero-based, like mido).

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidiMessage {
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8, velocity: u8 },
    ControlChange { channel: u8, control: u8, value: u8 },
    /// Anything else (pitch bend, aftertouch, SysEx, clock...), passed through raw.
    Other(Vec<u8>),
}

impl MidiMessage {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let (&status, data) = bytes.split_first()?;
        let channel = status & 0x0f;
        let d = |i: usize| data.get(i).copied().unwrap_or(0) & 0x7f;
        Some(match status & 0xf0 {
            0x90 if data.len() >= 2 => Self::NoteOn { channel, note: d(0), velocity: d(1) },
            0x80 if data.len() >= 2 => Self::NoteOff { channel, note: d(0), velocity: d(1) },
            0xB0 if data.len() >= 2 => Self::ControlChange { channel, control: d(0), value: d(1) },
            _ => Self::Other(bytes.to_vec()),
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        match *self {
            Self::NoteOn { channel, note, velocity } => vec![0x90 | channel, note, velocity],
            Self::NoteOff { channel, note, velocity } => vec![0x80 | channel, note, velocity],
            Self::ControlChange { channel, control, value } => vec![0xB0 | channel, control, value],
            Self::Other(ref b) => b.clone(),
        }
    }

    /// Note On with velocity > 0.
    pub fn is_note_press(&self) -> bool {
        matches!(self, Self::NoteOn { velocity, .. } if *velocity > 0)
    }
}
