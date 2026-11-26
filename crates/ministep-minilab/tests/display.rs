//! Ported from legacy/tests/test_minilab3_display*.py. Reference bytes were
//! printed by the Python `MiniLab3Display` (mido `Message.bytes()`).

use ministep_core::engine::{EngineEvent, display_command};
use ministep_core::message::MidiMessage;
use ministep_core::{Command, CommandName};
use ministep_minilab::packets::{self, step_status_colours};
use ministep_minilab::{DisplayError, MemorySink, MiniLab3Display};

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
}

fn cc(control: u8, value: u8) -> MidiMessage {
    MidiMessage::ControlChange { channel: 0, control, value }
}

fn with_pads() -> (MiniLab3Display, MemorySink, MemorySink) {
    let (screen, pads) = (MemorySink::new(), MemorySink::new());
    (MiniLab3Display::with_pads(screen.clone(), pads.clone()), screen, pads)
}

// --- reference bytes ---------------------------------------------------------------

#[test]
fn connect_matches_python() {
    let screen = MemorySink::new();
    let mut display = MiniLab3Display::new(screen.clone());
    display.connect();
    assert_eq!(
        screen.messages(),
        vec![hex("F0 00 20 6B 7F 42 04 01 60 01 00 02 00 F7"), hex("F0 00 20 6B 7F 42 02 02 40 6A 21 F7")]
    );
}

#[test]
fn text_screen_matches_python() {
    assert_eq!(
        packets::text_screen("MINISTEP", "READY"),
        hex("F0 00 20 6B 7F 42 04 02 60 1F 02 01 00 01 4D 49 4E 49 53 54 45 50 00 02 52 45 41 44 59 00 F7")
    );
}

#[test]
fn text_replaces_non_ascii_per_char_and_truncates_to_30_bytes() {
    assert_eq!(
        packets::text_screen("héllo wörld ✓ and a very long line exceeding thirty chars", ""),
        hex(
            "F0 00 20 6B 7F 42 04 02 60 1F 02 01 00 01 68 3F 6C 6C 6F 20 77 3F 72 6C 64 20 3F 20 61 6E 64 20 \
             61 20 76 65 72 79 20 6C 6F 6E 67 20 00 02 00 F7"
        )
    );
}

#[test]
fn encoder_screens_match_python() {
    assert_eq!(
        packets::encoder_screen("CUTOFF", 96, None),
        hex("F0 00 20 6B 7F 42 04 02 60 1F 03 01 60 00 00 01 43 55 54 4F 46 46 00 02 30 39 36 00 F7")
    );
    assert_eq!(
        packets::encoder_screen("BPM", 5, Some("142")),
        hex("F0 00 20 6B 7F 42 04 02 60 1F 03 01 05 00 00 01 42 50 4D 00 02 31 34 32 00 F7")
    );
}

#[test]
fn pad_colours_match_python() {
    let (mut display, _, pads) = with_pads();
    display.set_pad_colours(&[[1, 2, 3]; 8]).unwrap();
    assert_eq!(
        pads.last().unwrap(),
        hex("F0 00 20 6B 7F 42 04 02 16 00 01 02 03 01 02 03 01 02 03 01 02 03 01 02 03 01 02 03 01 02 03 01 02 03 F7")
    );
}

#[test]
fn step_status_matches_python_for_every_branch() {
    type Case = ((bool, bool, usize, usize), &'static str);
    let cases: [Case; 7] = [
        ((false, false, 0, 0), "00 00 14 00 00 14 00 00 14 00 00 14 00 00 14 00 00 14 00 00 14 00 00 14"),
        ((true, true, 2, 5), "00 00 14 00 00 14 00 7F 00 00 00 14 00 00 14 00 00 14 00 00 14 7F 00 00"),
        ((true, false, 0, 3), "00 00 14 00 00 14 00 00 14 7F 00 00 00 00 14 00 00 14 00 00 14 7F 2A 00"),
        ((true, true, 7, 0), "00 00 14 00 00 14 00 00 14 00 00 14 00 00 14 00 00 14 00 00 14 7F 2A 00"),
        ((true, false, 0, 7), "00 00 14 00 00 14 00 00 14 00 00 14 00 00 14 00 00 14 00 00 14 7F 00 00"),
        ((false, true, 9, 1), "00 00 14 00 7F 00 00 00 14 00 00 14 00 00 14 00 00 14 00 00 14 00 00 14"),
        ((true, false, 3, 15), "00 00 14 00 00 14 00 00 14 00 00 14 00 00 14 00 00 14 00 00 14 7F 00 00"),
    ];
    for ((recording, playing, playhead, cursor), colours) in cases {
        let (mut display, _, pads) = with_pads();
        display.show_step_status(recording, playing, playhead, cursor).unwrap();
        let expected = hex(&format!("F0 00 20 6B 7F 42 04 02 16 00 {colours} F7"));
        assert_eq!(pads.last().unwrap(), expected, "{recording} {playing} {playhead} {cursor}");
        assert_eq!(packets::pad_colours(&step_status_colours(recording, playing, playhead, cursor)), expected);
    }
}

// --- test_minilab3_display.py ------------------------------------------------------

#[test]
fn encoder_view_uses_label_value_and_refreshes() {
    let screen = MemorySink::new();
    let mut display = MiniLab3Display::new(screen.clone());
    display.connect();
    display.show_encoder("CUTOFF", 96, None).unwrap();
    display.refresh();

    let messages = screen.messages();
    assert_eq!(messages.len(), 4);
    let last = messages.last().unwrap();
    assert_eq!(last[0], 0xF0);
    let mut expected = vec![0x1F, 0x03, 0x01, 96, 0x00, 0x00, 0x01];
    expected.extend(b"CUTOFF");
    expected.extend([0x00, 0x02]);
    expected.extend(b"096");
    expected.extend([0x00, 0xF7]);
    assert!(last.ends_with(&expected));
    assert_eq!(messages[2], messages[3]);
}

#[test]
fn known_knob_cc_updates_the_display() {
    let screen = MemorySink::new();
    let mut display = MiniLab3Display::new(screen.clone());

    assert!(display.handle_message(&cc(87, 55), None, None));
    assert_eq!(
        screen.last().unwrap(),
        hex("F0 00 20 6B 7F 42 04 02 60 1F 03 01 37 00 00 01 43 43 20 38 37 00 02 30 35 35 00 F7")
    );
    assert!(!display.handle_message(&cc(1, 55), None, None));
    assert!(!display.handle_message(&MidiMessage::NoteOn { channel: 0, note: 74, velocity: 9 }, None, None));
    assert_eq!(screen.len(), 1);
}

#[test]
fn empty_label_falls_back_but_empty_value_text_is_kept() {
    let screen = MemorySink::new();
    let mut display = MiniLab3Display::new(screen.clone());
    assert!(display.handle_message(&cc(74, 0), Some(""), Some("")));
    assert_eq!(screen.last().unwrap(), hex("F0 00 20 6B 7F 42 04 02 60 1F 03 01 00 00 00 01 43 43 20 37 34 00 02 00 F7"));
}

#[test]
fn pad_colours_require_eight_valid_rgb_triplets() {
    let (mut display, _, pads) = with_pads();
    display.set_pad_colours(&[[1, 2, 3]; 8]).unwrap();
    assert_eq!(pads.len(), 1);
    let err = display.set_pad_colours(&[[1, 2, 3]; 7]).unwrap_err();
    assert_eq!(err, DisplayError::PadCount(7));
    assert!(err.to_string().contains("eight"));
    let err = display.set_pad_colours(&[[128, 2, 3]; 8]).unwrap_err();
    assert!(err.to_string().contains("0 and 127"));
    assert_eq!(pads.len(), 1);
}

#[test]
fn encoder_value_must_be_in_range_and_pads_need_a_port() {
    let screen = MemorySink::new();
    let mut display = MiniLab3Display::new(screen.clone());
    assert_eq!(display.show_encoder("X", 128, None), Err(DisplayError::EncoderValue(128)));
    assert!(screen.is_empty());
    assert_eq!(display.show_step_status(false, false, 0, 0), Err(DisplayError::NoPadPort));
}

#[test]
fn refresh_without_a_screen_sends_nothing() {
    let screen = MemorySink::new();
    let mut display = MiniLab3Display::new(screen.clone());
    display.refresh();
    assert!(screen.is_empty());
}

// --- engine events (main.py receive()) ---------------------------------------------

#[test]
fn engine_events_render_like_the_python_receive_loop() {
    let screen = MemorySink::new();
    let mut display = MiniLab3Display::new(screen.clone());

    display.show_event(&EngineEvent::Menu(Some(("MENU > BPM".into(), "120 BPM".into()))));
    assert_eq!(screen.last().unwrap(), packets::text_screen("MENU > BPM", "120 BPM"));

    display.show_event(&EngineEvent::Menu(None));
    assert_eq!(screen.last().unwrap(), packets::text_screen("MINISTEP", "READY"));

    display.show_event(&EngineEvent::Knob { page: "FILTER".into(), label: "CUTOFF".into(), value: 96 });
    assert_eq!(screen.last().unwrap(), packets::encoder_screen("FILTER CUTOFF", 96, None));

    display.show_event(&EngineEvent::Command { label: "BPM".into(), value: Some("142".into()), source: cc(116, 80) });
    assert_eq!(screen.last().unwrap(), packets::encoder_screen("BPM", 80, Some("142")));

    let before = screen.len();
    let pad = MidiMessage::NoteOn { channel: 9, note: 36, velocity: 100 };
    display.show_event(&EngineEvent::Command { label: "PLAY STOP".into(), value: None, source: pad });
    display.show_event(&EngineEvent::Input(cc(1, 5)));
    assert_eq!(screen.len(), before);

    display.show_event(&EngineEvent::Input(cc(116, 64)));
    assert_eq!(screen.last().unwrap(), packets::encoder_screen("CC 116", 64, None));
}

// --- test_minilab3_display_integration.py ------------------------------------------

#[test]
fn display_command_uses_parameter_names_and_scaled_values() {
    let show = |name, value| display_command(&Command::with(name, value));
    assert_eq!(show(CommandName::SET_BPM, 142.0), ("BPM".into(), Some("142".into())));
    assert_eq!(show(CommandName::SET_GATE, 0.65), ("GATE".into(), Some("0.65".into())));
    assert_eq!(show(CommandName::SET_TRANSPOSE, -12.0), ("TRANSPOSE".into(), Some("-12".into())));
}
