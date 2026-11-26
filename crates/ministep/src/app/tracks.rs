//! Track list: melodic track and drum lanes.

use super::*;

impl App {
    // --- track list ---------------------------------------------------------------------

    pub(super) fn track_list(&mut self, ui: &mut Ui, rect: Rect, s: &AppState) {
        let p = self.pal();
        let painter = ui.painter().clone();
        heading(&painter, pos2(rect.left() + 26.0, rect.top() + 4.0), "Track", p);
        let mut y = rect.top() + 38.0;
        let row_h = TRACK_ROW_HEIGHT;
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
        // Rows that fit above the "+" button; scroll to keep the selection visible.
        let capacity = (((rect.bottom() - y) / row_h).floor() as usize).saturating_sub(1).max(1);
        let selected = rows.iter().position(|r| r.0 == self.sel).unwrap_or(0);
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
        self.track_scroll = self.track_scroll.min(rows.len().saturating_sub(capacity));
        let (hidden_above, hidden_below) = (self.track_scroll, rows.len().saturating_sub(self.track_scroll + capacity));
        let total = rows.len();
        for (sel, name, detail, hot) in rows.into_iter().skip(self.track_scroll).take(capacity) {
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
        if hidden_above + hidden_below > 0 {
            painter.text(
                pos2(rect.right() - 14.0, rect.top() + 4.0),
                Align2::RIGHT_TOP,
                format!("{}–{} of {total}", hidden_above + 1, total - hidden_below),
                mono(12.0),
                p.muted,
            );
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
}
