//! Keyboard shortcuts of the editor window (global ones and those of the map view).
use super::*;

impl EditorApp {
    /// Global shortcuts (also while a text field has focus): save, play, script panel, help.
    pub(super) fn global_shortcuts(&mut self, ctx: &egui::Context) {
        let k = self.keymap.clone();
        if k.pressed(ctx, "file.save_as") {
            self.open_save_as();
        }
        if k.pressed(ctx, "file.save") && self.doc.is_some() {
            self.do_save();
        }
        if k.pressed(ctx, "play.save_play") {
            self.play(true);
        }
        if k.pressed(ctx, "play.play") {
            self.play(false);
        }
        if k.pressed(ctx, "play.log") {
            self.refresh_game_log();
            self.game_log.open = !self.game_log.open;
        }
        if k.pressed(ctx, "view.script") {
            self.toggle_script_panel();
        }
        if k.pressed(ctx, "view.script_max") && self.doc.is_some() {
            self.script_max = !self.script_max;
        }
        if k.pressed(ctx, "view.events") && self.doc.is_some() {
            self.toggle_events();
        }
        if k.pressed(ctx, "play.live") {
            self.settings.audiolive.live_open = !self.settings.audiolive.live_open;
        }
        if k.pressed(ctx, "help.controls") {
            self.show_controls = !self.show_controls;
        }
        if k.pressed(ctx, "view.prefabs") {
            self.toggle_prefabs();
        }
        if k.pressed(ctx, "file.open") && self.assets.is_some() {
            self.open_dialog();
        }
        if k.pressed(ctx, "file.new") && self.assets.is_some() {
            self.request(Pending::New);
        }
        if k.pressed(ctx, "file.browser") && self.assets.is_some() {
            self.browser.open = !self.browser.open;
        }
        self.campaign_shortcuts(ctx);
    }

    /// Map shortcuts (when no text field has focus), from the keymap.
    pub(super) fn shortcuts(&mut self, ctx: &egui::Context) {
        // an open menu / combo box gets the keys (Escape closes it rather than being taken by
        // the map)
        if ctx.egui_wants_keyboard_input() || self.script_max || egui::Popup::is_any_open(ctx) {
            return;
        }
        if self.logic_owns_keys(ctx) {
            return;
        }
        let (mut copy, mut cut, mut paste) = (false, false, false);
        ctx.input(|i| {
            for e in &i.events {
                match e {
                    egui::Event::Copy => copy = true,
                    egui::Event::Cut => cut = true,
                    egui::Event::Paste(_) => paste = true,
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.command => match key {
                        egui::Key::C => copy = true,
                        egui::Key::X => cut = true,
                        egui::Key::V => paste = true,
                        _ => {}
                    },
                    _ => {}
                }
            }
        });
        let k = self.keymap.clone();
        if k.pressed(ctx, "edit.redo2") || k.pressed(ctx, "edit.redo") {
            self.redo();
        } else if k.pressed(ctx, "edit.undo") {
            self.undo();
        }
        if k.pressed(ctx, "edit.select_all") {
            self.select_all();
        }
        if cut {
            self.cut();
        } else if copy {
            self.copy();
        }
        if paste {
            self.paste();
        }
        if k.pressed(ctx, "edit.cancel") {
            if self.fly_capture {
                self.set_fly_capture(false);
            } else {
                self.cancel_tool();
            }
        }
        if k.pressed(ctx, "view.fly") {
            self.toggle_fly();
        }
        if k.pressed(ctx, "view.fly_look") && self.camera.fly {
            let on = !self.fly_capture;
            self.set_fly_capture(on);
        }
        if k.pressed(ctx, "edit.delete") {
            self.delete_selection();
        }
        if self.pasting {
            if k.pressed(ctx, "paste.rotate") {
                self.rotate_paste();
            }
            if k.pressed(ctx, "paste.flip") {
                self.flip_paste();
            }
        } else if !self.selected.is_empty() {
            if k.pressed(ctx, "things.rotate_ccw") {
                self.rotate_things(-1);
            }
            if k.pressed(ctx, "things.rotate_cw") {
                self.rotate_things(1);
            }
        }
        if k.pressed(ctx, "things.door_lock") {
            self.toggle_door_lock();
        }
        if k.pressed(ctx, "edit.duplicate") {
            self.duplicate_things();
        }
        if k.pressed(ctx, "view.2d") && self.doc.is_some() {
            let m = if self.settings.layout.view_mode == crate::settings::ViewMode::TwoD {
                crate::settings::ViewMode::ThreeD
            } else {
                crate::settings::ViewMode::TwoD
            };
            self.set_view_mode(m);
        }
        if k.pressed(ctx, "view.split") && self.doc.is_some() {
            let m = if self.settings.layout.view_mode == crate::settings::ViewMode::Split {
                crate::settings::ViewMode::ThreeD
            } else {
                crate::settings::ViewMode::Split
            };
            self.set_view_mode(m);
        }
        if k.pressed(ctx, "brush.smaller") {
            self.brush_size = (self.brush_size - 1).max(1);
        }
        if k.pressed(ctx, "brush.larger") {
            self.brush_size = (self.brush_size + 1).min(tools::MAX_BRUSH);
        }
    }
}
