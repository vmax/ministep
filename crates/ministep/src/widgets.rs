//! Hand-painted controls in the hardware style: pills, icons, knobs, envelope.

use std::f32::consts::{PI, TAU};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use crate::theme::{Palette, sans};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Play,
    Stop,
    Rest,
    Hold,
    Undo,
    Clear,
    Root,
    Grid,
    Settings,
    Wave,
    Dots,
    Plus,
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    Moon,
    Commit,
    Follow,
}

/// Paint an icon centred in `rect` with stroke colour `c`.
pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, c: Color32) {
    let ctr = rect.center();
    let s = rect.height().min(rect.width()) * 0.5;
    let st = Stroke::new((s * 0.16).clamp(1.4, 2.4), c);
    let p = |x: f32, y: f32| pos2(ctr.x + x * s, ctr.y + y * s);
    match icon {
        Icon::Play => {
            painter.add(Shape::convex_polygon(vec![p(-0.35, -0.5), p(0.55, 0.0), p(-0.35, 0.5)], c, Stroke::NONE));
        }
        Icon::Stop => {
            painter.rect_filled(Rect::from_center_size(ctr, vec2(s * 0.95, s * 0.95)), 2.0, c);
        }
        Icon::Rest => {
            painter.line_segment([p(-0.6, 0.0), p(0.6, 0.0)], st);
            painter.line_segment([p(-0.6, -0.35), p(-0.6, 0.35)], st);
            painter.line_segment([p(0.6, -0.35), p(0.6, 0.35)], st);
        }
        Icon::Hold => {
            let pts: Vec<Pos2> = (0..=16).map(|i| {
                let t = i as f32 / 16.0;
                p(-0.65 + 1.3 * t, 0.25 - 0.55 * (PI * t).sin())
            }).collect();
            painter.add(Shape::line(pts, st));
            painter.circle_filled(p(-0.65, 0.3), s * 0.13, c);
            painter.circle_filled(p(0.65, 0.3), s * 0.13, c);
        }
        Icon::Undo => {
            let pts: Vec<Pos2> = (0..=20).map(|i| {
                let a = PI * 1.1 * i as f32 / 20.0 - PI * 0.05;
                p(0.1 + 0.45 * a.cos(), 0.05 + 0.45 * a.sin())
            }).collect();
            painter.add(Shape::line(pts, st));
            painter.add(Shape::convex_polygon(vec![p(-0.65, -0.05), p(-0.2, -0.05), p(-0.42, 0.35)], c, Stroke::NONE));
        }
        Icon::Clear => {
            painter.line_segment([p(-0.45, -0.45), p(0.45, 0.45)], st);
            painter.line_segment([p(0.45, -0.45), p(-0.45, 0.45)], st);
        }
        Icon::Root => {
            painter.circle_stroke(p(-0.15, 0.25), s * 0.3, st);
            painter.line_segment([p(0.15, 0.25), p(0.15, -0.6)], st);
            painter.line_segment([p(0.15, -0.6), p(0.55, -0.4)], st);
        }
        Icon::Grid => {
            for y in [-0.5, 0.0, 0.5] {
                for x in [-0.5, 0.0, 0.5] {
                    painter.circle_filled(p(x, y), s * 0.12, c);
                }
            }
        }
        Icon::Settings => {
            let r = Rect::from_center_size(ctr, vec2(s * 1.4, s * 1.4));
            painter.rect_stroke(r, s * 0.3, st, StrokeKind::Middle);
            painter.line_segment([p(0.15, -0.7), p(0.7, -0.7)], st);
            painter.line_segment([p(0.7, -0.7), p(0.7, -0.15)], st);
        }
        Icon::Wave => {
            let pts: Vec<Pos2> = (0..=24).map(|i| {
                let t = i as f32 / 24.0;
                p(-0.7 + 1.4 * t, -0.35 * (TAU * t).sin())
            }).collect();
            painter.add(Shape::line(pts, st));
        }
        Icon::Dots => {
            for y in [-0.5, 0.0, 0.5] {
                for x in [-0.22, 0.22] {
                    painter.circle_filled(p(x, y), s * 0.1, c);
                }
            }
        }
        Icon::Plus => {
            painter.line_segment([p(-0.5, 0.0), p(0.5, 0.0)], st);
            painter.line_segment([p(0.0, -0.5), p(0.0, 0.5)], st);
        }
        Icon::ChevronLeft => {
            painter.add(Shape::line(vec![p(0.2, -0.45), p(-0.25, 0.0), p(0.2, 0.45)], st));
        }
        Icon::ChevronRight => {
            painter.add(Shape::line(vec![p(-0.2, -0.45), p(0.25, 0.0), p(-0.2, 0.45)], st));
        }
        Icon::ChevronDown => {
            painter.add(Shape::line(vec![p(-0.4, -0.15), p(0.0, 0.25), p(0.4, -0.15)], st));
        }
        Icon::Moon => {
            painter.circle_filled(ctr, s * 0.6, c);
            painter.circle_filled(p(0.3, -0.25), s * 0.5, painter.ctx().global_style().visuals.panel_fill);
        }
        Icon::Commit => {
            painter.add(Shape::line(vec![p(-0.5, 0.0), p(-0.12, 0.4), p(0.55, -0.4)], st));
        }
        Icon::Follow => {
            painter.line_segment([p(-0.6, 0.0), p(0.45, 0.0)], st);
            painter.add(Shape::line(vec![p(0.15, -0.3), p(0.5, 0.0), p(0.15, 0.3)], st));
            painter.line_segment([p(-0.6, -0.45), p(-0.6, 0.45)], st);
        }
    }
}

/// A rounded pill button. `active` paints it ink with light content.
pub fn pill(ui: &mut Ui, rect: Rect, p: &Palette, active: bool, tip: &str, content: impl FnOnce(&egui::Painter, Rect, Color32)) -> Response {
    let resp = ui.interact(rect, ui.scope_id().with(("pill", tip, rect.min.x as i32, rect.min.y as i32)), Sense::click());
    let fill = if active {
        p.ink
    } else if resp.hovered() {
        p.well_hover
    } else {
        p.well
    };
    let painter = ui.painter();
    painter.rect_filled(rect, rect.height() * 0.5, fill);
    content(painter, rect, if active { p.on_ink } else { p.ink });
    if !tip.is_empty() {
        resp.clone().on_hover_text(tip);
    }
    resp
}

pub fn icon_pill(ui: &mut Ui, rect: Rect, p: &Palette, active: bool, icon: Icon, tip: &str) -> Response {
    pill(ui, rect, p, active, tip, |painter, r, c| {
        paint_icon(painter, Rect::from_center_size(r.center(), vec2(r.height() * 0.5, r.height() * 0.5)), icon, c);
    })
}

pub fn text_pill(ui: &mut Ui, rect: Rect, p: &Palette, active: bool, text: &str, size: f32) -> Response {
    pill(ui, rect, p, active, "", |painter, r, c| {
        painter.text(r.center(), Align2::CENTER_CENTER, text, sans(size), c);
    })
}

/// A rotary knob: vertical drag (shift = fine), scroll, double-click resets.
pub struct Knob<'a> {
    pub value: &'a mut u8,
    pub default: u8,
    pub label: &'a str,
    pub display: String,
    pub bold_arc: bool,
}

const KNOB_START: f32 = PI * 0.75;
const KNOB_SWEEP: f32 = PI * 1.5;

impl Knob<'_> {
    /// Returns true when the value changed.
    pub fn show(self, ui: &mut Ui, rect: Rect, p: &Palette) -> bool {
        let id = ui.scope_id().with(("knob", self.label, rect.min.x as i32, rect.min.y as i32));
        let r = (rect.width().min(rect.height() - 40.0) * 0.5 - 2.0).max(14.0);
        let centre = pos2(rect.center().x, rect.min.y + r + 2.0);
        let body = Rect::from_center_size(centre, Vec2::splat(r * 2.0));
        let resp = ui.interact(body, id, Sense::click_and_drag());
        let before = *self.value;
        let acc_id = id.with("acc");
        if resp.drag_started() {
            ui.data_mut(|d| d.insert_temp(acc_id, f32::from(*self.value)));
        }
        if resp.dragged() {
            let fine = ui.input(|i| i.modifiers.shift);
            let per_px = if fine { 0.15 } else { 0.6 };
            let mut acc: f32 = ui.data(|d| d.get_temp(acc_id)).unwrap_or(f32::from(*self.value));
            acc = (acc - resp.drag_delta().y * per_px + resp.drag_delta().x * per_px * 0.5).clamp(0.0, 127.0);
            ui.data_mut(|d| d.insert_temp(acc_id, acc));
            *self.value = acc.round() as u8;
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                *self.value = (f32::from(*self.value) + scroll * 0.25).clamp(0.0, 127.0).round() as u8;
            }
        }
        if resp.double_clicked() {
            *self.value = self.default;
        }
        let painter = ui.painter();
        let t = f32::from(*self.value) / 127.0;
        painter.circle_filled(centre, r, if resp.hovered() || resp.dragged() { p.well_hover } else { p.well });
        painter.circle_stroke(centre, r, Stroke::new(1.0, p.line));
        // Value arc just inside the rim.
        let arc_r = r - if self.bold_arc { 4.0 } else { 3.0 };
        let steps = 48;
        let end = KNOB_START + KNOB_SWEEP * t;
        if t > 0.0 {
            let pts: Vec<Pos2> = (0..=steps)
                .map(|i| {
                    let a = KNOB_START + (end - KNOB_START) * i as f32 / steps as f32;
                    centre + vec2(a.cos(), a.sin()) * arc_r
                })
                .collect();
            painter.add(Shape::line(pts, Stroke::new(if self.bold_arc { 5.0 } else { 2.0 }, p.ink)));
        }
        // Pointer.
        let dir = vec2(end.cos(), end.sin());
        painter.line_segment([centre + dir * (r * 0.45), centre + dir * (r - 7.0)], Stroke::new(3.0, p.ink));
        painter.text(pos2(centre.x, centre.y + r + 16.0), Align2::CENTER_CENTER, self.label.to_uppercase(), sans(13.0), p.ink);
        painter.text(pos2(centre.x, centre.y + r + 34.0), Align2::CENTER_CENTER, &self.display, sans(13.0), p.muted);
        if resp.dragged() || resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        *self.value != before
    }
}

/// Filled ADSR shape from four 0..127 knob values; returns nothing (display only).
pub fn paint_envelope(painter: &egui::Painter, rect: Rect, p: &Palette, adsr: [u8; 4]) {
    let [a, d, s, r] = adsr.map(|v| f32::from(v) / 127.0);
    let w = rect.width();
    let seg = w / 4.0;
    let x0 = rect.left();
    let top = rect.top() + 6.0;
    let bottom = rect.bottom() - 4.0;
    let sus_y = bottom - (bottom - top) * s;
    let xa = x0 + seg * (0.04 + 0.96 * a);
    let xd = xa + seg * (0.06 + 0.94 * d);
    let xr_start = x0 + w - seg * (0.04 + 0.96 * r);
    let xr_start = xr_start.max(xd + 8.0);
    let pts = [pos2(x0, bottom), pos2(xa, top), pos2(xd, sus_y), pos2(xr_start, sus_y), pos2(x0 + w, bottom)];
    // Fill as a fan of convex pieces (the outline itself is not convex).
    let base = |x: f32| pos2(x, bottom);
    painter.add(Shape::convex_polygon(vec![pts[0], pts[1], base(xa)], p.ink, Stroke::NONE));
    painter.add(Shape::convex_polygon(vec![base(xa), pts[1], pts[2], base(xd)], p.ink, Stroke::NONE));
    painter.add(Shape::convex_polygon(vec![base(xd), pts[2], pts[3], base(xr_start)], p.ink, Stroke::NONE));
    painter.add(Shape::convex_polygon(vec![base(xr_start), pts[3], pts[4]], p.ink, Stroke::NONE));
    for pt in pts {
        painter.circle_filled(pt, 4.5, p.ink);
        painter.circle_filled(pt, 2.0, p.bg);
    }
}

/// Small caps section heading.
pub fn heading(painter: &egui::Painter, pos: Pos2, text: &str, p: &Palette) {
    painter.text(pos, Align2::LEFT_TOP, text.to_uppercase(), sans(13.0), p.muted);
}

