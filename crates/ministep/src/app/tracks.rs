//! Track list: melodic track and drum lanes.

use super::*;

impl App {
    // --- track list ---------------------------------------------------------------------

    pub(super) fn track_list(&mut self, ui: &mut Ui, rect: Rect, s: &AppState) {
        let p = self.pal();
        let painter = ui.painter().clone();
        heading(&painter, pos2(rect.left() + 26.0, rect.top() + 4.0), "Track", p);
        let mut y = rect.top() + 38.0;
        let drums_active = matches!(self.sel, Sel::Lane(_));

        // Melodic track.
        let r = Rect::from_min_size(pos2(rect.left() + 8.0, y), vec2(rect.width() - 16.0, TRACK_ROW_HEIGHT - 6.0));
        let resp = ui.interact(r, ui.scope_id().with("track_melodic"), Sense::click());
        let active = self.sel == Sel::Melodic;
        painter.rect_filled(r, 12.0, if active { p.ink } else if resp.hovered() { p.well } else { Color32::TRANSPARENT });
        let fg = if active { p.on_ink } else { p.ink };
        paint_icon(&painter, Rect::from_center_size(pos2(r.left() + 26.0, r.center().y), vec2(18.0, 18.0)), Icon::Dots, if active { p.on_ink } else { p.muted });
        painter.text(pos2(r.left() + 52.0, r.center().y), Align2::LEFT_CENTER, "melodic", sans(18.0), fg);
        paint_icon(&painter, Rect::from_center_size(pos2(r.right() - 26.0, r.center().y), vec2(24.0, 20.0)), Icon::Wave, fg);
        if s.playing && s.sequence.get(s.playhead).is_some_and(|st| st.note.is_some()) {
            painter.circle_filled(pos2(r.left() + 10.0, r.center().y), 3.0, p.accent);
        }
        if resp.clicked() {
            self.select(Sel::Melodic);
        }
        y += TRACK_ROW_HEIGHT;

        // The drum group: one entry that opens the combined grid.
        let r = Rect::from_min_size(pos2(rect.left() + 8.0, y), vec2(rect.width() - 16.0, TRACK_ROW_HEIGHT - 6.0));
        let resp = ui.interact(r, ui.scope_id().with("track_drums"), Sense::click());
        painter.rect_filled(r, 12.0, if drums_active { p.ink } else if resp.hovered() { p.well } else { Color32::TRANSPARENT });
        let fg = if drums_active { p.on_ink } else { p.ink };
        paint_icon(&painter, Rect::from_center_size(pos2(r.left() + 26.0, r.center().y), vec2(18.0, 18.0)), Icon::Grid, if drums_active { p.on_ink } else { p.muted });
        painter.text(pos2(r.left() + 52.0, r.center().y), Align2::LEFT_CENTER, "drums", sans(18.0), fg);
        painter.text(pos2(r.right() - 14.0, r.center().y), Align2::RIGHT_CENTER, format!("{} lanes", s.drums.lanes.len()), mono(12.0), if drums_active { p.on_ink } else { p.muted });
        if s.playing && s.drums.lanes.iter().any(|l| l.steps[s.drum_playhead].active) {
            painter.circle_filled(pos2(r.left() + 10.0, r.center().y), 3.0, p.accent);
        }
        if resp.clicked() && !drums_active {
            self.select(Sel::Lane(s.drums.cursor_lane));
        }
        y += TRACK_ROW_HEIGHT;

        // Its lanes, indented; they scroll when there are more than fit.
        let row_h = LANE_ROW_HEIGHT;
        let lanes = &s.drums.lanes;
        let capacity = (((rect.bottom() - y - ADD_BUTTON_HEIGHT) / row_h).floor() as usize).max(1);
        let selected = match self.sel {
            Sel::Lane(i) => i,
            Sel::Melodic => self.track_scroll,
        };
        if ui.rect_contains_pointer(rect) {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.5 {
                self.track_scroll = (self.track_scroll as f32 - scroll / row_h).max(0.0).round() as usize;
            }
        }
        if selected < self.track_scroll {
            self.track_scroll = selected;
        } else if selected >= self.track_scroll + capacity && self.track_scroll_follow != selected {
            self.track_scroll = selected + 1 - capacity;
        }
        self.track_scroll_follow = selected;
        self.track_scroll = self.track_scroll.min(lanes.len().saturating_sub(capacity));
        let (hidden_above, hidden_below) = (self.track_scroll, lanes.len().saturating_sub(self.track_scroll + capacity));
        let group_top = y;
        for (i, lane) in lanes.iter().enumerate().skip(self.track_scroll).take(capacity) {
            let r = Rect::from_min_size(pos2(rect.left() + 8.0 + LANE_INDENT, y), vec2(rect.width() - 16.0 - LANE_INDENT, row_h - 4.0));
            let resp = ui.interact(r, ui.scope_id().with(("lane", i)), Sense::click());
            let focused = self.sel == Sel::Lane(i);
            if focused {
                painter.rect_filled(r, 10.0, p.well_hover);
            } else if resp.hovered() {
                painter.rect_filled(r, 10.0, p.well);
            }
            painter.text(pos2(r.left() + 16.0, r.center().y), Align2::LEFT_CENTER, lane.name.to_lowercase(), sans(16.0), if drums_active { p.ink } else { p.muted });
            painter.text(pos2(r.right() - 12.0, r.center().y), Align2::RIGHT_CENTER, note_name(lane.note), mono(12.0), p.muted);
            if s.playing && lane.steps[s.drum_playhead].active {
                painter.circle_filled(pos2(r.left() + 6.0, r.center().y), 2.5, p.accent);
            }
            if resp.clicked() {
                self.select(Sel::Lane(i));
            }
            resp.context_menu(|ui| {
                if ui.button("Clear lane").clicked() {
                    self.engine.edit(|st| {
                        if let Some(l) = st.drums.lanes.get_mut(i) {
                            l.steps = Default::default();
                        }
                    });
                    ui.close();
                }
                if lanes.len() > 1 && ui.button("Remove lane").clicked() {
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
            y += row_h;
        }
        // Group bracket.
        painter.vline(rect.left() + 8.0 + LANE_INDENT * 0.5, group_top..=(y - 4.0), Stroke::new(1.5, if drums_active { p.ink } else { p.line }));
        if hidden_above + hidden_below > 0 {
            painter.text(
                pos2(rect.right() - 14.0, rect.top() + 4.0),
                Align2::RIGHT_TOP,
                format!("lanes {}–{} of {}", hidden_above + 1, lanes.len() - hidden_below, lanes.len()),
                mono(12.0),
                p.muted,
            );
        }
        let add = Rect::from_min_size(pos2(rect.left() + 8.0 + LANE_INDENT, y + 4.0), vec2(56.0, 40.0));
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
}
