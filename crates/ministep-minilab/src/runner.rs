//! Background thread that mirrors the engine onto the MiniLab.

use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, bounded, never, select, tick};
use ministep_core::engine::{Engine, EngineEvent};

use crate::display::MiniLab3Display;

/// How often the screen is re-sent and the pads are checked.
pub const REFRESH_INTERVAL: Duration = Duration::from_millis(100);

type StepStatus = (bool, bool, usize, usize);

/// Running display thread. Dropping it stops the thread and closes the display.
pub struct DisplayHandle {
    stop: Option<crossbeam_channel::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl DisplayHandle {
    /// Stop and join (same as dropping).
    pub fn stop(self) {}
}

impl Drop for DisplayHandle {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Connect the display, show `MINISTEP / READY`, then mirror engine events and
/// re-send the screen every [`REFRESH_INTERVAL`], updating the pads whenever
/// (recording, playing, playhead, cursor) changes.
pub fn run_display(engine: Arc<Engine>, display: MiniLab3Display) -> DisplayHandle {
    run_display_with_interval(engine, display, REFRESH_INTERVAL)
}

/// [`run_display`] with a custom refresh interval.
pub fn run_display_with_interval(engine: Arc<Engine>, mut display: MiniLab3Display, interval: Duration) -> DisplayHandle {
    // Subscribe before spawning so no event is missed.
    let events = engine.events();
    let (stop_tx, stop_rx) = bounded::<()>(0);
    let thread = std::thread::Builder::new()
        .name("ministep-minilab".into())
        .spawn(move || {
            display.connect();
            display.show_text("MINISTEP", "READY");
            display_loop(&engine, &mut display, &events, &stop_rx, interval);
            drop(display);
        })
        .expect("spawn MiniLab display thread");
    DisplayHandle { stop: Some(stop_tx), thread: Some(thread) }
}

fn display_loop(
    engine: &Engine,
    display: &mut MiniLab3Display,
    events: &Receiver<EngineEvent>,
    stop: &Receiver<()>,
    interval: Duration,
) {
    let ticker = tick(interval);
    let disconnected = never();
    let mut events = events;
    let mut last: Option<StepStatus> = None;
    refresh(engine, display, &mut last);
    loop {
        select! {
            recv(stop) -> _ => return,
            recv(events) -> event => match event {
                Ok(event) => display.show_event(&event),
                Err(_) => events = &disconnected,
            },
            recv(ticker) -> _ => refresh(engine, display, &mut last),
        }
    }
}

fn refresh(engine: &Engine, display: &mut MiniLab3Display, last: &mut Option<StepStatus>) {
    display.refresh();
    let status = {
        let s = engine.state();
        (s.recording, s.playing, s.playhead, s.cursor)
    };
    if *last != Some(status) {
        // Only fails without a pad port; the screen still works.
        let _ = display.show_step_status(status.0, status.1, status.2, status.3);
        *last = Some(status);
    }
}
