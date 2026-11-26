//! Bottom strip and synth tab: track card, CC profile knobs, drum synth voice.

use super::*;

impl App {
    // --- synth strip ------------------------------------------------------------------------

    pub(super) fn track_card(&mut self, ui: &mut Ui, rect: Rect, s: &AppState) {
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
        let icon = if self.sel == Sel::Melodic { Icon::Wave } else { Icon::Grid };
        paint_icon(&painter, Rect::from_min_size(pos2(rect.left(), rect.top() + 2.0), vec2(30.0, 24.0)), icon, p.ink);
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
    pub(super) fn synth_groups(&mut self, ui: &mut Ui, rect: Rect, big: bool) -> bool {
        let p = self.pal();
        let painter = ui.painter().clone();
        let pages: Vec<PageView> = {
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
        let knob_w = if big { KNOB_WIDTH_LARGE } else { KNOB_WIDTH };
        let env_w = if big { ENVELOPE_WIDTH_LARGE } else { ENVELOPE_WIDTH };
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

    pub(super) fn bottom_strip(&mut self, ui: &mut Ui, rect: Rect, s: &AppState) {
        let p = self.pal();
        ui.painter().hline(rect.left()..=rect.right(), rect.top(), Stroke::new(1.5, p.line));
        let card = Rect::from_min_max(pos2(rect.left(), rect.top() + 26.0), pos2(rect.left() + CARD_WIDTH, rect.bottom() - 10.0));
        self.track_card(ui, card, s);
        let groups = Rect::from_min_max(pos2(card.right() + CARD_TO_PANEL, rect.top() + 26.0), pos2(rect.right(), rect.bottom()));
        if let Sel::Lane(lane) = self.sel {
            self.voice_panel(ui, groups, s, lane);
        } else if !self.synth_groups(ui, groups, false) {
            self.no_profile_hint(ui, groups);
        }
    }

    /// The built-in drum synth voice of one lane: model, waveform, five knobs, routing.
    pub(super) fn voice_panel(&mut self, ui: &mut Ui, rect: Rect, s: &AppState, lane: usize) {
        let p = self.pal();
        let painter = ui.painter().clone();
        let Some(voice) = s.drums.lanes.get(lane).map(|l| l.voice) else { return };
        let set_voice = |engine: &Engine, f: &dyn Fn(&mut VoiceParams)| {
            engine.edit(|st| {
                if let Some(l) = st.drums.lanes.get_mut(lane) {
                    f(&mut l.voice);
                }
            });
        };

        // Header: section name + route switch.
        heading(&painter, rect.min, "Drum synth", p);
        let routes = [(DrumRoute::Synth, "synth"), (DrumRoute::Both, "both"), (DrumRoute::Midi, "midi")];
        let rw = 70.0;
        let rx = rect.right() - routes.len() as f32 * (rw + 4.0);
        for (i, (route, label)) in routes.iter().enumerate() {
            let r = Rect::from_min_size(pos2(rx + i as f32 * (rw + 4.0), rect.top() - 8.0), vec2(rw, 32.0));
            if text_pill(ui, r, p, s.drum_route == *route, label, 14.0).on_hover_text("Where drum hits go: built-in synth, MIDI, or both").clicked() {
                self.engine.set_drum_route(*route);
            }
        }
        if let Some(dev) = &self.audio_device {
            painter.text(pos2(rx - 14.0, rect.top() + 8.0), Align2::RIGHT_CENTER, format!("♪ {dev}"), mono(12.0), p.faint);
        } else if s.drum_route.synth() {
            painter.text(pos2(rx - 14.0, rect.top() + 8.0), Align2::RIGHT_CENTER, "no audio output", mono(12.0), p.accent);
        }

        // Model chips.
        let chip_w = CHIP_WIDTH;
        let chips_top = rect.top() + 34.0;
        for (i, model) in VoiceModel::ALL.iter().enumerate() {
            let col = i % 3;
            let row = i / 3;
            let r = Rect::from_min_size(pos2(rect.left() + col as f32 * (chip_w + 6.0), chips_top + row as f32 * 40.0), vec2(chip_w, CHIP_HEIGHT));
            if text_pill(ui, r, p, voice.model == *model, model.label(), 12.5).clicked() && voice.model != *model {
                let m = *model;
                set_voice(&self.engine, &|v| {
                    let level = v.level;
                    *v = VoiceParams { level, ..VoiceParams::new(m) };
                });
                self.engine.audition_lane(lane, 110);
            }
        }

        // Waveform preview (click to play).
        // Five knobs always fit: the waveform gives up width first, then the knobs shrink.
        let wave_left = rect.left() + 3.0 * (chip_w + 6.0) + 18.0;
        let room = rect.right() - wave_left - 36.0;
        let wave_w = (room - 5.0 * KNOB_WIDTH).clamp(WAVE_MIN_WIDTH, WAVE_WIDTH);
        let knob_w = ((room - wave_w) / 5.0).min(KNOB_WIDTH);
        let wave = Rect::from_min_size(pos2(wave_left, chips_top), vec2(wave_w, 114.0));
        let wresp = ui.interact(wave, ui.scope_id().with(("wave", lane)), Sense::click());
        painter.rect_filled(wave, 12.0, if wresp.hovered() { p.well_hover } else { p.well });
        let peaks = self.peaks(voice, (wave.width() - 24.0) as usize);
        let mid = wave.center().y;
        let h = wave.height() * 0.42;
        for (i, pk) in peaks.iter().enumerate() {
            let x = wave.left() + 12.0 + i as f32;
            painter.vline(x, (mid - pk * h)..=(mid + pk * h), Stroke::new(1.0, p.ink));
        }
        if wresp.on_hover_text("Play this voice").clicked() {
            self.engine.audition_lane(lane, 110);
        }

        // Knobs.
        let kx = wave.right() + 36.0;
        painter.vline(kx - 18.0, (rect.top() + 30.0)..=(rect.top() + 230.0), Stroke::new(1.0, p.line));
        for i in 0..5 {
            let r = Rect::from_min_size(pos2(kx + i as f32 * knob_w, rect.top() + 34.0), vec2(knob_w - 18.0, knob_w + 30.0));
            let mut value = (voice.knob(i) * 127.0).round() as u8;
            let default = (VoiceParams::new(voice.model).knob(i) * 127.0).round() as u8;
            let label = voice.knob_label(i);
            if (Knob { value: &mut value, default, label, display: voice.knob_display(i), bold_arc: i == 1 }).show(ui, r, p) {
                let v = f32::from(value) / 127.0;
                set_voice(&self.engine, &|vp| vp.set_knob(i, v));
            }
            // Releasing a knob plays the result.
            if ui.input(|inp| inp.pointer.any_released()) && r.contains(ui.input(|inp| inp.pointer.interact_pos().unwrap_or_default())) && !s.playing {
                self.engine.audition_lane(lane, 110);
            }
        }
    }

    /// Peak envelope of a rendered hit, one value per column (cached).
    pub(super) fn peaks(&mut self, voice: VoiceParams, columns: usize) -> Vec<f32> {
        if let Some((cached, peaks)) = &self.preview
            && *cached == voice && peaks.len() == columns {
                return peaks.clone();
            }
        let sr = 16_000.0;
        let samples = ministep_drumsynth::render(voice, 110, sr, 1.5);
        let columns = columns.max(1);
        // Fixed time scale (1.5 s across) so decay changes are visible.
        let per = ((1.5 * sr) as usize / columns).max(1);
        let mut peaks: Vec<f32> = (0..columns)
            .map(|c| samples.iter().skip(c * per).take(per).fold(0.0f32, |m, v| m.max(v.abs())))
            .collect();
        let max = peaks.iter().cloned().fold(0.0f32, f32::max).max(1e-6);
        let norm = (1.0 / max).min(4.0);
        for pk in &mut peaks {
            *pk = (*pk * norm).min(1.0).sqrt();
        }
        self.preview = Some((voice, peaks.clone()));
        peaks
    }

    pub(super) fn no_profile_hint(&mut self, ui: &mut Ui, rect: Rect) {
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

    pub(super) fn pick_profile(&mut self) {
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

    pub(super) fn synth_tab(&mut self, ui: &mut Ui, rect: Rect) {
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
