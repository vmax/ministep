//! The window: top transport bar, track list, step grid, synth strip, status line.
//!
//! Everything is painted by hand on egui's painter so the look stays close to
//! a piece of hardware. All edits go through `Engine` (thread-safe), so the
//! GUI, MIDI input and the remote socket never disagree.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Align2, Key, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use ministep_core::controller::{Command, CommandName};
use ministep_core::drums::{DRUM_STEPS, DrumLane};
use ministep_core::engine::Engine;
use ministep_core::notes::note_name;
use ministep_core::patterns::PatternKind;
use ministep_core::profiles::PageKind;
use ministep_core::state::{AppState, LOOP_LENGTHS, Track};

use crate::ports::Ports;
use crate::theme::{self, DARK, LIGHT, Palette, mono, sans};
use crate::widgets::{Icon, Knob, heading, icon_pill, paint_envelope, paint_icon, pill, text_pill};

const STEPS_PER_PAGE: usize = 16;
const GUTTER: f32 = 56.0;
/// Extra GM drum notes offered when adding lanes.
const EXTRA_LANES: [(&str, u8); 8] =
    [("Clap", 39), ("Rim", 37), ("Low Tom", 45), ("Hi Tom", 50), ("Crash", 49), ("Ride", 51), ("Cowbell", 56), ("Shaker", 70)];

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

    // --- top bar ----------------------------------------------------------------------

    fn top_bar(&mut self, ui: &mut Ui, rect: Rect, s: &AppState) {
        let p = self.pal();
        let painter = ui.painter().clone();
        let cy = rect.center().y;
        // Logo: a step-pattern mark.
        let mark = Rect::from_center_size(pos2(rect.left() + 44.0, cy), vec2(52.0, 30.0));
        for (i, h) in [1.0f32, 0.55, 0.8, 0.35].iter().enumerate() {
            let w = 10.0;
            let x = mark.left() + i as f32 * 13.5;
            let r = Rect::from_min_max(pos2(x, mark.bottom() - mark.height() * h), pos2(x + w, mark.bottom()));
            painter.rect_filled(r, 3.0, p.ink);
        }
        painter.text(pos2(rect.left() + 96.0, cy), Align2::LEFT_CENTER, &self.title, sans(22.0), p.ink);

        // Transport, centred.
        let mid = rect.center().x - 60.0;
        let stop = Rect::from_center_size(pos2(mid - 100.0, cy), vec2(44.0, 44.0));
        let sresp = ui.interact(stop, ui.scope_id().with("stop"), Sense::click());
        paint_icon(&painter, stop.shrink(10.0), Icon::Stop, if sresp.hovered() { p.muted } else { p.ink });
        if sresp.on_hover_text("Stop (Space)").clicked() {
            self.engine.stop();
        }
        let play = Rect::from_center_size(pos2(mid, cy), vec2(116.0, 54.0));
        let presp = pill(ui, play, p, true, "Play / stop (Space)", |pt, r, _| {
            paint_icon(pt, Rect::from_center_size(r.center(), vec2(26.0, 26.0)), Icon::Play, if s.playing { p.accent } else { p.on_ink });
        });
        if presp.clicked() {
            self.engine.toggle_play();
        }
        let rec = Rect::from_center_size(pos2(mid + 118.0, cy), vec2(40.0, 40.0));
        let rresp = ui.interact(rec, ui.scope_id().with("rec"), Sense::click());
        painter.circle_filled(rec.center(), 11.0, if s.recording { p.accent } else { p.rec_off });
        if rresp.hovered() {
            painter.circle_stroke(rec.center(), 15.0, Stroke::new(1.0, p.line));
        }
        if rresp.on_hover_text("REC: every played note becomes a step (M)").clicked() {
            self.engine.execute(Command::new(CommandName::RECORD_TOGGLE));
        }
        let sep = |x: f32| painter.vline(x, (cy - 22.0)..=(cy + 22.0), Stroke::new(1.5, p.line));
        sep(mid + 160.0);

        // BPM: drag, scroll, or double-click to type.
        let bpm_rect = Rect::from_center_size(pos2(mid + 250.0, cy), vec2(150.0, 44.0));
        if let Some(text) = &mut self.bpm_edit {
            let edit = ui.put(bpm_rect.shrink2(vec2(20.0, 6.0)), egui::TextEdit::singleline(text).font(mono(22.0)).align(Align2::CENTER_CENTER));
            edit.request_focus();
            if edit.lost_focus() || ui.input(|i| i.key_pressed(Key::Enter)) {
                if let Ok(v) = text.trim().parse::<f64>() {
                    self.engine.execute(Command::with(CommandName::SET_BPM, v));
                }
                self.bpm_edit = None;
            }
        } else {
            let resp = ui.interact(bpm_rect, ui.scope_id().with("bpm"), Sense::click_and_drag());
            let galley_rect = painter.text(pos2(bpm_rect.center().x + 18.0, cy), Align2::RIGHT_CENTER, format!("{:.1}", s.bpm), mono(28.0), p.ink);
            painter.text(pos2(galley_rect.right() + 6.0, cy + 5.0), Align2::LEFT_CENTER, "BPM", sans(14.0), p.muted);
            if resp.dragged() {
                let fine = ui.input(|i| i.modifiers.shift);
                let d = -resp.drag_delta().y + resp.drag_delta().x;
                self.engine.edit(|st| st.set_bpm(st.bpm + f64::from(d) * if fine { 0.05 } else { 0.25 }));
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                let sc = ui.input(|i| i.smooth_scroll_delta.y);
                if sc != 0.0 {
                    self.engine.edit(|st| st.set_bpm((st.bpm + f64::from(sc) * 0.05 * 10.0).round() / 10.0));
                }
            }
            if resp.double_clicked() {
                self.bpm_edit = Some(format!("{:.1}", s.bpm));
            }
            resp.on_hover_text("Tempo: drag or scroll (Shift = fine), double-click to type, , / . keys");
        }
        sep(mid + 340.0);
        let div = Rect::from_center_size(pos2(mid + 395.0, cy), vec2(80.0, 44.0));
        let dresp = ui.interact(div, ui.scope_id().with("div"), Sense::click());
        painter.text(div.center(), Align2::CENTER_CENTER, format!("1/{}", s.step_division), mono(26.0), if dresp.hovered() { p.muted } else { p.ink });
        if dresp.on_hover_text("Step division (PageUp / PageDown)").clicked() {
            self.engine.execute(Command::new(CommandName::STEP_DIVISION_UP));
        }
        // Gate as a tiny pulse glyph + percentage.
        let gate = Rect::from_center_size(pos2(mid + 480.0, cy), vec2(74.0, 44.0));
        let gresp = ui.interact(gate, ui.scope_id().with("gate"), Sense::click_and_drag());
        let gw = 26.0;
        let gx = gate.left() + 4.0;
        let base = cy + 8.0;
        let pulse_w = gw * s.default_gate as f32;
        painter.add(egui::Shape::line(
            vec![pos2(gx, base), pos2(gx, base - 16.0), pos2(gx + pulse_w, base - 16.0), pos2(gx + pulse_w, base), pos2(gx + gw, base)],
            Stroke::new(2.0, p.ink),
        ));
        painter.text(pos2(gx + gw + 6.0, cy), Align2::LEFT_CENTER, format!("{:.0}%", s.default_gate * 100.0), mono(15.0), p.muted);
        if gresp.dragged() {
            let d = f64::from(-gresp.drag_delta().y + gresp.drag_delta().x) * 0.004;
            self.engine.edit(|st| st.set_default_gate((st.default_gate + d).clamp(0.05, 1.0), true));
        }
        gresp.on_hover_text("Gate length of every step: drag ([ / ] keys)");

        // Right: patterns, theme, settings.
        let right = rect.right() - 30.0;
        let set = Rect::from_center_size(pos2(right - 20.0, cy), vec2(44.0, 44.0));
        let set_resp = ui.interact(set, ui.scope_id().with("settings"), Sense::click());
        paint_icon(&painter, set.shrink(11.0), Icon::Settings, if set_resp.hovered() { p.muted } else { p.ink });
        if set_resp.on_hover_text("MIDI ports, synth profile, MIDI learn").clicked() {
            self.ports.refresh();
            self.settings_open = true;
        }
        let pat = Rect::from_center_size(pos2(right - 90.0, cy), vec2(44.0, 44.0));
        let pat_resp = ui.interact(pat, ui.scope_id().with("patterns"), Sense::click());
        paint_icon(&painter, pat.shrink(11.0), Icon::Grid, if pat_resp.hovered() { p.muted } else { p.ink });
        if pat_resp.on_hover_text("Patterns (Shift+S / Shift+L)").clicked() {
            self.open_patterns(false);
        }
        let moon = Rect::from_center_size(pos2(right - 155.0, cy), vec2(36.0, 36.0));
        let moon_resp = ui.interact(moon, ui.scope_id().with("theme"), Sense::click());
        paint_icon(&painter, moon.shrink(9.0), Icon::Moon, if moon_resp.hovered() { p.muted } else { p.ink });
        if moon_resp.on_hover_text("Light / dark").clicked() {
            self.dark = !self.dark;
            theme::apply_visuals(ui.ctx(), self.pal(), self.dark);
        }
    }

    // --- track list ---------------------------------------------------------------------

    fn track_list(&mut self, ui: &mut Ui, rect: Rect, s: &AppState) {
        let p = self.pal();
        let painter = ui.painter().clone();
        heading(&painter, pos2(rect.left() + 26.0, rect.top() + 4.0), "Track", p);
        let mut y = rect.top() + 38.0;
        let row_h = 52.0;
        let rows: Vec<(Sel, String, String, bool)> = std::iter::once((
            Sel::Melodic,
            "melodic".to_string(),
            format!("ch {}", s.output_channel),
            s.playing && s.sequence.get(s.playhead).is_some_and(|st| st.note.is_some()),
        ))
        .chain(s.drums.lanes.iter().enumerate().map(|(i, l)| {
            (Sel::Lane(i), l.name.to_lowercase(), note_name(l.note), s.playing && l.steps[s.drum_playhead].active)
        }))
        .collect();
        for (sel, name, detail, hot) in rows {
            let r = Rect::from_min_size(pos2(rect.left() + 8.0, y), vec2(rect.width() - 16.0, row_h - 6.0));
            let resp = ui.interact(r, ui.scope_id().with(("track", name.as_str(), y as i32)), Sense::click());
            let active = self.sel == sel;
            if active {
                painter.rect_filled(r, 12.0, p.ink);
            } else if resp.hovered() {
                painter.rect_filled(r, 12.0, p.well);
            }
            let fg = if active { p.on_ink } else { p.ink };
            paint_icon(&painter, Rect::from_center_size(pos2(r.left() + 26.0, r.center().y), vec2(18.0, 18.0)), Icon::Dots, if active { p.on_ink } else { p.muted });
            painter.text(pos2(r.left() + 52.0, r.center().y), Align2::LEFT_CENTER, &name, sans(18.0), fg);
            if sel == Sel::Melodic {
                paint_icon(&painter, Rect::from_center_size(pos2(r.right() - 26.0, r.center().y), vec2(24.0, 20.0)), Icon::Wave, fg);
            } else {
                painter.text(pos2(r.right() - 14.0, r.center().y), Align2::RIGHT_CENTER, &detail, mono(12.0), if active { p.on_ink } else { p.muted });
            }
            if hot {
                painter.circle_filled(pos2(r.left() + 10.0, r.center().y), 3.0, p.accent);
            }
            if resp.clicked() {
                self.select(sel);
            }
            if let Sel::Lane(i) = sel {
                resp.context_menu(|ui| {
                    if ui.button("Clear lane").clicked() {
                        self.engine.edit(|st| {
                            if let Some(l) = st.drums.lanes.get_mut(i) {
                                l.steps = Default::default();
                            }
                        });
                        ui.close();
                    }
                    if s.drums.lanes.len() > 1 && ui.button("Remove lane").clicked() {
                        self.engine.edit(|st| {
                            if i < st.drums.lanes.len() {
                                st.drums.lanes.remove(i);
                                st.drums.cursor_lane = st.drums.cursor_lane.min(st.drums.lanes.len() - 1);
                            }
                        });
                        self.sel = Sel::Lane(i.saturating_sub(1));
                        ui.close();
                    }
                });
            }
            y += row_h;
        }
        let add = Rect::from_min_size(pos2(rect.left() + 12.0, y + 6.0), vec2(64.0, 48.0));
        let add_resp = pill(ui, add, p, false, "Add a drum lane", |pt, r, c| {
            paint_icon(pt, Rect::from_center_size(r.center(), vec2(18.0, 18.0)), Icon::Plus, c);
        });
        if add_resp.clicked() {
            let used: Vec<u8> = s.drums.lanes.iter().map(|l| l.note).collect();
            if let Some(&(name, note)) = EXTRA_LANES.iter().find(|(_, n)| !used.contains(n)) {
                let index = s.drums.lanes.len();
                self.engine.edit(|st| st.drums.lanes.push(DrumLane::new(name, note)));
                self.select(Sel::Lane(index));
            }
        }
    }

    // --- step area ------------------------------------------------------------------------

    fn step_area(&mut self, ui: &mut Ui, rect: Rect, s: &AppState) {
        let p = self.pal();
        let painter = ui.painter().clone();
        let melodic = self.sel == Sel::Melodic;
        let len = s.sequence.len();
        let pages = if melodic { len / STEPS_PER_PAGE + 1 } else { 1 };
        if melodic {
            if s.playing && self.follow {
                self.page = s.playhead / STEPS_PER_PAGE;
            } else if s.cursor != self.last_cursor {
                self.page = s.cursor / STEPS_PER_PAGE;
            }
            self.last_cursor = s.cursor;
            self.page = self.page.min(pages - 1);
        } else {
            self.page = 0;
        }
        let first = self.page * STEPS_PER_PAGE;

        let grid_left = rect.left() + GUTTER;
        let loop_w = 96.0;
        let grid_right = rect.right() - 8.0;
        let cell_gap = 8.0;
        let cell_w = ((grid_right - grid_left) - cell_gap * (STEPS_PER_PAGE as f32 - 1.0)) / STEPS_PER_PAGE as f32;
        let cell_x = |col: usize| grid_left + col as f32 * (cell_w + cell_gap);

        // Ruler.
        let ruler_y = rect.top() + 22.0;
        for col in 0..STEPS_PER_PAGE {
            let x = cell_x(col) + 2.0;
            let beat = col % 4 == 0;
            if beat {
                let n = (first + col) / 4 + 1;
                painter.text(pos2(x, ruler_y - 4.0), Align2::LEFT_BOTTOM, n.to_string(), sans(17.0), p.ink);
            }
            let h = if beat { 14.0 } else { 9.0 };
            painter.vline(x, (ruler_y + 18.0 - h)..=(ruler_y + 18.0), Stroke::new(if beat { 2.0 } else { 1.2 }, p.ink.gamma_multiply(if beat { 1.0 } else { 0.55 })));
        }
        // Loop span selector (top right).
        let loop_rect = Rect::from_min_size(pos2(grid_right - loop_w, rect.top() - 6.0), vec2(loop_w, 44.0));
        let loop_label = if melodic { s.loop_length.map_or("FULL".into(), |l| l.to_string()) } else { "16".into() };
        let lresp = pill(ui, loop_rect, p, false, "Loop span: plays the first N steps without deleting the rest", |pt, r, c| {
            pt.text(pos2(r.center().x - 8.0, r.center().y), Align2::CENTER_CENTER, &loop_label, sans(18.0), c);
            paint_icon(pt, Rect::from_center_size(pos2(r.right() - 20.0, r.center().y), vec2(14.0, 14.0)), Icon::ChevronDown, c);
        });
        if melodic {
            lresp.context_menu(|ui| {
                for l in LOOP_LENGTHS {
                    let label = l.map_or("FULL".into(), |n| n.to_string());
                    if ui.selectable_label(s.loop_length == l, label).clicked() {
                        self.engine.edit(|st| {
                            let _ = st.set_loop_length(l);
                        });
                        ui.close();
                    }
                }
            });
            if lresp.clicked() {
                self.engine.edit(|st| st.cycle_loop_length(1));
            }
            // Page dots.
            if pages > 1 {
                for pg in 0..pages {
                    let c = pos2(loop_rect.left() - 20.0 - (pages - 1 - pg) as f32 * 16.0, loop_rect.center().y);
                    let r = Rect::from_center_size(c, vec2(14.0, 14.0));
                    let resp = ui.interact(r, ui.scope_id().with(("page", pg)), Sense::click());
                    painter.circle_filled(c, if pg == self.page { 5.0 } else { 3.5 }, if pg == self.page { p.ink } else { p.faint });
                    if resp.on_hover_text(format!("Steps {}–{}", pg * 16 + 1, pg * 16 + 16)).clicked() {
                        self.page = pg;
                        self.follow = false;
                    }
                }
            }
        }

        // Cells.
        let cells_top = ruler_y + 34.0;
        let cell_h = (cell_w * 1.05).clamp(48.0, 96.0);
        let row = Rect::from_min_max(pos2(grid_left, cells_top), pos2(grid_right, cells_top + cell_h));
        let vel_top = row.bottom() + 22.0;
        let vel_h = (rect.bottom() - vel_top - 8.0).clamp(80.0, 150.0);
        let vel_row = Rect::from_min_max(pos2(grid_left, vel_top), pos2(grid_right, vel_top + vel_h));
        painter.text(pos2(rect.left() + 6.0, vel_row.center().y), Align2::LEFT_CENTER, "VEL", sans(14.0), p.muted);

        // Audition / commit dot in the gutter.
        let dot = pos2(rect.left() + 20.0, row.center().y);
        let pending: Option<String> = if melodic {
            s.last_note_played.map(|(n, _)| note_name(n))
        } else if !s.drums.audition.is_empty() {
            Some(s.drums.audition.keys().filter_map(|&i| s.drums.lanes.get(i)).map(|l| l.name.chars().next().unwrap_or('?')).collect())
        } else {
            None
        };
        let dot_rect = Rect::from_center_size(dot, vec2(40.0, 40.0));
        let dresp = ui.interact(dot_rect, ui.scope_id().with("commitdot"), Sense::click());
        let armed = pending.is_some();
        painter.circle_filled(dot, 8.0, if s.recording { p.accent } else if armed { p.ink } else { p.faint });
        if let Some(text) = &pending {
            painter.text(pos2(dot.x, dot.y + 22.0), Align2::CENTER_TOP, text, mono(12.0), p.muted);
        }
        if dresp.on_hover_text("Commit the auditioned note (Enter)").clicked() {
            self.engine.commit();
        }

        let row_resp = ui.interact(row, ui.scope_id().with(("cells", melodic)), Sense::click_and_drag());
        let vel_resp = ui.interact(vel_row, ui.scope_id().with(("vel", melodic)), Sense::click_and_drag());
        let col_at = |pos: Pos2| -> Option<usize> {
            let c = ((pos.x - grid_left) / (cell_w + cell_gap)).floor();
            (c >= 0.0 && (c as usize) < STEPS_PER_PAGE).then_some(c as usize)
        };

        if melodic {
            self.melodic_cells(ui, &painter, s, first, row, cell_w, &cell_x, &row_resp, &col_at);
        } else if let Sel::Lane(lane) = self.sel {
            self.drum_cells(ui, &painter, s, lane, row, cell_w, &cell_x, &row_resp, &col_at);
        }

        // Velocity lane.
        for col in 0..STEPS_PER_PAGE {
            let r = Rect::from_min_size(pos2(cell_x(col), vel_row.top()), vec2(cell_w, vel_h));
            painter.rect_filled(r, 8.0, p.well.gamma_multiply(0.55));
            let (vel, on) = if melodic {
                match s.sequence.get(first + col) {
                    Some(st) => (st.velocity, st.note.is_some()),
                    None => continue,
                }
            } else if let Sel::Lane(lane) = self.sel {
                match s.drums.lanes.get(lane) {
                    Some(l) => (l.steps[col].velocity, l.steps[col].active),
                    None => continue,
                }
            } else {
                continue;
            };
            let h = (vel_h - 16.0) * f32::from(vel) / 127.0;
            let x = r.left() + cell_w * 0.35;
            let shown_h = if on { h } else { h.min(16.0) };
            painter.line_segment([pos2(x, r.bottom() - 8.0), pos2(x, r.bottom() - 8.0 - shown_h)], Stroke::new(if on { 4.0 } else { 2.0 }, if on { p.ink } else { p.faint }));
        }
        if vel_resp.dragged() || vel_resp.clicked() {
            if let Some(pos) = vel_resp.interact_pointer_pos() {
                if let Some(col) = col_at(pos) {
                    let v = (((vel_row.bottom() - 8.0 - pos.y) / (vel_h - 16.0)) * 127.0).clamp(1.0, 127.0) as u8;
                    let sel = self.sel;
                    self.engine.edit(|st| match sel {
                        Sel::Melodic => {
                            if let Some(step) = st.sequence.get_mut(first + col) {
                                step.velocity = v;
                            }
                        }
                        Sel::Lane(l) => {
                            if let Some(lane) = st.drums.lanes.get_mut(l) {
                                lane.steps[col].velocity = v;
                            }
                        }
                    });
                }
            }
        }
        if vel_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }

        // Playhead: an orange line through ruler, cells and velocity.
        if s.playing {
            let pos = if melodic { s.playhead } else { s.drum_playhead };
            if melodic && (pos < first || pos >= first + STEPS_PER_PAGE) {
                return;
            }
            let x = cell_x(pos - first) - cell_gap * 0.5 + 1.0;
            painter.vline(x, (ruler_y + 2.0)..=vel_row.bottom(), Stroke::new(2.5, p.accent));
            painter.circle_filled(pos2(x, ruler_y + 2.0), 4.0, p.accent);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn melodic_cells(
        &mut self,
        ui: &mut Ui,
        painter: &egui::Painter,
        s: &AppState,
        first: usize,
        row: Rect,
        cell_w: f32,
        cell_x: &dyn Fn(usize) -> f32,
        resp: &egui::Response,
        col_at: &dyn Fn(Pos2) -> Option<usize>,
    ) {
        let p = self.pal();
        let len = s.sequence.len();
        let span = s.playback_length();
        for col in 0..STEPS_PER_PAGE {
            let i = first + col;
            let r = Rect::from_min_size(pos2(cell_x(col), row.top()), vec2(cell_w, row.height()));
            let hovered = resp.hover_pos().is_some_and(|h| r.contains(h));
            if let Some(step) = s.sequence.get(i) {
                let dim = if i >= span { 0.35 } else { 1.0 };
                match s.playback_note(step).filter(|_| step.note.is_some()) {
                    Some(n) => {
                        painter.rect_filled(r, 10.0, p.ink.gamma_multiply(dim));
                        painter.text(r.center(), Align2::CENTER_CENTER, note_name(n), mono((cell_w * 0.3).clamp(11.0, 16.0)), p.on_ink);
                        if step.tie && i + 1 < len && col + 1 < STEPS_PER_PAGE {
                            let bar = Rect::from_min_max(pos2(r.right() - 2.0, r.center().y - 5.0), pos2(cell_x(col + 1) + 2.0, r.center().y + 5.0));
                            painter.rect_filled(bar, 0.0, p.ink.gamma_multiply(dim));
                        }
                    }
                    None => {
                        painter.rect_filled(r, 10.0, if hovered { p.well_hover } else { p.well });
                        painter.hline((r.center().x - 8.0)..=(r.center().x + 8.0), r.center().y, Stroke::new(2.0, p.muted.gamma_multiply(dim)));
                    }
                }
                if i == s.cursor {
                    painter.rect_stroke(r.expand(3.0), 12.0, Stroke::new(2.0, p.accent), StrokeKind::Outside);
                }
            } else if i == len {
                painter.rect_stroke(r.shrink(1.0), 10.0, Stroke::new(1.5, p.faint), StrokeKind::Inside);
                paint_icon(painter, Rect::from_center_size(r.center(), vec2(14.0, 14.0)), Icon::Plus, if hovered { p.ink } else { p.faint });
            } else {
                painter.rect_filled(r, 10.0, p.well.gamma_multiply(0.45));
            }
        }
        // Click selects; vertical drag or scroll changes pitch; click on "+" commits.
        if let Some(pos) = resp.interact_pointer_pos().or(resp.hover_pos()) {
            let Some(col) = col_at(pos) else { return };
            let i = first + col;
            if resp.clicked() {
                if i < len {
                    self.engine.edit(|st| st.cursor = i);
                } else if i == len {
                    self.engine.commit();
                }
            }
            if resp.drag_started() && i < len {
                self.note_drag = 0.0;
                self.engine.edit(|st| st.cursor = i);
            }
            if resp.dragged() && s.cursor < len {
                self.note_drag -= resp.drag_delta().y / 9.0;
                let whole = self.note_drag.trunc();
                if whole != 0.0 {
                    self.note_drag -= whole;
                    self.engine.edit(|st| {
                        st.adjust_cursor_note(whole as i32);
                    });
                }
            }
            if resp.hovered() && i < len {
                let sc = ui.input(|inp| inp.smooth_scroll_delta.y);
                if sc.abs() > 0.5 {
                    self.engine.edit(|st| {
                        st.cursor = i;
                        st.adjust_cursor_note(if sc > 0.0 { 1 } else { -1 });
                    });
                }
            }
            if i < len {
                resp.context_menu(|ui| {
                    if ui.button("Replace with auditioned note (E)").clicked() {
                        self.engine.edit(|st| {
                            st.cursor = i;
                            st.replace_cursor_with_last_note();
                        });
                        ui.close();
                    }
                    if ui.button("Make rest").clicked() {
                        self.engine.edit(|st| {
                            if let Some(step) = st.sequence.get_mut(i) {
                                step.note = None;
                                step.tie = false;
                            }
                        });
                        ui.close();
                    }
                    if ui.button("Toggle tie to next").clicked() {
                        self.engine.edit(|st| {
                            if let Some(step) = st.sequence.get_mut(i) {
                                step.tie = !step.tie;
                            }
                        });
                        ui.close();
                    }
                    if ui.button("Delete step (X)").clicked() {
                        self.engine.edit(|st| {
                            st.cursor = i;
                            st.delete_cursor();
                        });
                        ui.close();
                    }
                });
            }
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn drum_cells(
        &mut self,
        ui: &mut Ui,
        painter: &egui::Painter,
        s: &AppState,
        lane: usize,
        row: Rect,
        cell_w: f32,
        cell_x: &dyn Fn(usize) -> f32,
        resp: &egui::Response,
        col_at: &dyn Fn(Pos2) -> Option<usize>,
    ) {
        let p = self.pal();
        let Some(l) = s.drums.lanes.get(lane) else { return };
        for col in 0..DRUM_STEPS {
            let r = Rect::from_min_size(pos2(cell_x(col), row.top()), vec2(cell_w, row.height()));
            let hovered = resp.hover_pos().is_some_and(|h| r.contains(h));
            let on = l.steps[col].active;
            painter.rect_filled(r, 10.0, if on { p.ink } else if hovered { p.well_hover } else { p.well });
            if s.drums.cursor_step == col && s.drums.cursor_lane == lane && ui.ctx().memory(|m| m.focused().is_none()) {
                painter.rect_stroke(r.expand(3.0), 12.0, Stroke::new(2.0, p.accent.gamma_multiply(0.7)), StrokeKind::Outside);
            }
        }
        // Paint mode: the first cell decides on/off for the whole drag.
        let paint_id = ui.scope_id().with("drum_paint");
        if let Some(pos) = resp.interact_pointer_pos() {
            if let Some(col) = col_at(pos) {
                if resp.drag_started() || resp.clicked() {
                    let target = !l.steps[col].active;
                    ui.data_mut(|d| d.insert_temp(paint_id, target));
                }
                let target: Option<bool> = ui.data(|d| d.get_temp(paint_id));
                if (resp.dragged() || resp.clicked() || resp.drag_started()) && let Some(target) = target {
                    self.engine.edit(|st| {
                        st.drums.cursor_step = col;
                        if let Some(l) = st.drums.lanes.get_mut(lane) {
                            l.steps[col].active = target;
                        }
                    });
                }
            }
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }

    // --- tools row ------------------------------------------------------------------------

    fn tabs_and_tools(&mut self, ui: &mut Ui, rect: Rect, s: &AppState) {
        let p = self.pal();
        let group = Rect::from_min_size(rect.min, vec2(304.0, 52.0));
        ui.painter().rect_filled(group, 26.0, p.well);
        let step_r = Rect::from_min_size(group.min, vec2(150.0, 52.0));
        let synth_r = Rect::from_min_size(pos2(group.left() + 152.0, group.top()), vec2(152.0, 52.0));
        if text_pill(ui, step_r, p, self.tab == Tab::Step, "step", 20.0).clicked() {
            self.tab = Tab::Step;
        }
        if text_pill(ui, synth_r, p, self.tab == Tab::Synth, "synth", 20.0).clicked() {
            self.tab = Tab::Synth;
        }
        if self.tab != Tab::Step {
            return;
        }
        let melodic = self.sel == Sel::Melodic;
        let tools: Vec<(Icon, &str, Box<dyn Fn(&Engine)>)> = if melodic {
            vec![
                (Icon::Commit, "Commit auditioned note (Enter)", Box::new(|e: &Engine| {
                    e.commit();
                })),
                (Icon::Rest, "Rest (R)", Box::new(|e: &Engine| e.execute(Command::new(CommandName::REST)))),
                (Icon::Hold, "Hold: tie the last note one more step (H)", Box::new(|e: &Engine| e.execute(Command::new(CommandName::HOLD)))),
                (Icon::Undo, "Undo last step (Backspace)", Box::new(|e: &Engine| e.execute(Command::new(CommandName::UNDO)))),
                (Icon::Root, "Set root: play a key next (T)", Box::new(|e: &Engine| e.execute(Command::new(CommandName::ARM_ROOT_CAPTURE)))),
                (Icon::Follow, "Follow playhead", Box::new(|_: &Engine| {})),
                (Icon::Clear, "Clear sequence (C)", Box::new(|e: &Engine| e.execute(Command::new(CommandName::CLEAR)))),
            ]
        } else {
            vec![
                (Icon::Commit, "Commit auditioned hits (Enter)", Box::new(|e: &Engine| {
                    e.commit();
                })),
                (Icon::Rest, "Skip step (R)", Box::new(|e: &Engine| e.execute(Command::new(CommandName::REST)))),
                (Icon::Clear, "Clear all drums (C)", Box::new(|e: &Engine| e.edit(|s| s.drums.clear()))),
            ]
        };
        let w = 76.0;
        let gap = 6.0;
        let mut x = rect.right() - 8.0 - tools.len() as f32 * (w + gap) + gap;
        for (icon, tip, action) in tools {
            let r = Rect::from_min_size(pos2(x, rect.top()), vec2(w, 52.0));
            let active = (icon == Icon::Root && s.root_capture_armed) || (icon == Icon::Follow && self.follow);
            if icon_pill(ui, r, p, active, icon, tip).clicked() {
                if icon == Icon::Follow {
                    self.follow = !self.follow;
                } else {
                    action(&self.engine);
                }
            }
            x += w + gap;
        }
    }

    // --- synth strip ------------------------------------------------------------------------

    fn track_card(&mut self, ui: &mut Ui, rect: Rect, s: &AppState) {
        let p = self.pal();
        let painter = ui.painter().clone();
        let (title, big, sub) = match self.sel {
            Sel::Melodic => {
                let cursor = s.sequence.get(s.cursor);
                let big = cursor.map_or("—".into(), |st| st.note.map_or("REST".into(), note_name));
                let sub = format!(
                    "step {}/{}   oct {:+}   tr {:+}   ch {}",
                    if s.sequence.is_empty() { 0 } else { s.cursor + 1 },
                    s.sequence.len(),
                    s.octave,
                    s.transpose,
                    s.output_channel
                );
                ("melodic".to_string(), big, sub)
            }
            Sel::Lane(i) => {
                let lane = s.drums.lanes.get(i);
                let hits = lane.map_or(0, |l| l.steps.iter().filter(|x| x.active).count());
                (
                    lane.map_or(String::new(), |l| l.name.to_lowercase()),
                    lane.map_or(String::new(), |l| note_name(l.note)),
                    format!("{hits} hits   ch {}", s.drums.channel),
                )
            }
        };
        paint_icon(&painter, Rect::from_min_size(pos2(rect.left(), rect.top() + 2.0), vec2(30.0, 24.0)), Icon::Wave, p.ink);
        painter.text(pos2(rect.left() + 42.0, rect.top() + 14.0), Align2::LEFT_CENTER, &title, sans(20.0), p.ink);
        let prev = Rect::from_center_size(pos2(rect.left() + 222.0, rect.top() + 14.0), vec2(28.0, 28.0));
        let next = Rect::from_center_size(pos2(rect.left() + 258.0, rect.top() + 14.0), vec2(28.0, 28.0));
        let count = s.drums.lanes.len() + 1;
        let index = match self.sel {
            Sel::Melodic => 0,
            Sel::Lane(i) => i + 1,
        };
        let to_sel = |i: usize| if i == 0 { Sel::Melodic } else { Sel::Lane(i - 1) };
        for (r, icon, delta) in [(prev, Icon::ChevronLeft, count - 1), (next, Icon::ChevronRight, 1)] {
            let resp = ui.interact(r, ui.scope_id().with(("cycle", delta)), Sense::click());
            paint_icon(&painter, r.shrink(5.0), icon, if resp.hovered() { p.muted } else { p.ink });
            if resp.clicked() {
                self.select(to_sel((index + delta) % count));
            }
        }
        heading(&painter, pos2(rect.left(), rect.top() + 46.0), if self.sel == Sel::Melodic { "Selected step" } else { "Lane note" }, p);
        let screen = Rect::from_min_max(pos2(rect.left(), rect.top() + 70.0), pos2(rect.right(), rect.bottom()));
        painter.rect_filled(screen, 12.0, p.well);
        painter.text(pos2(screen.left() + 22.0, screen.center().y - 10.0), Align2::LEFT_CENTER, &big, mono(48.0), p.ink);
        painter.text(pos2(screen.left() + 24.0, screen.bottom() - 18.0), Align2::LEFT_CENTER, &sub, mono(12.0), p.muted);
        // Lane note: scroll/drag to change.
        if let Sel::Lane(i) = self.sel {
            let resp = ui.interact(screen, ui.scope_id().with("lane_note"), Sense::click_and_drag());
            let sc = ui.input(|inp| inp.smooth_scroll_delta.y);
            let delta = if resp.hovered() && sc.abs() > 0.5 { sc.signum() as i32 } else if resp.dragged() && resp.drag_delta().y.abs() > 4.0 { -resp.drag_delta().y.signum() as i32 } else { 0 };
            if delta != 0 {
                self.engine.edit(|st| {
                    if let Some(l) = st.drums.lanes.get_mut(i) {
                        l.note = (i32::from(l.note) + delta).clamp(0, 127) as u8;
                    }
                });
            }
            resp.on_hover_text("Scroll or drag to change the lane's MIDI note");
        }
    }

    /// Profile knob groups laid out left to right; returns false if no profile.
    fn synth_groups(&mut self, ui: &mut Ui, rect: Rect, big: bool) -> bool {
        let p = self.pal();
        let painter = ui.painter().clone();
        let pages: Vec<(String, PageKind, Vec<(u8, String, u8, String)>)> = {
            let control = self.engine.control();
            let Some(router) = control.profile.as_ref() else { return false };
            router
                .profile
                .pages
                .iter()
                .enumerate()
                .map(|(pi, page)| {
                    let knobs = page
                        .knobs
                        .iter()
                        .map(|(&k, b)| {
                            let v = router.value(pi, k);
                            (k, b.label.clone(), v, b.display(v))
                        })
                        .collect();
                    (page.name.clone(), page.kind, knobs)
                })
                .collect()
        };
        let knob_w = if big { 118.0 } else { 104.0 };
        let env_w = if big { 380.0 } else { 330.0 };
        let mut x = rect.left();
        let mut y = rect.top();
        for (pi, (name, kind, knobs)) in pages.iter().enumerate() {
            let width = match kind {
                PageKind::Envelope => env_w,
                PageKind::Knobs => (knobs.len().max(1) as f32) * knob_w,
            };
            if x + width > rect.right() + 1.0 && x > rect.left() {
                if !big {
                    break;
                }
                x = rect.left();
                y += 220.0;
                if y + 180.0 > rect.bottom() {
                    break;
                }
            }
            if x > rect.left() {
                painter.vline(x - 18.0, (y)..=(y + 200.0), Stroke::new(1.0, p.line));
            }
            heading(&painter, pos2(x, y), name, p);
            let body_top = y + 34.0;
            match kind {
                PageKind::Envelope => {
                    let env = Rect::from_min_size(pos2(x + 8.0, body_top + 4.0), vec2(width - 30.0, 100.0));
                    let mut adsr = [0u8; 4];
                    for (k, _, v, _) in knobs {
                        if (1..=4).contains(k) {
                            adsr[(*k - 1) as usize] = *v;
                        }
                    }
                    paint_envelope(&painter, env, p, adsr);
                    // Values below; drag each column to edit.
                    let col_w = env.width() / 4.0;
                    for (k, label, v, display) in knobs.iter().filter(|(k, ..)| (1..=4).contains(k)) {
                        let cx = env.left() + col_w * (f32::from(*k) - 0.5);
                        let r = Rect::from_center_size(pos2(cx, env.bottom() + 40.0), vec2(col_w, 50.0));
                        let resp = ui.interact(r, ui.scope_id().with(("env", pi, *k)), Sense::drag());
                        painter.text(pos2(cx, r.top() + 10.0), Align2::CENTER_CENTER, short_label(label), sans(14.0), p.ink);
                        painter.text(pos2(cx, r.top() + 32.0), Align2::CENTER_CENTER, display, sans(14.0), p.muted);
                        if resp.dragged() {
                            let nv = (f32::from(*v) - resp.drag_delta().y * 0.6 + resp.drag_delta().x * 0.3).clamp(0.0, 127.0) as u8;
                            self.engine.set_knob(pi, *k, nv);
                        }
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                        }
                    }
                }
                PageKind::Knobs => {
                    if knobs.is_empty() {
                        painter.text(pos2(x, body_top + 30.0), Align2::LEFT_CENTER, "no knobs", sans(13.0), p.faint);
                    }
                    for (n, (k, label, v, display)) in knobs.iter().enumerate() {
                        let r = Rect::from_min_size(pos2(x + n as f32 * knob_w, body_top + 6.0), vec2(knob_w - 18.0, knob_w + 30.0));
                        let mut value = *v;
                        let default = self.engine.control().profile.as_ref().and_then(|r| r.profile.pages[pi].knobs.get(k).map(|b| b.default)).unwrap_or(0);
                        let changed = Knob { value: &mut value, default, label, display: display.clone(), bold_arc: n == 0 && name.to_uppercase().contains("FILTER") }.show(ui, r, p);
                        if changed {
                            self.engine.set_knob(pi, *k, value);
                        }
                    }
                }
            }
            x += width + 36.0;
        }
        true
    }

    fn bottom_strip(&mut self, ui: &mut Ui, rect: Rect, s: &AppState) {
        let p = self.pal();
        ui.painter().hline(rect.left()..=rect.right(), rect.top(), Stroke::new(1.5, p.line));
        let card = Rect::from_min_max(pos2(rect.left(), rect.top() + 26.0), pos2(rect.left() + 300.0, rect.bottom() - 10.0));
        self.track_card(ui, card, s);
        let groups = Rect::from_min_max(pos2(card.right() + 56.0, rect.top() + 26.0), pos2(rect.right(), rect.bottom()));
        if !self.synth_groups(ui, groups, false) {
            self.no_profile_hint(ui, groups);
        }
    }

    fn no_profile_hint(&mut self, ui: &mut Ui, rect: Rect) {
        let p = self.pal();
        let painter = ui.painter().clone();
        heading(&painter, rect.min, "Synth", p);
        painter.text(
            pos2(rect.left(), rect.top() + 46.0),
            Align2::LEFT_CENTER,
            "Load a synth profile to turn these into knobs that send MIDI CC to your synth.",
            sans(15.0),
            p.muted,
        );
        let b = Rect::from_min_size(pos2(rect.left(), rect.top() + 76.0), vec2(220.0, 44.0));
        if text_pill(ui, b, p, false, "Load profile…", 16.0).clicked() {
            self.pick_profile();
        }
    }

    fn pick_profile(&mut self) {
        let start = self.profile_path.as_ref().and_then(|p| p.parent().map(PathBuf::from)).unwrap_or_else(|| PathBuf::from("profiles"));
        if let Some(path) = rfd::FileDialog::new().add_filter("Synth profile", &["yaml", "yml"]).set_directory(start).pick_file() {
            self.load_profile(&path);
        }
    }

    pub fn load_profile(&mut self, path: &std::path::Path) {
        match ministep_core::profiles::load_profile(path).map_err(|e| e.0).and_then(|prof| self.ports.router_for(prof, None)) {
            Ok(router) => {
                self.engine.set_profile(Some(router));
                self.profile_path = Some(path.to_path_buf());
                self.engine.edit(|s| s.status_message = format!("Loaded synth profile {}.", path.display()));
            }
            Err(e) => self.engine.edit(|s| s.status_message = format!("Cannot load profile: {e}")),
        }
    }

    // --- synth tab ------------------------------------------------------------------------

    fn synth_tab(&mut self, ui: &mut Ui, rect: Rect) {
        let p = self.pal();
        let painter = ui.painter().clone();
        let name = self.engine.control().profile.as_ref().map(|r| r.profile.name.clone());
        let head = Rect::from_min_size(rect.min, vec2(rect.width(), 40.0));
        painter.text(pos2(head.left(), head.center().y), Align2::LEFT_CENTER, name.as_deref().unwrap_or("no profile"), sans(22.0), p.ink);
        let out = self.engine.control().profile.as_ref().map(|r| r.output.lock().name.clone()).unwrap_or_default();
        if !out.is_empty() {
            painter.text(pos2(head.left() + 260.0, head.center().y), Align2::LEFT_CENTER, format!("→ {out}"), mono(13.0), p.muted);
        }
        let b = Rect::from_min_size(pos2(head.right() - 190.0, head.top()), vec2(190.0, 40.0));
        if text_pill(ui, b, p, false, "Load profile…", 15.0).clicked() {
            self.pick_profile();
        }
        let body = Rect::from_min_max(pos2(rect.left(), head.bottom() + 30.0), rect.max);
        if !self.synth_groups(ui, body, true) {
            self.no_profile_hint(ui, body);
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

    // --- modals -----------------------------------------------------------------------------

    fn open_patterns(&mut self, save: bool) {
        let kind = self.engine.pattern_kind();
        self.patterns = Some(PatternDialog { save, kind, name: String::new(), names: self.engine.patterns().list_names(kind), focus: true });
    }

    fn patterns_modal(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.patterns else { return };
        let p = if self.dark { &DARK } else { &LIGHT };
        let mut close = false;
        let mut action: Option<(bool, String)> = None;
        let modal = egui::Modal::new(egui::Id::unique("patterns")).frame(egui::Frame::popup(&ctx.global_style()).fill(p.bg).inner_margin(24.0).corner_radius(18.0)).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.horizontal(|ui| {
                for (save, label) in [(false, "load"), (true, "save")] {
                    if ui.selectable_label(dlg.save == save, egui::RichText::new(label).size(17.0)).clicked() {
                        dlg.save = save;
                    }
                }
                ui.add_space(12.0);
                for kind in PatternKind::ALL {
                    if ui.selectable_label(dlg.kind == kind, egui::RichText::new(kind.as_str()).size(14.0)).clicked() {
                        dlg.kind = kind;
                        dlg.names = self.engine.patterns().list_names(kind);
                    }
                }
            });
            ui.add_space(10.0);
            let edit = ui.add(egui::TextEdit::singleline(&mut dlg.name).hint_text(if dlg.save { "name" } else { "filter" }).font(sans(18.0)).desired_width(f32::INFINITY));
            if dlg.focus {
                edit.request_focus();
                dlg.focus = false;
            }
            ui.add_space(8.0);
            let needle = dlg.name.to_lowercase();
            let matches: Vec<&String> = dlg.names.iter().filter(|n| fuzzy(&needle, &n.to_lowercase())).collect();
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                for n in &matches {
                    let resp = ui.selectable_label(false, egui::RichText::new(n.as_str()).font(mono(15.0)));
                    if resp.clicked() {
                        action = Some((dlg.save, (*n).clone()));
                    }
                }
                if matches.is_empty() {
                    ui.label(egui::RichText::new(if dlg.save { "Enter saves a new pattern." } else { "No patterns." }).color(p.muted));
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let go = ui.add(egui::Button::new(egui::RichText::new(if dlg.save { "Save" } else { "Load" }).size(16.0)).fill(p.ink).corner_radius(16.0).min_size(vec2(90.0, 34.0)));
                let go = go.clicked() || ui.input(|i| i.key_pressed(Key::Enter));
                if go {
                    let name = if !dlg.save && !matches.is_empty() && !matches.iter().any(|m| **m == dlg.name) { matches[0].clone() } else { dlg.name.clone() };
                    if !name.trim().is_empty() {
                        action = Some((dlg.save, name));
                    }
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if modal.should_close() {
            close = true;
        }
        if let Some((save, name)) = action {
            let kind = dlg.kind;
            let ok = if save { self.engine.save_pattern(&name, Some(kind)) } else { self.engine.load_pattern(&name, Some(kind)) };
            if ok {
                self.title = name.trim().to_string();
            }
            close = true;
        }
        if close {
            self.patterns = None;
        }
    }

    fn settings_modal(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let p = if self.dark { &DARK } else { &LIGHT };
        let s = self.engine.state().clone();
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("settings")).frame(egui::Frame::popup(&ctx.global_style()).fill(p.bg).inner_margin(26.0).corner_radius(18.0)).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(egui::RichText::new("MIDI").size(20.0));
            ui.add_space(8.0);
            egui::Grid::new("ports").num_columns(2).spacing(vec2(16.0, 10.0)).show(ui, |ui| {
                ui.label("Input");
                let current = self.ports.input_name().map(str::to_string);
                let mut choice: Option<Option<String>> = None;
                egui::ComboBox::from_id_salt("in").width(340.0).selected_text(current.clone().unwrap_or("none".into())).show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_none(), "none").clicked() {
                        choice = Some(None);
                    }
                    for name in &self.ports.inputs {
                        if ui.selectable_label(current.as_deref() == Some(name), name).clicked() {
                            choice = Some(Some(name.clone()));
                        }
                    }
                });
                if let Some(c) = choice {
                    if let Err(e) = self.ports.set_input(&self.engine, c.as_deref()) {
                        self.engine.edit(|st| st.status_message = e);
                    }
                }
                ui.end_row();

                for (label, drums) in [("Output", false), ("Drum output", true)] {
                    ui.label(label);
                    let current = if drums { s.selected_drum_output.clone() } else { s.selected_output.clone() };
                    let shown = current.clone().unwrap_or(if drums { "same as output".into() } else { "none".into() });
                    let mut choice: Option<Option<String>> = None;
                    egui::ComboBox::from_id_salt(label).width(340.0).selected_text(shown).show_ui(ui, |ui| {
                        if ui.selectable_label(current.is_none(), if drums { "same as output" } else { "none" }).clicked() {
                            choice = Some(None);
                        }
                        for name in &self.ports.outputs {
                            let same = current.as_deref() == Some(name.as_str()) || (current.as_deref() == Some(crate::ports::VIRTUAL_OUT) && name == crate::ports::VIRTUAL_LABEL);
                            if ui.selectable_label(same, name).clicked() {
                                choice = Some(Some(name.clone()));
                            }
                        }
                    });
                    if let Some(c) = choice {
                        let (m, d) = if drums { (s.selected_output.clone(), c) } else { (c, s.selected_drum_output.clone()) };
                        if let Err(e) = self.ports.set_outputs(&self.engine, m.as_deref(), d.as_deref()) {
                            self.engine.edit(|st| st.status_message = e);
                        }
                    }
                    ui.end_row();
                }
                ui.label("Melodic channel");
                let mut ch = s.output_channel;
                if ui.add(egui::DragValue::new(&mut ch).range(1..=16)).changed() {
                    self.engine.edit(|st| st.output_channel = ch);
                }
                ui.end_row();
                ui.label("Drum channel");
                let mut dch = s.drums.channel;
                if ui.add(egui::DragValue::new(&mut dch).range(1..=16)).changed() {
                    self.engine.edit(|st| st.drums.channel = dch);
                }
                ui.end_row();
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.button("Rescan ports").clicked() {
                    self.ports.refresh();
                }
                if ui.button("Load synth profile…").clicked() {
                    self.pick_profile();
                }
                if self.engine.control().profile.is_some() && ui.button("Remove profile").clicked() {
                    self.engine.set_profile(None);
                }
            });
            if self.engine.control().mapping.is_some() {
                ui.add_space(14.0);
                ui.label(egui::RichText::new("MIDI learn").size(18.0));
                ui.label(egui::RichText::new("Pick a command, then press a pad or turn a control.").color(p.muted));
                ui.horizontal_wrapped(|ui| {
                    for &c in CommandName::ALL {
                        if ui.small_button(c.as_str()).clicked() {
                            self.engine.arm_midi_learn(c);
                            close = true;
                        }
                    }
                });
            }
            ui.add_space(16.0);
            if ui.add(egui::Button::new(egui::RichText::new("Done").size(16.0)).corner_radius(16.0).min_size(vec2(90.0, 34.0))).clicked() {
                close = true;
            }
        });
        if modal.should_close() || close {
            self.settings_open = false;
        }
    }

    // --- keyboard ---------------------------------------------------------------------------

    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.patterns.is_some() || self.settings_open {
            return;
        }
        let e = Arc::clone(&self.engine);
        let pressed: Vec<(Key, egui::Modifiers)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|ev| match ev {
                    egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                    _ => None,
                })
                .collect()
        });
        let drums = self.sel != Sel::Melodic;
        let cmd = |n: CommandName| e.execute(Command::new(n));
        for (key, m) in pressed {
            if m.command {
                continue;
            }
            match key {
                Key::Space => cmd(CommandName::PLAY_STOP),
                Key::Enter => {
                    if drums && e.state().drums.audition.is_empty() {
                        e.edit(|s| {
                            s.drums.toggle_hit();
                        });
                    } else {
                        e.commit();
                    }
                }
                Key::M => cmd(CommandName::RECORD_TOGGLE),
                Key::R => cmd(CommandName::REST),
                Key::H if !drums => cmd(CommandName::HOLD),
                Key::U | Key::Backspace if !drums => cmd(CommandName::UNDO),
                Key::C => {
                    if drums { e.edit(|s| s.drums.clear()) } else { cmd(CommandName::CLEAR) }
                }
                Key::Home => cmd(CommandName::RESTART),
                Key::ArrowLeft => e.edit(|s| if drums { s.drums.move_step(-1) } else { s.move_cursor(-1) }),
                Key::ArrowRight => e.edit(|s| if drums { s.drums.move_step(1) } else { s.move_cursor(1) }),
                Key::ArrowUp | Key::ArrowDown => {
                    let dir = if key == Key::ArrowUp { 1 } else { -1 };
                    if drums {
                        let cur = match self.sel {
                            Sel::Lane(i) => i as i64,
                            Sel::Melodic => 0,
                        };
                        let n = e.state().drums.lanes.len() as i64;
                        if n > 0 {
                            self.select(Sel::Lane((cur - dir).rem_euclid(n) as usize));
                        }
                    } else {
                        let by = if m.shift { 12 } else { 1 };
                        e.edit(|s| {
                            s.adjust_cursor_note(dir as i32 * by);
                        });
                    }
                }
                Key::E if !drums => e.edit(|s| {
                    s.replace_cursor_with_last_note();
                }),
                Key::X | Key::Delete => e.edit(|s| {
                    if drums {
                        s.drums.clear_hit();
                    } else {
                        s.delete_cursor();
                    }
                }),
                Key::Comma => cmd(CommandName::BPM_DOWN),
                Key::Period => cmd(CommandName::BPM_UP),
                Key::PageUp => cmd(CommandName::STEP_DIVISION_UP),
                Key::PageDown => cmd(CommandName::STEP_DIVISION_DOWN),
                Key::OpenBracket => e.edit(|s| s.set_default_gate((s.default_gate - 0.05).max(0.05), true)),
                Key::CloseBracket => e.edit(|s| s.set_default_gate(s.default_gate + 0.05, true)),
                Key::Minus if drums => e.edit(|s| {
                    s.drums.adjust_velocity(-10);
                }),
                Key::Plus | Key::Equals if drums => e.edit(|s| {
                    s.drums.adjust_velocity(10);
                }),
                Key::Escape if drums => e.edit(|s| s.drums.clear_audition()),
                Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 if drums => {
                    let i = match key {
                        Key::Num1 => 0,
                        Key::Num2 => 1,
                        Key::Num3 => 2,
                        _ => 3,
                    };
                    if i < e.state().drums.lanes.len() {
                        self.select(Sel::Lane(i));
                    }
                }
                Key::T if !drums => cmd(CommandName::ARM_ROOT_CAPTURE),
                Key::K => {
                    if e.control().mapping.is_some() {
                        self.ports.refresh();
                        self.settings_open = true;
                    } else {
                        e.edit(|s| s.status_message = "MIDI Learn needs --mapping minilab3.".into());
                    }
                }
                Key::S if m.shift => self.open_patterns(true),
                Key::L if m.shift => self.open_patterns(false),
                Key::S => {
                    let _ = e.save();
                }
                Key::L => {
                    let _ = e.load();
                }
                Key::Tab => {
                    let s = e.state().active_track;
                    self.select(if s == Track::Melodic { Sel::Lane(e.state().drums.cursor_lane) } else { Sel::Melodic });
                }
                _ => {}
            }
        }
    }
}

fn short_label(label: &str) -> String {
    let up = label.to_uppercase();
    match up.as_str() {
        "ATTACK" => "A".into(),
        "DECAY" => "D".into(),
        "SUSTAIN" => "S".into(),
        "RELEASE" => "R".into(),
        _ => up,
    }
}

/// Subsequence match: "egy1" matches "egypt_01".
fn fuzzy(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.keys(&ctx);
        let s = self.engine.state().clone();
        self.sync_selection(&s);
        let p = self.pal();
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, p.bg);
        // Leave room for the macOS traffic lights (full-size content view).
        let area = full.shrink2(vec2(28.0, 0.0));
        let top = Rect::from_min_size(pos2(area.left(), area.top() + 26.0), vec2(area.width(), 76.0));
        self.top_bar(ui, top, &s);

        let status = Rect::from_min_max(pos2(area.left(), full.bottom() - 30.0), pos2(area.right(), full.bottom() - 8.0));
        let left_w = 300.0;
        let body_top = top.bottom() + 28.0;
        let tracks = Rect::from_min_max(pos2(area.left(), body_top + 64.0), pos2(area.left() + left_w, status.top() - 300.0));
        self.track_list(ui, tracks, &s);
        let main = Rect::from_min_max(pos2(area.left() + left_w + 8.0, body_top), pos2(area.right(), status.top()));
        self.tabs_and_tools(ui, Rect::from_min_size(main.min, vec2(main.width(), 52.0)), &s);
        match self.tab {
            Tab::Step => {
                let steps = Rect::from_min_max(pos2(main.left(), main.top() + 84.0), pos2(main.right(), status.top() - 330.0));
                self.step_area(ui, steps, &s);
                let strip_top = status.top() - 300.0;
                let strip = Rect::from_min_max(pos2(area.left(), strip_top), pos2(area.right(), status.top() - 8.0));
                self.bottom_strip(ui, strip, &s);
            }
            Tab::Synth => {
                let body = Rect::from_min_max(pos2(main.left(), main.top() + 84.0), pos2(main.right(), status.top() - 8.0));
                self.synth_tab(ui, body);
            }
        }
        self.status_line(ui, status, &s);
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

