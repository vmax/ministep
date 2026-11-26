//! Patterns and settings dialogs.

use super::*;

impl App {
    // --- modals -----------------------------------------------------------------------------

    pub(super) fn open_patterns(&mut self, save: bool) {
        let kind = self.engine.pattern_kind();
        self.patterns = Some(PatternDialog { save, kind, name: String::new(), names: self.engine.patterns().list_names(kind), focus: true });
    }

    pub(super) fn patterns_modal(&mut self, ctx: &egui::Context) {
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

    pub(super) fn settings_modal(&mut self, ctx: &egui::Context) {
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
                if let Some(c) = choice
                    && let Err(e) = self.ports.set_input(&self.engine, c.as_deref()) {
                        self.engine.edit(|st| st.status_message = e);
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
}

/// Subsequence match: "egy1" matches "egypt_01".
fn fuzzy(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}
