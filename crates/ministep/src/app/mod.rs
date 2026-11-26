//! The window: top transport bar, track list, step grid, synth strip, status line.
//!
//! Everything is painted by hand on egui's painter so the look stays close to
//! a piece of hardware. All edits go through `Engine` (thread-safe), so the
//! GUI, MIDI input and the remote socket never disagree.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, Key, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use ministep_core::controller::{Command, CommandName};
use ministep_core::drums::{DRUM_STEPS, DrumLane};
use ministep_core::engine::Engine;
use ministep_core::notes::note_name;
use ministep_core::patterns::PatternKind;
use ministep_core::profiles::PageKind;
use ministep_core::state::{AppState, LOOP_LENGTHS, Track};
use ministep_core::voice::{DrumRoute, VoiceModel, VoiceParams};

use crate::ports::Ports;
use crate::theme::{self, DARK, LIGHT, Palette, mono, sans};
use crate::widgets::{Icon, Knob, heading, icon_pill, paint_envelope, paint_icon, pill, text_pill};

mod keys;
mod layout;
mod modals;
mod steps;
mod synth;
mod top_bar;
mod tracks;

use layout::*;
pub use layout::MIN_WINDOW;

/// Extra GM drum notes offered when adding lanes.
const EXTRA_LANES: [(&str, u8); 8] =
    [("Clap", 39), ("Rim", 37), ("Low Tom", 45), ("Hi Tom", 50), ("Crash", 49), ("Ride", 51), ("Cowbell", 56), ("Shaker", 70)];

/// Toolbar button: icon, tooltip, action.
type Tool = (Icon, &'static str, Box<dyn Fn(&Engine)>);
/// Synth page snapshot: name, kind, knobs as (number, label, value, display).
type PageView = (String, PageKind, Vec<(u8, String, u8, String)>);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sel {
    Melodic,
    Lane(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    Step,
    Synth,
}

struct PatternDialog {
    save: bool,
    kind: PatternKind,
    name: String,
    names: Vec<String>,
    focus: bool,
}

pub struct App {
    engine: Arc<Engine>,
    pub ports: Ports,
    sel: Sel,
    tab: Tab,
    page: usize,
    follow: bool,
    dark: bool,
    settings_open: bool,
    patterns: Option<PatternDialog>,
    /// Name shown next to the logo: last saved/loaded pattern.
    title: String,
    profile_path: Option<PathBuf>,
    bpm_edit: Option<String>,
    note_drag: f32,
    last_cursor: usize,
    /// First visible row of the track list, and the selection it last followed.
    track_scroll: usize,
    track_scroll_follow: usize,
    /// Cached waveform preview: (params, peaks per column).
    preview: Option<(VoiceParams, Vec<f32>)>,
    /// Name of the audio device the drum synth plays on, if running.
    pub audio_device: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, engine: Arc<Engine>, ports: Ports, profile_path: Option<PathBuf>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let dark = std::env::var_os("MINISTEP_DARK").is_some();
        theme::apply_visuals(&cc.egui_ctx, if dark { &DARK } else { &LIGHT }, dark);
        let ctx = cc.egui_ctx.clone();
        engine.subscribe(move || ctx.request_repaint());
        let sel = if engine.state().active_track == Track::Drums { Sel::Lane(0) } else { Sel::Melodic };
        Self {
            engine,
            ports,
            sel,
            tab: Tab::Step,
            page: 0,
            follow: true,
            dark,
            settings_open: false,
            patterns: None,
            title: "sequence".into(),
            profile_path,
            bpm_edit: None,
            note_drag: 0.0,
            last_cursor: 0,
            track_scroll: 0,
            track_scroll_follow: 0,
            preview: None,
            audio_device: None,
        }
    }

    fn pal(&self) -> &'static Palette {
        if self.dark { &DARK } else { &LIGHT }
    }

    fn select(&mut self, sel: Sel) {
        self.sel = sel;
        self.engine.edit(|s| match sel {
            Sel::Melodic => s.active_track = Track::Melodic,
            Sel::Lane(i) => {
                s.active_track = Track::Drums;
                s.drums.cursor_lane = i.min(s.drums.lanes.len().saturating_sub(1));
            }
        });
    }

    fn sync_selection(&mut self, s: &AppState) {
        // The remote socket or MIDI can switch tracks; follow it.
        match (s.active_track, self.sel) {
            (Track::Melodic, Sel::Lane(_)) => self.sel = Sel::Melodic,
            (Track::Drums, Sel::Melodic) => self.sel = Sel::Lane(s.drums.cursor_lane),
            (Track::Drums, Sel::Lane(i)) if i != s.drums.cursor_lane || i >= s.drums.lanes.len() => {
                self.sel = Sel::Lane(s.drums.cursor_lane.min(s.drums.lanes.len().saturating_sub(1)));
            }
            _ => {}
        }
    }

    // --- status line ------------------------------------------------------------------------

    fn status_line(&self, ui: &Ui, rect: Rect, s: &AppState) {
        let p = self.pal();
        let painter = ui.painter();
        painter.text(pos2(rect.left(), rect.center().y), Align2::LEFT_CENTER, &s.status_message, mono(12.0), p.muted);
        let ports = format!(
            "IN {}   OUT {}{}",
            s.selected_input.as_deref().unwrap_or("—"),
            s.selected_output.as_deref().unwrap_or("—"),
            s.selected_drum_output.as_deref().map(|d| format!("   DRUMS {d}")).unwrap_or_default()
        );
        painter.text(pos2(rect.right(), rect.center().y), Align2::RIGHT_CENTER, ports, mono(12.0), p.faint);
    }

}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.keys(&ctx);
        let s = self.engine.state().clone();
        self.sync_selection(&s);
        let p = self.pal();
        let l = Layout::new(ui.max_rect());
        ui.painter().rect_filled(l.full, 0.0, p.bg);
        self.top_bar(ui, l.top, &s);
        self.track_list(ui, l.tracks, &s);
        self.tabs_and_tools(ui, l.tools, &s);
        match self.tab {
            Tab::Step => {
                self.step_area(ui, l.steps, &s);
                self.bottom_strip(ui, l.strip, &s);
            }
            Tab::Synth => self.synth_tab(ui, l.synth),
        }
        self.status_line(ui, l.status, &s);
        self.patterns_modal(&ctx);
        self.settings_modal(&ctx);
        if s.playing {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self) {
        self.engine.shutdown();
    }
}

