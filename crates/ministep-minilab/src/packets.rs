//! Pure MiniLab 3 SysEx builders (no IO).
//!
//! The format is empirically derived; every builder reproduces the legacy
//! Python `minilab3_display.py` byte-for-byte, including the `F0 ... F7` framing
//! mido adds.

/// Arturia manufacturer ID, MiniLab 3 device bytes.
pub const HEADER: [u8; 5] = [0x00, 0x20, 0x6B, 0x7F, 0x42];
const ARTURIA_CONNECT: [u8; 7] = [0x04, 0x01, 0x60, 0x01, 0x00, 0x02, 0x00];
const DAW_CONNECT: [u8; 5] = [0x02, 0x02, 0x40, 0x6A, 0x21];
/// Longest text the OLED accepts per line, in bytes.
pub const MAX_TEXT: usize = 30;

/// An RGB pad colour, each component 0..=127.
pub type Rgb = [u8; 3];

/// Idle pads: dim blue.
pub const PAD_IDLE: Rgb = [0, 0, 20];
/// Playhead while playing.
pub const PAD_PLAYHEAD: Rgb = [0, 127, 0];
/// Recording cursor / record indicator.
pub const PAD_RECORD: Rgb = [127, 0, 0];
/// Record-armed indicator on pad 8.
pub const PAD_AMBER: Rgb = [127, 42, 0];

/// Wrap a body in `F0 <header> body F7`.
pub fn sysex(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + HEADER.len() + 2);
    out.push(0xF0);
    out.extend_from_slice(&HEADER);
    out.extend_from_slice(body);
    out.push(0xF7);
    out
}

/// Screen-safe text: one `?` per non-ASCII character, cut to 30 bytes
/// (Python `str.encode("ascii", errors="replace")[:30]`).
pub fn text_bytes(value: &str) -> Vec<u8> {
    value.chars().map(|c| if c.is_ascii() { c as u8 } else { b'?' }).take(MAX_TEXT).collect()
}

/// The two messages that claim the Arturia/DAW display channel before drawing.
pub fn connect() -> [Vec<u8>; 2] {
    [sysex(&ARTURIA_CONNECT), sysex(&DAW_CONNECT)]
}

/// Two-line text screen.
pub fn text_screen(line1: &str, line2: &str) -> Vec<u8> {
    let mut body = vec![0x04, 0x02, 0x60, 0x1F, 0x02, 0x01, 0x00, 0x01];
    body.extend(text_bytes(line1));
    body.extend([0x00, 0x02]);
    body.extend(text_bytes(line2));
    body.push(0x00);
    sysex(&body)
}

/// Encoder screen: label, a 0..=127 value bar, and the value text (defaults to
/// the zero-padded value, e.g. `096`). Callers must keep `value <= 127`.
pub fn encoder_screen(label: &str, value: u8, value_text: Option<&str>) -> Vec<u8> {
    debug_assert!(value <= 127);
    let mut body = vec![0x04, 0x02, 0x60, 0x1F, 0x03, 0x01, value, 0x00, 0x00, 0x01];
    body.extend(text_bytes(label));
    body.extend([0x00, 0x02]);
    match value_text {
        Some(text) => body.extend(text_bytes(text)),
        None => body.extend(text_bytes(&format!("{value:03}"))),
    }
    body.push(0x00);
    sysex(&body)
}

/// Eight temporary RGB pad colours (sent on the normal MIDI port).
/// Callers must keep every component `<= 127`.
pub fn pad_colours(colours: &[Rgb; 8]) -> Vec<u8> {
    let mut body = vec![0x04, 0x02, 0x16, 0x00];
    body.extend(colours.iter().flatten());
    sysex(&body)
}

/// MiniStep transport state as pad colours.
///
/// The playback step moves in green. While stopped and recording, the cursor
/// step is red. Pad 8 is amber as a record indicator, or red while recording
/// during playback (amber again when the playhead itself is on pad 8).
pub fn step_status_colours(recording: bool, playing: bool, playhead: usize, cursor: usize) -> [Rgb; 8] {
    let mut colours = [PAD_IDLE; 8];
    let mut active = playhead % 8;
    if playing {
        colours[active] = PAD_PLAYHEAD;
    } else if recording {
        active = cursor % 8;
        colours[active] = PAD_RECORD;
    }
    if recording && playing {
        colours[7] = if active == 7 { PAD_AMBER } else { PAD_RECORD };
    } else if recording && active != 7 {
        colours[7] = PAD_AMBER;
    }
    colours
}
