//! The MiniLab 3 display client: OLED screens and RGB pads over raw MIDI.

use std::sync::{Arc, Mutex};

use ministep_core::engine::EngineEvent;
use ministep_core::message::MidiMessage;
use ministep_core::profiles::minilab_knob;
use ministep_midi::{MidiError, RawOutput, find_port, output_ports};

use crate::packets::{self, Rgb};

/// Normal MIDI port: pad colours.
pub const MIDI_PORT: &str = "Minilab3 MIDI";
/// ALV port: OLED SysEx.
pub const ALV_PORT: &str = "Minilab3 ALV";

/// Where display bytes go. Implemented for [`RawOutput`]; tests record bytes.
pub trait DisplaySink {
    fn send(&mut self, bytes: &[u8]);
}

impl DisplaySink for RawOutput {
    fn send(&mut self, bytes: &[u8]) {
        // The firmware gets a fresh copy every refresh, so a dropped packet heals itself.
        let _ = RawOutput::send(self, bytes);
    }
}

/// Records every message; clones share one log.
#[derive(Clone, Default)]
pub struct MemorySink(Arc<Mutex<Vec<Vec<u8>>>>);

impl MemorySink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn messages(&self) -> Vec<Vec<u8>> {
        self.0.lock().unwrap().clone()
    }

    pub fn last(&self) -> Option<Vec<u8>> {
        self.0.lock().unwrap().last().cloned()
    }

    pub fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl DisplaySink for MemorySink {
    fn send(&mut self, bytes: &[u8]) {
        self.0.lock().unwrap().push(bytes.to_vec());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayError {
    EncoderValue(u8),
    PadCount(usize),
    PadComponent(u8),
    NoPadPort,
}

impl std::fmt::Display for DisplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EncoderValue(v) => write!(f, "MiniLab encoder value must be between 0 and 127 (got {v})"),
            Self::PadCount(n) => write!(f, "MiniLab needs exactly eight RGB pad colours (got {n})"),
            Self::PadComponent(v) => write!(f, "MiniLab RGB components must be between 0 and 127 (got {v})"),
            Self::NoPadPort => f.write_str("A MIDI pad output port was not configured"),
        }
    }
}

impl std::error::Error for DisplayError {}

type Sink = Box<dyn DisplaySink + Send>;

/// A MiniLab 3 OLED screen and, optionally, its RGB pads.
///
/// The last screen is retained; [`refresh`](Self::refresh) re-sends it because
/// the firmware overwrites the OLED with its own status. Dropping the display
/// closes its ports.
pub struct MiniLab3Display {
    display: Sink,
    pads: Option<Sink>,
    screen: Option<Vec<u8>>,
}

impl MiniLab3Display {
    /// Display without pads.
    pub fn new(display: impl DisplaySink + Send + 'static) -> Self {
        Self { display: Box::new(display), pads: None, screen: None }
    }

    /// Display plus pad-colour output.
    pub fn with_pads(display: impl DisplaySink + Send + 'static, pads: impl DisplaySink + Send + 'static) -> Self {
        Self { display: Box::new(display), pads: Some(Box::new(pads)), screen: None }
    }

    /// Open the MiniLab's ALV (OLED) and MIDI (pad) ports. Call [`connect`](Self::connect) next.
    pub fn open() -> Result<Self, MidiError> {
        Self::open_ports(ALV_PORT, MIDI_PORT)
    }

    /// Open named ports: exact name first, else an unambiguous match from [`find_port`].
    pub fn open_ports(display_port: &str, pad_port: &str) -> Result<Self, MidiError> {
        let ports = output_ports();
        let resolve = |name: &str| find_port(name, &ports).unwrap_or_else(|| name.to_string());
        let display = RawOutput::open(&resolve(display_port))?;
        let pads = RawOutput::open(&resolve(pad_port))?;
        Ok(Self::with_pads(display, pads))
    }

    /// Claim the Arturia/DAW display channel before drawing.
    pub fn connect(&mut self) {
        for packet in packets::connect() {
            self.display.send(&packet);
        }
    }

    /// Two-line text screen.
    pub fn show_text(&mut self, line1: &str, line2: &str) {
        self.screen = Some(packets::text_screen(line1, line2));
        self.refresh();
    }

    /// Encoder screen with a 0..=127 value bar.
    pub fn show_encoder(&mut self, label: &str, value: u8, value_text: Option<&str>) -> Result<(), DisplayError> {
        if value > 127 {
            return Err(DisplayError::EncoderValue(value));
        }
        self.screen = Some(packets::encoder_screen(label, value, value_text));
        self.refresh();
        Ok(())
    }

    /// Render a physical MiniLab knob CC; returns whether it was one.
    /// An empty `label` falls back to `CC <n>`, like the Python `label or ...`.
    pub fn handle_message(&mut self, message: &MidiMessage, label: Option<&str>, value_text: Option<&str>) -> bool {
        let MidiMessage::ControlChange { control, value, .. } = *message else { return false };
        if minilab_knob(control).is_none() {
            return false;
        }
        let label = match label {
            Some(l) if !l.is_empty() => l.to_string(),
            _ => format!("CC {control}"),
        };
        // Parsed messages are always in range; a hand-built value > 127 is not drawn.
        self.show_encoder(&label, value, value_text).is_ok()
    }

    /// Show an engine event (the `receive()` display logic of the Python app).
    ///
    /// `Command` events carry no source CC, so they render as a text screen and
    /// only when they have a value (see crate docs).
    pub fn show_event(&mut self, event: &EngineEvent) {
        match event {
            EngineEvent::Menu(screen) => match screen {
                Some((line1, line2)) => self.show_text(line1, line2),
                None => self.show_text("MINISTEP", "READY"),
            },
            EngineEvent::Knob { page, label, value } => {
                let _ = self.show_encoder(&format!("{page} {label}"), *value, None);
            }
            EngineEvent::Command { label, value: Some(value) } => self.show_text(label, value),
            EngineEvent::Command { value: None, .. } => {}
            EngineEvent::Input(message) => {
                self.handle_message(message, None, None);
            }
        }
    }

    /// Re-send the most recent screen.
    pub fn refresh(&mut self) {
        if let Some(screen) = &self.screen {
            self.display.send(screen);
        }
    }

    /// Set the eight pad colours, each component 0..=127.
    pub fn set_pad_colours(&mut self, colours: &[Rgb]) -> Result<(), DisplayError> {
        let pads = self.pads.as_mut().ok_or(DisplayError::NoPadPort)?;
        let colours: &[Rgb; 8] = colours.try_into().map_err(|_| DisplayError::PadCount(colours.len()))?;
        if let Some(&bad) = colours.iter().flatten().find(|&&c| c > 127) {
            return Err(DisplayError::PadComponent(bad));
        }
        pads.send(&packets::pad_colours(colours));
        Ok(())
    }

    /// Show transport state across the pads (see [`packets::step_status_colours`]).
    pub fn show_step_status(&mut self, recording: bool, playing: bool, playhead: usize, cursor: usize) -> Result<(), DisplayError> {
        self.set_pad_colours(&packets::step_status_colours(recording, playing, playhead, cursor))
    }

    /// Close the ports (same as dropping).
    pub fn close(self) {}
}
