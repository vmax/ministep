//! Palette, fonts and type scale: warm light grey, ink-black controls, one
//! orange accent for time (playhead) and record.

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId};

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Color32,
    /// Inactive cells, knob bodies, pill groups.
    pub well: Color32,
    pub well_hover: Color32,
    pub ink: Color32,
    /// Text on ink.
    pub on_ink: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub line: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub rec_off: Color32,
}

pub const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xEC, 0xEB, 0xE8),
    well: Color32::from_rgb(0xE0, 0xDF, 0xDB),
    well_hover: Color32::from_rgb(0xD6, 0xD5, 0xD1),
    ink: Color32::from_rgb(0x1D, 0x1D, 0x1C),
    on_ink: Color32::from_rgb(0xF4, 0xF3, 0xF0),
    muted: Color32::from_rgb(0x84, 0x83, 0x7F),
    faint: Color32::from_rgb(0xB8, 0xB7, 0xB2),
    line: Color32::from_rgb(0xCC, 0xCB, 0xC6),
    accent: Color32::from_rgb(0xF2, 0x63, 0x2B),
    accent_soft: Color32::from_rgb(0xF7, 0xB8, 0x9B),
    rec_off: Color32::from_rgb(0xC4, 0xC3, 0xBE),
};

pub const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x17, 0x17, 0x16),
    well: Color32::from_rgb(0x26, 0x26, 0x25),
    well_hover: Color32::from_rgb(0x30, 0x30, 0x2E),
    ink: Color32::from_rgb(0xEE, 0xED, 0xE9),
    on_ink: Color32::from_rgb(0x17, 0x17, 0x16),
    muted: Color32::from_rgb(0x8C, 0x8B, 0x86),
    faint: Color32::from_rgb(0x4A, 0x4A, 0x47),
    line: Color32::from_rgb(0x33, 0x33, 0x31),
    accent: Color32::from_rgb(0xF2, 0x63, 0x2B),
    accent_soft: Color32::from_rgb(0x7A, 0x3B, 0x22),
    rec_off: Color32::from_rgb(0x44, 0x44, 0x41),
};

pub fn sans(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// Use San Francisco from the system when present (not bundled: licence).
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let load = |path: &str| std::fs::read(path).ok().map(|b| std::sync::Arc::new(FontData::from_owned(b)));
    if let Some(sf) = load("/System/Library/Fonts/SFNS.ttf") {
        fonts.font_data.insert("sf".into(), sf);
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "sf".into());
    }
    if let Some(sfm) = load("/System/Library/Fonts/SFNSMono.ttf") {
        fonts.font_data.insert("sfmono".into(), sfm);
        fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "sfmono".into());
    }
    ctx.set_fonts(fonts);
}

pub fn apply_visuals(ctx: &egui::Context, p: &Palette, dark: bool) {
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.bg;
    v.window_fill = p.bg;
    v.extreme_bg_color = p.well;
    // Text colour follows each widget state (no global override: an override
    // also paints ink text onto ink fills, e.g. a pressed row or a filled button).
    v.selection.bg_fill = p.accent_soft;
    v.selection.stroke = egui::Stroke::new(1.0, p.ink);
    let w = &mut v.widgets;
    w.noninteractive.fg_stroke.color = p.ink;
    w.inactive.fg_stroke.color = p.ink;
    w.inactive.weak_bg_fill = p.well;
    w.inactive.bg_fill = p.well;
    w.hovered.fg_stroke.color = p.ink;
    w.hovered.weak_bg_fill = p.well_hover;
    w.hovered.bg_fill = p.well_hover;
    w.active.fg_stroke.color = p.on_ink;
    w.active.weak_bg_fill = p.ink;
    w.active.bg_fill = p.ink;
    w.open.fg_stroke.color = p.ink;
    w.open.weak_bg_fill = p.well_hover;
    v.window_stroke = egui::Stroke::new(1.0, p.line);
    v.window_corner_radius = 14.into();
    ctx.set_visuals(v);
}
