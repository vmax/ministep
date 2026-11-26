//! MiniStep core: sequencer state, file formats, transport, and engine.
//! No UI and no MIDI backend; outputs are injected as [`output::RawMidiOut`].

pub mod clock;
pub mod controller;
pub mod drums;
pub mod engine;
mod jsonx;
pub mod message;
pub mod notes;
pub mod output;
pub mod patterns;
pub mod profiles;
pub mod rt;
pub mod state;
pub mod timing;
pub mod transport;
pub mod voice;

pub use controller::{Command, CommandName, Mapping};
pub use engine::{Engine, EngineConfig, EngineEvent};
pub use message::MidiMessage;
pub use output::{OwnedOutput, Owner, RawMidiOut, SharedOutput};
pub use state::{AppState, Step, Track};
pub use transport::{Outputs, Transport};
pub use voice::{DrumRoute, DrumVoiceSink, VoiceModel, VoiceParams};
