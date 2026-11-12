//! Accessors for the UI tests (`crate::ui_tests`).
use super::*;

impl EditorApp {
    /// Load another install folder, as the first-run dialog does (also when `KFX_INSTALL` chose
    /// the start-up install).
    pub fn use_install(&mut self, p: &Path) {
        self.set_install(p.to_path_buf());
    }
    /// Type into the first-run folder field.
    pub fn first_run_text(&mut self, t: &str) {
        self.first_run.text = t.to_string();
    }
    /// The rules being edited in Map > Random doodads.
    pub fn doodad_dialog_set(&mut self) -> Option<&mut crate::doodads::Doodads> {
        self.doodad_dialog.as_mut().map(|d| &mut d.set)
    }
    pub fn toast_texts(&self) -> Vec<String> {
        self.toasts.iter().map(|t| t.text.clone()).collect()
    }
    pub fn script_text(&mut self, name: &str) -> Option<String> {
        let ws = self.script_ws()?;
        ws.editor(name)
            .map(|e| String::from_utf8_lossy(&e.text_bytes()).into_owned())
    }
    pub fn wizard_open(&mut self) -> bool {
        self.script_ws().is_some_and(|w| w.wizard_open())
    }
    pub fn pane_rects(&self) -> &[egui::Rect] {
        &self.panes
    }
    /// Top height of slab (x, y) in the view's terrain.
    pub fn slab_top(&self, x: i32, y: i32) -> f32 {
        self.terrain.slab_top(x.max(0) as usize, y.max(0) as usize) as f32
    }
    pub fn browser_open(&self) -> bool {
        self.browser.open
    }
    pub fn brush_kind(&self) -> Option<u16> {
        self.brush
    }
    pub fn brush_size(&self) -> u32 {
        self.brush_size
    }
    pub fn selection_rect(&self) -> Option<((i32, i32), (i32, i32))> {
        self.selection
    }
    pub fn capturing_shortcut(&self) -> bool {
        self.shortcut_capture.is_some()
    }
    pub fn new_dialog_open(&self) -> bool {
        self.new_dialog.is_some()
    }
    pub fn pending_prompt(&self) -> bool {
        self.pending.is_some()
    }
    pub fn selected_string(&self) -> Option<usize> {
        self.strings.as_ref().and_then(|s| s.selected)
    }
    pub fn tool_name(&self) -> &'static str {
        self.tool.name()
    }
    pub fn on_disk(&self) -> bool {
        self.doc_on_disk
    }
    pub fn close_first_run(&mut self) {
        if self.assets.is_some() {
            self.first_run.open = false;
        }
    }
    /// Forget the unsaved changes (so that a following New / Open does not ask).
    pub fn discard_changes_for_test(&mut self) {
        if let Some(d) = self.doc.as_mut() {
            d.mark_saved();
        }
    }
    pub fn script_panel_open(&self) -> bool {
        self.settings.script_dock == crate::settings::ScriptDock::Docked && self.scripts.is_some()
    }
    /// A point inside the text of the docked script editor (left part of the panel).
    pub fn script_text_rect(&self) -> Option<egui::Rect> {
        let r = self.script_rect?;
        Some(egui::Rect::from_min_size(
            r.min + EVec2::new(r.width() * 0.12, r.height() * 0.25),
            EVec2::new(r.width() * 0.3, r.height() * 0.3),
        ))
    }
    pub fn script_active_text(&mut self) -> String {
        self.scripts
            .as_mut()
            .map(|s| String::from_utf8_lossy(&s.ws.active_editor_mut().text_bytes()).into_owned())
            .unwrap_or_default()
    }
    /// The centre subtile of an open, walkable slab near slab (x, y).
    pub fn find_floor_subtile(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        let d = self.doc.as_ref()?;
        for r in 0..12 {
            for dy in -r..=r {
                for dx in -r..=r {
                    let (sx, sy) = (x + dx, y + dy);
                    if d.state_at(sx, sy).is_none() {
                        continue;
                    }
                    // open floor all around and no thing within two slabs
                    let open = (-1..=1).all(|oy| {
                        (-1..=1).all(|ox| {
                            d.state_at(sx + ox, sy + oy)
                                .is_some_and(|n| !assets::is_solid(&d.gd, n.kind as usize))
                        })
                    });
                    let clear = (-6..=8).all(|oy| {
                        (-6..=8).all(|ox| d.doc.things_at(3 * sx + ox, 3 * sy + oy).is_empty())
                    });
                    if open && clear && d.doc.floor_height(3 * sx + 1, 3 * sy + 1) <= 1 {
                        return Some((3 * sx + 1, 3 * sy + 1));
                    }
                }
            }
        }
        None
    }
}
