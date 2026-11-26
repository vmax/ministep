//! Arturia MiniLab 3 OLED and RGB-pad display for MiniStep.
//!
//! - [`packets`]: pure SysEx builders, byte-identical to the legacy Python.
//! - [`MiniLab3Display`]: screen state over any [`DisplaySink`].
//! - [`run_display`]: a thread mirroring an [`Engine`](ministep_core::Engine).
//!
//! Engine `Command` events carry a label and formatted value but not the CC
//! that produced them, so they are shown as a text screen (label / value)
//! rather than the Python encoder screen with a value bar, and value-less
//! commands are not shown.

pub mod display;
pub mod packets;
pub mod runner;

pub use display::{ALV_PORT, DisplayError, DisplaySink, MIDI_PORT, MemorySink, MiniLab3Display};
pub use runner::{DisplayHandle, REFRESH_INTERVAL, run_display, run_display_with_interval};
