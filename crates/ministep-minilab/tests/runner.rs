//! `run_display` against a real engine (no MIDI hardware: sinks record bytes).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ministep_core::engine::{Engine, EngineConfig};
use ministep_core::message::MidiMessage;
use ministep_core::patterns::PatternStore;
use ministep_core::{AppState, Outputs};
use ministep_minilab::packets::{self, step_status_colours};
use ministep_minilab::{DisplaySink, MemorySink, MiniLab3Display, run_display, run_display_with_interval};

fn engine(dir: &tempfile::TempDir) -> Arc<Engine> {
    let config = EngineConfig {
        patterns: PatternStore::new(dir.path().join("patterns")),
        sequence_path: dir.path().join("sequence.json"),
        encoder_menu: true,
        realtime: false,
        ..EngineConfig::default()
    };
    Engine::new(AppState::default(), Outputs::default(), config)
}

fn cc(control: u8, value: u8) -> MidiMessage {
    MidiMessage::ControlChange { channel: 0, control, value }
}

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn mirrors_engine_events_refreshes_and_tracks_pads() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir);
    let (screen, pads) = (MemorySink::new(), MemorySink::new());
    let handle = run_display_with_interval(
        Arc::clone(&engine),
        MiniLab3Display::with_pads(screen.clone(), pads.clone()),
        Duration::from_millis(10),
    );

    // Startup: connect, splash, then initial pad state.
    let [connect_a, connect_b] = packets::connect();
    let splash = packets::text_screen("MINISTEP", "READY");
    wait_for("splash", || screen.len() >= 3);
    assert_eq!(screen.messages()[..3], [connect_a, connect_b, splash.clone()]);
    let idle = packets::pad_colours(&step_status_colours(false, false, 0, 0));
    wait_for("initial pads", || pads.last() == Some(idle.clone()));

    // The screen is re-sent periodically.
    let n = screen.len();
    wait_for("refresh", || screen.len() >= n + 3);
    assert!(screen.messages()[3..].iter().all(|m| *m == splash));
    // Pads are only re-sent on change.
    assert_eq!(pads.len(), 1);

    // Unmapped knob CC -> Input event -> encoder screen.
    engine.handle_midi(cc(74, 64));
    let knob = packets::encoder_screen("CC 74", 64, None);
    wait_for("knob screen", || screen.last() == Some(knob.clone()));

    // Encoder press opens the menu; shift-press closes it.
    engine.handle_midi(cc(115, 127));
    let (l1, l2) = engine.menu_screen().expect("menu open");
    let menu = packets::text_screen(&l1, &l2);
    wait_for("menu screen", || screen.last() == Some(menu.clone()));
    engine.handle_midi(cc(113, 127));
    wait_for("menu closed", || screen.last() == Some(splash.clone()));

    // Transport change -> pads.
    engine.edit(|s| {
        s.recording = true;
        s.cursor = 3;
    });
    let rec = packets::pad_colours(&step_status_colours(true, false, 0, 3));
    wait_for("record pads", || pads.last() == Some(rec.clone()));
    let count = pads.len();

    drop(handle);
    let after = (screen.len(), pads.len());
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!((screen.len(), pads.len()), after, "thread stopped");
    assert_eq!(count, 2);
    engine.shutdown();
}

struct DropFlag(Arc<AtomicBool>);

impl DisplaySink for DropFlag {
    fn send(&mut self, _: &[u8]) {}
}

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[test]
fn dropping_the_handle_closes_the_display() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir);
    let (screen_closed, pads_closed) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let display = MiniLab3Display::with_pads(DropFlag(screen_closed.clone()), DropFlag(pads_closed.clone()));
    let handle = run_display(Arc::clone(&engine), display);
    std::thread::sleep(Duration::from_millis(20));
    assert!(!screen_closed.load(Ordering::SeqCst));
    drop(handle);
    assert!(screen_closed.load(Ordering::SeqCst));
    assert!(pads_closed.load(Ordering::SeqCst));
    engine.shutdown();
}
