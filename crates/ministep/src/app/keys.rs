//! Keyboard shortcuts (the TUI key map).

use super::*;

impl App {
    // --- keyboard ---------------------------------------------------------------------------

    pub(super) fn keys(&mut self, ctx: &egui::Context) {
        // The only text fields are in the dialogs and the BPM editor.
        if self.patterns.is_some() || self.settings_open || self.bpm_edit.is_some() {
            return;
        }
        // Our painted controls must not hold keyboard focus: egui would treat Tab
        // and the arrows as focus navigation and report that it wants the keyboard.
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
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
