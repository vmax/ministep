//! Transport bar: logo, stop/play/REC, tempo, division, gate, menus.

use super::*;

impl App {
    // --- top bar ----------------------------------------------------------------------

    pub(super) fn top_bar(&mut self, ui: &mut Ui, rect: Rect, s: &AppState) {
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
        // Centre the transport cluster (stop … gate) between the title and the
        // right-hand icons, so narrow windows never overlap.
        let free_left = rect.left() + TITLE_WIDTH;
        let free_right = rect.right() - RIGHT_ICONS_WIDTH;
        let mid = free_left + ((free_right - free_left) - TRANSPORT_CLUSTER_WIDTH).max(0.0) * 0.5 + TRANSPORT_MID_OFFSET;
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
}
