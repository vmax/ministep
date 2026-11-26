//! MIDI output abstraction with per-source note ownership.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

/// Who emitted a note: input pass-through, melodic playback, or drums.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Owner {
    Input,
    Sequencer,
    Drums,
}

/// A raw MIDI byte sink (a midir connection, a recorder in tests).
pub trait RawMidiOut: Send {
    fn send(&mut self, bytes: &[u8]);
}

impl RawMidiOut for Vec<Vec<u8>> {
    fn send(&mut self, bytes: &[u8]) {
        self.push(bytes.to_vec());
    }
}

/// Reference-counts output notes by source so one source's Note Off never cuts
/// another source's same-pitch note.
pub struct OwnedOutput {
    raw: Box<dyn RawMidiOut>,
    owners: HashMap<(Owner, u8, u8), u32>,
    pub name: String,
}

pub type SharedOutput = Arc<Mutex<OwnedOutput>>;

impl OwnedOutput {
    pub fn new(name: impl Into<String>, raw: Box<dyn RawMidiOut>) -> Self {
        Self { raw, owners: HashMap::new(), name: name.into() }
    }

    pub fn shared(self) -> SharedOutput {
        Arc::new(Mutex::new(self))
    }

    fn total(&self, note: u8, channel: u8) -> u32 {
        self.owners.iter().filter(|((_, n, c), _)| *n == note && *c == channel).map(|(_, v)| *v).sum()
    }

    /// `channel` is zero-based.
    pub fn note_on(&mut self, note: u8, velocity: u8, channel: u8, owner: Owner) {
        let was_sounding = self.total(note, channel) > 0;
        *self.owners.entry((owner, note, channel)).or_insert(0) += 1;
        if !was_sounding {
            self.raw.send(&[0x90 | (channel & 0x0f), note & 0x7f, velocity & 0x7f]);
        }
    }

    pub fn note_off(&mut self, note: u8, channel: u8, owner: Owner) {
        let key = (owner, note, channel);
        match self.owners.get_mut(&key) {
            None | Some(0) => return,
            Some(count) => {
                *count -= 1;
                if *count == 0 {
                    self.owners.remove(&key);
                }
            }
        }
        if self.total(note, channel) == 0 {
            self.raw.send(&[0x80 | (channel & 0x0f), note & 0x7f, 0]);
        }
    }

    pub fn control_change(&mut self, control: u8, value: u8, channel: u8) {
        self.raw.send(&[0xB0 | (channel & 0x0f), control & 0x7f, value & 0x7f]);
    }

    /// Send a raw message; channel messages are re-channelled when `channel` is set.
    pub fn send_raw(&mut self, bytes: &[u8], channel: Option<u8>) {
        match (bytes.first(), channel) {
            (Some(&status), Some(ch)) if (0x80..0xF0).contains(&status) => {
                let mut msg = bytes.to_vec();
                msg[0] = (status & 0xF0) | (ch & 0x0f);
                self.raw.send(&msg);
            }
            _ => self.raw.send(bytes),
        }
    }

    pub fn release_owner(&mut self, owner: Owner) {
        let keys: Vec<_> = self.owners.keys().filter(|k| k.0 == owner).copied().collect();
        for (o, note, channel) in keys {
            while self.owners.contains_key(&(o, note, channel)) {
                self.note_off(note, channel, o);
            }
        }
    }

    /// CC 123 on all 16 channels; forgets all ownership.
    pub fn all_notes_off(&mut self) {
        self.owners.clear();
        for ch in 0..16 {
            self.control_change(123, 0, ch);
        }
    }

    pub fn sounding(&self) -> usize {
        self.owners.values().map(|&v| v as usize).sum()
    }
}

/// Recording outputs for tests and simulations.
pub mod testing {
    use super::*;

    /// A raw output recording into a shared log, for assertions.
    #[derive(Clone, Default)]
    pub struct Recorder(pub Arc<Mutex<Vec<Vec<u8>>>>);

    impl RawMidiOut for Recorder {
        fn send(&mut self, bytes: &[u8]) {
            self.0.lock().push(bytes.to_vec());
        }
    }

    impl Recorder {
        pub fn output(&self) -> SharedOutput {
            OwnedOutput::new("test", Box::new(self.clone())).shared()
        }
        pub fn take(&self) -> Vec<Vec<u8>> {
            std::mem::take(&mut *self.0.lock())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Recorder;
    use super::*;

    #[test]
    fn ownership_prevents_cross_source_cutoff() {
        let rec = Recorder::default();
        let out = rec.output();
        let mut o = out.lock();
        o.note_on(60, 100, 0, Owner::Input);
        o.note_on(60, 90, 0, Owner::Sequencer);
        o.note_off(60, 0, Owner::Input);
        o.note_off(60, 0, Owner::Input); // unknown owner: ignored
        o.note_off(60, 0, Owner::Sequencer);
        assert_eq!(rec.take(), vec![vec![0x90, 60, 100], vec![0x80, 60, 0]]);
    }
}
