//! Step grid: ruler, cells, velocity lane, playhead, tabs and tools.

use super::*;

impl App {
    // --- step area ------------------------------------------------------------------------

    pub(super) fn step_area(&mut self, ui: &mut Ui, rect: Rect, s: &AppState) {
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

        let grid_left = rect.left() + if melodic { GUTTER } else { DRUM_GUTTER };
        let loop_w = LOOP_PILL_WIDTH;
        let grid_right = rect.right() - 8.0;
        let cell_gap = CELL_GAP;
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

        let cells_top = ruler_y + 34.0;
        if !melodic {
            let area = Rect::from_min_max(pos2(rect.left(), cells_top), pos2(grid_right, rect.bottom()));
            self.drum_grid(ui, &painter, s, area, grid_left, cell_w, ruler_y);
            return;
        }

        // Cells.
        let cell_h = (cell_w * 1.05).clamp(CELL_MIN_HEIGHT, CELL_MAX_HEIGHT);
        let row = Rect::from_min_max(pos2(grid_left, cells_top), pos2(grid_right, cells_top + cell_h));
        let vel_top = row.bottom() + 22.0;
        let vel_h = (rect.bottom() - vel_top - 8.0).clamp(VEL_MIN_HEIGHT, VEL_MAX_HEIGHT);
        let vel_row = Rect::from_min_max(pos2(grid_left, vel_top), pos2(grid_right, vel_top + vel_h));
        painter.text(pos2(rect.left() + 6.0, vel_row.center().y), Align2::LEFT_CENTER, "VEL", sans(14.0), p.muted);

        // Audition / commit dot in the gutter.
        let dot = pos2(rect.left() + 20.0, row.center().y);
        let pending = s.last_note_played.map(|(n, _)| note_name(n));
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

        self.melodic_cells(ui, &painter, s, first, row, cell_w, &cell_x, &row_resp, &col_at);

        // Velocity lane.
        for col in 0..STEPS_PER_PAGE {
            let r = Rect::from_min_size(pos2(cell_x(col), vel_row.top()), vec2(cell_w, vel_h));
            painter.rect_filled(r, 8.0, p.well.gamma_multiply(0.55));
            let Some(st) = s.sequence.get(first + col) else { continue };
            let (vel, on) = (st.velocity, st.note.is_some());
            let h = (vel_h - 16.0) * f32::from(vel) / 127.0;
            let x = r.left() + cell_w * 0.35;
            let shown_h = if on { h } else { h.min(16.0) };
            painter.line_segment([pos2(x, r.bottom() - 8.0), pos2(x, r.bottom() - 8.0 - shown_h)], Stroke::new(if on { 4.0 } else { 2.0 }, if on { p.ink } else { p.faint }));
        }
        if (vel_resp.dragged() || vel_resp.clicked())
            && let Some(pos) = vel_resp.interact_pointer_pos()
                && let Some(col) = col_at(pos) {
                    let v = (((vel_row.bottom() - 8.0 - pos.y) / (vel_h - 16.0)) * 127.0).clamp(1.0, 127.0) as u8;
                    self.engine.edit(|st| {
                        if let Some(step) = st.sequence.get_mut(first + col) {
                            step.velocity = v;
                        }
                    });
                }
        if vel_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }

        // Playhead: an orange line through ruler, cells and velocity.
        if s.playing {
            let pos = s.playhead;
            if pos < first || pos >= first + STEPS_PER_PAGE {
                return;
            }
            let x = cell_x(pos - first) - cell_gap * 0.5 + 1.0;
            painter.vline(x, (ruler_y + 2.0)..=vel_row.bottom(), Stroke::new(2.5, p.accent));
            painter.circle_filled(pos2(x, ruler_y + 2.0), 4.0, p.accent);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn melodic_cells(
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
                        painter.rect_filled(r, CELL_RADIUS, p.ink.gamma_multiply(dim));
                        painter.text(r.center(), Align2::CENTER_CENTER, note_name(n), mono((cell_w * 0.3).clamp(11.0, 16.0)), p.on_ink);
                        if step.tie && i + 1 < len && col + 1 < STEPS_PER_PAGE {
                            let bar = Rect::from_min_max(pos2(r.right() - 2.0, r.center().y - 5.0), pos2(cell_x(col + 1) + 2.0, r.center().y + 5.0));
                            painter.rect_filled(bar, 0.0, p.ink.gamma_multiply(dim));
                        }
                    }
                    None => {
                        painter.rect_filled(r, CELL_RADIUS, if hovered { p.well_hover } else { p.well });
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
                painter.rect_filled(r, CELL_RADIUS, p.well.gamma_multiply(0.45));
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

    /// All drum lanes as one drum-machine grid: a row per lane, labels on the left,
    /// velocity of the focused lane below, one playhead through every row.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn drum_grid(&mut self, ui: &mut Ui, painter: &egui::Painter, s: &AppState, area: Rect, grid_left: f32, cell_w: f32, ruler_y: f32) {
        let p = self.pal();
        let lanes = &s.drums.lanes;
        if lanes.is_empty() {
            return;
        }
        let focus = match self.sel {
            Sel::Lane(i) => i.min(lanes.len() - 1),
            Sel::Melodic => 0,
        };
        let geo = drum_rows(area.height(), lanes.len());
        let (row_h, vel_h) = (geo.row, geo.vel);
        let pitch = row_h + geo.gap;
        let cell_x = |col: usize| grid_left + col as f32 * (cell_w + CELL_GAP);
        let row_y = |i: usize| area.top() + i as f32 * pitch;
        let rows = Rect::from_min_max(pos2(grid_left, area.top()), pos2(area.right(), row_y(lanes.len()) - geo.gap));
        let radius = (row_h * 0.25).clamp(3.0, CELL_RADIUS);
        let playing_col = s.playing.then_some(s.drum_playhead);

        // Lane labels: click to focus; a dot marks an auditioned (uncommitted) hit.
        for (i, lane) in lanes.iter().enumerate() {
            let label = Rect::from_min_max(pos2(area.left(), row_y(i)), pos2(grid_left - 10.0, row_y(i) + row_h));
            let resp = ui.interact(label, ui.scope_id().with(("lane_label", i)), Sense::click());
            let focused = i == focus;
            if focused {
                painter.rect_filled(Rect::from_min_size(label.min, vec2(3.0, label.height())), 1.5, p.ink);
            }
            let size = row_h.clamp(11.0, 15.0);
            painter.text(pos2(label.left() + 10.0, label.center().y), Align2::LEFT_CENTER, lane.name.to_lowercase(), sans(size), if focused { p.ink } else { p.muted });
            if s.drums.audition.contains_key(&i) {
                painter.circle_filled(pos2(label.right() - 4.0, label.center().y), 3.5, p.ink);
            }
            if resp.on_hover_text(format!("{} · {} — click to focus", lane.name, note_name(lane.note))).clicked() {
                self.select(Sel::Lane(i));
            }
        }

        // Cells.
        for (i, lane) in lanes.iter().enumerate() {
            for (col, step) in lane.steps.iter().enumerate() {
                let r = Rect::from_min_size(pos2(cell_x(col), row_y(i)), vec2(cell_w, row_h));
                let fill = if step.active {
                    if playing_col == Some(col) {
                        p.accent
                    } else {
                        // Softer hits read lighter, like LED brightness.
                        p.ink.gamma_multiply(0.45 + 0.55 * f32::from(step.velocity) / 127.0)
                    }
                } else if i == focus {
                    p.well_hover
                } else {
                    p.well
                };
                painter.rect_filled(r, radius, fill);
                if !step.active && col % 4 == 0 {
                    painter.circle_filled(r.center(), 1.6, p.faint);
                }
            }
        }
        // Keyboard cursor.
        if s.drums.cursor_lane < lanes.len() {
            let r = Rect::from_min_size(pos2(cell_x(s.drums.cursor_step), row_y(s.drums.cursor_lane)), vec2(cell_w, row_h));
            painter.rect_stroke(r.expand(2.0), radius + 2.0, Stroke::new(1.5, p.accent.gamma_multiply(0.8)), StrokeKind::Outside);
        }

        // Click toggles; dragging paints across steps and lanes with the first cell's new state.
        let resp = ui.interact(rows, ui.scope_id().with("drum_grid"), Sense::click_and_drag());
        let cell_at = |pos: Pos2| -> Option<(usize, usize)> {
            let col = ((pos.x - grid_left) / (cell_w + CELL_GAP)).floor();
            let row = ((pos.y - area.top()) / pitch).floor();
            (col >= 0.0 && (col as usize) < DRUM_STEPS && row >= 0.0 && (row as usize) < lanes.len()).then_some((row as usize, col as usize))
        };
        let paint_id = ui.scope_id().with("drum_paint");
        if let Some((row, col)) = resp.interact_pointer_pos().and_then(cell_at) {
            if resp.drag_started() || resp.clicked() {
                ui.data_mut(|d| d.insert_temp(paint_id, !lanes[row].steps[col].active));
                if row != focus {
                    self.select(Sel::Lane(row));
                }
            }
            let target: Option<bool> = ui.data(|d| d.get_temp(paint_id));
            if (resp.dragged() || resp.clicked() || resp.drag_started())
                && let Some(target) = target
                && lanes[row].steps[col].active != target
            {
                self.engine.edit(|st| {
                    st.drums.cursor_lane = row;
                    st.drums.cursor_step = col;
                    if let Some(l) = st.drums.lanes.get_mut(row) {
                        l.steps[col].active = target;
                    }
                });
                if target && !s.playing {
                    self.engine.audition_lane(row, lanes[row].steps[col].velocity);
                }
            }
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        // Velocity of the focused lane.
        let vel_top = rows.bottom() + DRUM_VEL_GAP;
        let vel_row = Rect::from_min_max(pos2(grid_left, vel_top), pos2(area.right(), vel_top + vel_h));
        painter.text(pos2(area.left() + 10.0, vel_row.center().y - 8.0), Align2::LEFT_CENTER, "VEL", sans(13.0), p.muted);
        painter.text(pos2(area.left() + 10.0, vel_row.center().y + 9.0), Align2::LEFT_CENTER, lanes[focus].name.to_lowercase(), sans(11.0), p.faint);
        for (col, step) in lanes[focus].steps.iter().enumerate() {
            let r = Rect::from_min_size(pos2(cell_x(col), vel_row.top()), vec2(cell_w, vel_h));
            painter.rect_filled(r, 6.0, p.well.gamma_multiply(0.55));
            let h = (vel_h - 12.0) * f32::from(step.velocity) / 127.0;
            let x = r.left() + cell_w * 0.35;
            let shown = if step.active { h } else { h.min(10.0) };
            painter.line_segment([pos2(x, r.bottom() - 6.0), pos2(x, r.bottom() - 6.0 - shown)], Stroke::new(if step.active { 4.0 } else { 2.0 }, if step.active { p.ink } else { p.faint }));
        }
        let vresp = ui.interact(vel_row, ui.scope_id().with("drum_vel"), Sense::click_and_drag());
        if (vresp.dragged() || vresp.clicked())
            && let Some(pos) = vresp.interact_pointer_pos()
        {
            let col = ((pos.x - grid_left) / (cell_w + CELL_GAP)).floor();
            if col >= 0.0 && (col as usize) < DRUM_STEPS {
                let v = (((vel_row.bottom() - 6.0 - pos.y) / (vel_h - 12.0)) * 127.0).clamp(1.0, 127.0) as u8;
                self.engine.edit(|st| {
                    if let Some(l) = st.drums.lanes.get_mut(focus) {
                        l.steps[col as usize].velocity = v;
                    }
                });
            }
        }
        if vresp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }

        // Commit dot (auditioned hits) in the ruler gutter.
        if !s.drums.audition.is_empty() {
            let dot = pos2(area.left() + 16.0, ruler_y + 10.0);
            let dresp = ui.interact(Rect::from_center_size(dot, vec2(28.0, 28.0)), ui.scope_id().with("drum_commit"), Sense::click());
            painter.circle_filled(dot, 7.0, if s.recording { p.accent } else { p.ink });
            if dresp.on_hover_text("Commit the auditioned hits (Enter)").clicked() {
                self.engine.commit();
            }
        }

        // Playhead through every row.
        if let Some(col) = playing_col {
            let x = cell_x(col) - CELL_GAP * 0.5 + 1.0;
            painter.vline(x, (ruler_y + 2.0)..=vel_row.bottom(), Stroke::new(2.5, p.accent));
            painter.circle_filled(pos2(x, ruler_y + 2.0), 4.0, p.accent);
        }
    }

    // --- tools row ------------------------------------------------------------------------

    pub(super) fn tabs_and_tools(&mut self, ui: &mut Ui, rect: Rect, s: &AppState) {
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
        let tools: Vec<Tool> = if melodic {
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
}
