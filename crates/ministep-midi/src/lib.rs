//! MIDI port discovery and connections (CoreMIDI via midir).

use midir::os::unix::{VirtualInput, VirtualOutput};
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use ministep_core::output::{OwnedOutput, RawMidiOut, SharedOutput};
use ministep_core::message::MidiMessage;

const CLIENT: &str = "MiniStep";

pub fn input_ports() -> Vec<String> {
    let Ok(midi) = MidiInput::new(CLIENT) else { return Vec::new() };
    midi.ports().iter().filter_map(|p| midi.port_name(p).ok()).collect()
}

pub fn output_ports() -> Vec<String> {
    let Ok(midi) = MidiOutput::new(CLIENT) else { return Vec::new() };
    midi.ports().iter().filter_map(|p| midi.port_name(p).ok()).collect()
}

/// Exact, case-insensitive, or unambiguous substring match.
pub fn find_port(query: &str, ports: &[String]) -> Option<String> {
    if let Some(p) = ports.iter().find(|p| *p == query) {
        return Some(p.clone());
    }
    let q = query.to_lowercase();
    if let Some(p) = ports.iter().find(|p| p.to_lowercase() == q) {
        return Some(p.clone());
    }
    let matches: Vec<_> = ports.iter().filter(|p| p.to_lowercase().contains(&q)).collect();
    (matches.len() == 1).then(|| matches[0].clone())
}

struct Conn(MidiOutputConnection);

impl RawMidiOut for Conn {
    fn send(&mut self, bytes: &[u8]) {
        let _ = self.0.send(bytes);
    }
}

#[derive(Debug)]
pub struct MidiError(pub String);

impl std::fmt::Display for MidiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MidiError {}

fn err(e: impl std::fmt::Display) -> MidiError {
    MidiError(e.to_string())
}

/// Open an output port by exact name.
pub fn open_output(name: &str) -> Result<SharedOutput, MidiError> {
    let midi = MidiOutput::new(CLIENT).map_err(err)?;
    let port = midi
        .ports()
        .into_iter()
        .find(|p| midi.port_name(p).is_ok_and(|n| n == name))
        .ok_or_else(|| MidiError(format!("No MIDI output named {name:?}")))?;
    let conn = midi.connect(&port, "ministep-out").map_err(err)?;
    Ok(OwnedOutput::new(name, Box::new(Conn(conn))).shared())
}

/// Create a virtual source other apps (Ableton) see as `name`: no IAC setup needed.
pub fn open_virtual_output(name: &str) -> Result<SharedOutput, MidiError> {
    let midi = MidiOutput::new(CLIENT).map_err(err)?;
    let conn = midi.create_virtual(name).map_err(err)?;
    Ok(OwnedOutput::new(name, Box::new(Conn(conn))).shared())
}

/// An open input; dropping it closes the port.
pub struct InputHandle {
    _conn: MidiInputConnection<()>,
    pub name: String,
}

/// Open an input port; `on_message` runs on CoreMIDI's thread for every message.
pub fn open_input(
    name: &str,
    on_message: impl Fn(MidiMessage) + Send + 'static,
) -> Result<InputHandle, MidiError> {
    let mut midi = MidiInput::new(CLIENT).map_err(err)?;
    midi.ignore(midir::Ignore::TimeAndActiveSense);
    let port = midi
        .ports()
        .into_iter()
        .find(|p| midi.port_name(p).is_ok_and(|n| n == name))
        .ok_or_else(|| MidiError(format!("No MIDI input named {name:?}")))?;
    let conn = midi
        .connect(&port, "ministep-in", move |_, bytes, ()| {
            if let Some(m) = MidiMessage::parse(bytes) {
                on_message(m);
            }
        }, ())
        .map_err(err)?;
    Ok(InputHandle { _conn: conn, name: name.to_string() })
}

/// A virtual destination (for loopback tests and benches).
pub fn open_virtual_input(
    name: &str,
    on_message: impl Fn(&[u8]) + Send + 'static,
) -> Result<InputHandle, MidiError> {
    let midi = MidiInput::new(CLIENT).map_err(err)?;
    let conn = midi.create_virtual(name, move |_, bytes, ()| on_message(bytes), ()).map_err(err)?;
    Ok(InputHandle { _conn: conn, name: name.to_string() })
}

/// Raw SysEx/short-message output without ownership tracking (MiniLab display).
pub struct RawOutput(MidiOutputConnection);

impl RawOutput {
    pub fn open(name: &str) -> Result<Self, MidiError> {
        let midi = MidiOutput::new(CLIENT).map_err(err)?;
        let port = midi
            .ports()
            .into_iter()
            .find(|p| midi.port_name(p).is_ok_and(|n| n == name))
            .ok_or_else(|| MidiError(format!("No MIDI output named {name:?}")))?;
        Ok(Self(midi.connect(&port, "ministep-raw").map_err(err)?))
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError> {
        self.0.send(bytes).map_err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_port_rules() {
        let ports = vec!["IAC Driver MiniStep".to_string(), "IAC Driver Drums".to_string(), "Minilab3 MIDI".to_string()];
        assert_eq!(find_port("Minilab3 MIDI", &ports).as_deref(), Some("Minilab3 MIDI"));
        assert_eq!(find_port("minilab3 midi", &ports).as_deref(), Some("Minilab3 MIDI"));
        assert_eq!(find_port("drums", &ports).as_deref(), Some("IAC Driver Drums"));
        assert_eq!(find_port("IAC", &ports), None);
    }
}
