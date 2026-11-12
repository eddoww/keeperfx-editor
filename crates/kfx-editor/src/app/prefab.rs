//! Prefabs (`kfx_prefab`) in the editor: the Prefabs panel (starter set, user library, the
//! campaign's library), placing a prefab with the paste ghost (R / F / Escape as for the
//! clipboard), applying it as one undo step, its script fragment appended to the level scripts
//! and paired with that step, and Edit > Save selection as prefab.
//!
//! Placing: the prefab becomes the clipboard (in the chosen orientation and owner mapping) so
//! the paste ghost, its shortcuts and Escape are the clipboard's; a click applies the prefab
//! with `kfx_prefab::apply` (which, unlike a paste, keeps masked cells, action point keys,
//! hero gates and the script fragment). The clipboard the user had is given back when the
//! placing ends.
//!
//! Script fragments: the script editors keep their own undo history, so the text cannot be
//! part of the map's undo step itself. It is paired with it instead: while the placement's
//! step is applied (after the placement, after a redo) the fragment is at the end of the
//! script, when the step is undone (Ctrl+Z, the History panel) the same text is taken out
//! again, each time as one step of the script editor. A fragment the user changed in between
//! is left alone (the status bar says so).
use super::*;
use crate::prefabs::{self as pf, PairedFragment, ThumbCache, ThumbRenderer};
use kfx_prefab::apply::{ApplyOptions, MaskMode, OwnerMap, Report};
use kfx_prefab::capture::{CaptureOptions, Selection};
use kfx_prefab::i18n::{trd, trf};
use kfx_prefab::library::{Library, Origin};
use kfx_prefab::script::{append_fragment, ScriptKind};
use kfx_prefab::transform::Rotation;
use kfx_prefab::ui::{BrowserContext, BrowserEvent, PrefabBrowser};
use kfx_prefab::Prefab;

/// A prefab being placed with the paste ghost.
pub struct Placing {
    pub prefab: Prefab,
    pub options: ApplyOptions,
    /// The clipboard form of the oriented prefab (what the ghost shows).
    pub clip: Clipboard,
    /// The user's clipboard, given back when the placing ends.
    pub saved: Option<Clipboard>,
}

/// Fields of Edit > Save selection as prefab.
#[derive(Clone, Debug)]
pub struct SaveDialog {
    pub name: String,
    pub author: String,
    pub description: String,
    pub tags: String,
    pub mask_empty: bool,
    pub things: bool,
    pub destination: Origin,
    pub script_dk: String,
    pub error: Option<String>,
}

impl Default for SaveDialog {
    fn default() -> Self {
        SaveDialog {
            name: String::new(),
            author: String::new(),
            description: String::new(),
            tags: String::new(),
            mask_empty: false,
            things: true,
            destination: Origin::User,
            script_dk: String::new(),
            error: None,
        }
    }
}

pub struct PrefabState {
    pub browser: PrefabBrowser,
    thumbs: ThumbCache,
    renderer: Option<ThumbRenderer>,
    /// (game data, texture set) the cached thumbnails were drawn with.
    thumb_ctx: (usize, u8),
    /// Cache key of every library entry (recomputed after the library changed).
    keys: Option<Vec<u64>>,
    failed: HashSet<u64>,
    campaign_root: Option<PathBuf>,
    pub placing: Option<Placing>,
    pub paired: Vec<PairedFragment>,
    pub save: Option<SaveDialog>,
}

impl PrefabState {
    fn new(card_size: f32) -> PrefabState {
        let mut lib = Library::new();
        lib.add_embedded_starter();
        if let Some(u) = pf::user_folder() {
            lib.add_root(Origin::User, u, false);
        }
        lib.scan();
        let mut browser = PrefabBrowser::new(lib);
        browser.host_save_dialog = true;
        browser.card_size = card_size.clamp(48.0, 192.0);
        let thumbs: ThumbCache = Default::default();
        let t = thumbs.clone();
        browser.thumbnails = Some(Box::new(move |p: &Prefab| {
            t.lock().ok()?.get(&pf::prefab_key(p)).cloned()
        }));
        PrefabState {
            browser,
            thumbs,
            renderer: None,
            thumb_ctx: (0, 0),
            keys: None,
            failed: HashSet::new(),
            campaign_root: None,
            placing: None,
            paired: vec![],
            save: None,
        }
    }
}

fn mask_name(m: MaskMode) -> &'static str {
    match m {
        MaskMode::Replace => "replace",
        MaskMode::SkipEmpty => "skip_empty",
        MaskMode::KeepExisting => "keep_existing",
    }
}

impl EditorApp {
    fn prefab_state(&mut self) -> &mut PrefabState {
        let card = self.settings.prefabs.card_size;
        self.prefabs.get_or_insert_with(|| PrefabState::new(card))
    }

    /// Show or hide the Prefabs panel.
    pub fn open_prefabs(&mut self, open: bool) {
        self.settings.prefabs.open = open;
        if open {
            self.prefab_state();
            self.prefab_roots_tick();
        }
    }

    pub fn toggle_prefabs(&mut self) {
        let open = !self.settings.prefabs.open;
        self.open_prefabs(open);
    }

    /// The browser (created on first use).
    pub fn prefab_browser(&mut self) -> &mut PrefabBrowser {
        &mut self.prefab_state().browser
    }

    /// The campaign library follows the open level: `prefabs/` in the levels folder of a
    /// level that belongs to a campaign or map pack (`kfx_prefab::library::campaign_folder`).
    fn prefab_roots_tick(&mut self) {
        let want = self
            .doc
            .as_ref()
            .filter(|d| d.pack_cfg.is_some())
            .map(|d| kfx_prefab::library::campaign_folder(&d.doc.folder));
        let Some(st) = self.prefabs.as_mut() else {
            return;
        };
        if st.campaign_root == want {
            return;
        }
        st.browser
            .library
            .roots
            .retain(|r| r.origin != Origin::Campaign);
        if let Some(p) = &want {
            st.browser
                .library
                .add_root(Origin::Campaign, p.clone(), false);
        }
        st.campaign_root = want;
        st.browser.rescan();
        st.keys = None;
    }

    /// Draw missing thumbnails with the terrain renderer (a few per frame).
    fn prefab_thumbs_tick(&mut self) {
        let (Some(doc), Some(atlas), Some(st)) =
            (self.doc.as_ref(), self.atlas.clone(), self.prefabs.as_mut())
        else {
            return;
        };
        let ctx_key = (Arc::as_ptr(&doc.gd) as usize, doc.doc.texture_id);
        if st.thumb_ctx != ctx_key {
            st.thumb_ctx = ctx_key;
            if let Ok(mut c) = st.thumbs.lock() {
                c.clear();
            }
            st.failed.clear();
            st.browser.forget_thumbnails();
        }
        let keys = st
            .keys
            .get_or_insert_with(|| {
                st.browser
                    .library
                    .entries
                    .iter()
                    .map(|e| pf::prefab_key(&e.prefab))
                    .collect()
            })
            .clone();
        let start = Instant::now();
        let mut drawn = 0;
        let owners = st.browser.owners.clone();
        for (i, k) in keys.iter().enumerate() {
            if st.failed.contains(k) || st.thumbs.lock().is_ok_and(|c| c.contains_key(k)) {
                continue;
            }
            if drawn > 0 && start.elapsed() > Duration::from_millis(40) {
                self.ctx.request_repaint();
                break;
            }
            let Some(e) = st.browser.library.entries.get(i) else {
                break;
            };
            let r = st
                .renderer
                .get_or_insert_with(|| ThumbRenderer::new(&self.gpu.device));
            match r.render(
                &self.gpu.device,
                &self.gpu.queue,
                &atlas,
                &self.cubes,
                &doc.gd,
                doc.doc.texture_id,
                &e.prefab,
                &owners,
                128,
            ) {
                Some(img) => {
                    if let Ok(mut c) = st.thumbs.lock() {
                        c.insert(*k, pf::to_color_image(&img));
                    }
                }
                None => {
                    st.failed.insert(*k);
                }
            }
            drawn += 1;
        }
        if drawn > 0 {
            st.browser.forget_thumbnails();
        }
    }

    /// A thumbnail PNG of `p` from the terrain renderer (for a newly saved prefab).
    fn prefab_thumbnail_png(&mut self, p: &Prefab) -> Option<Vec<u8>> {
        let doc = self.doc.as_ref()?;
        let atlas = self.atlas.clone()?;
        let st = self.prefabs.as_mut()?;
        let r = st
            .renderer
            .get_or_insert_with(|| ThumbRenderer::new(&self.gpu.device));
        let img = r.render(
            &self.gpu.device,
            &self.gpu.queue,
            &atlas,
            &self.cubes,
            &doc.gd,
            doc.doc.texture_id,
            p,
            &OwnerMap::default(),
            96,
        )?;
        img.to_png().ok()
    }

    // ------------------------------------------------------------------------- panel

    /// Per frame: the docked Prefabs panel.
    pub(super) fn prefab_dock_ui(&mut self, ui: &mut egui::Ui) {
        if !self.settings.prefabs.open || !self.settings.prefabs.docked {
            return;
        }
        // no room beside the map and the other docks: show it as a window this frame instead of
        // squeezing it to a sliver
        self.prefab_squeezed = ui.available_width() - super::MIN_MAP_W < 360.0;
        if self.prefab_squeezed {
            return;
        }
        let max = super::side_panel_max(ui);
        let w = self.settings.prefabs.dock_w.min(max);
        let r = egui::Panel::right("prefab-dock")
            .resizable(true)
            .default_size(w)
            .size_range(360f32.min(max)..=max)
            .show(ui, |ui| self.prefab_body(ui));
        self.settings.prefabs.dock_w = r.response.rect.width();
    }

    /// Per frame: the Prefabs window (when not docked), the save dialog, the placing upkeep.
    pub(super) fn prefab_window_ui(&mut self, ctx: &egui::Context) {
        self.prefab_placing_tick();
        if self.settings.prefabs.open && (!self.settings.prefabs.docked || self.prefab_squeezed) {
            let mut open = true;
            egui::Window::new(trd("kfx_editor.prefabs", "Prefabs"))
                .open(&mut open)
                .default_size([760.0, 560.0])
                .show(ctx, |ui| self.prefab_body(ui));
            if !open {
                self.settings.prefabs.open = false;
            }
        }
        self.prefab_save_ui(ctx);
    }

    fn prefab_body(&mut self, ui: &mut egui::Ui) {
        self.prefab_state();
        self.prefab_roots_tick();
        self.prefab_thumbs_tick();
        let docked = self.settings.prefabs.docked;
        ui.horizontal(|ui| {
            ui.heading(RichText::new(trd("kfx_editor.prefabs", "Prefabs")).color(GOLD));
            if ui
                .small_button(if docked {
                    trd("kfx_editor.window_e36e", "🗗 Window")
                } else {
                    trd("kfx_editor.dock", "⬅ Dock")
                })
                .on_hover_text(trd(
                    "kfx_editor.own_window_docked_beside_the_map",
                    "Own window / docked beside the map",
                ))
                .clicked()
            {
                self.settings.prefabs.docked = !docked;
            }
            if docked
                && ui
                    .small_button("✖")
                    .on_hover_text(trd("kfx_editor.close", "Close"))
                    .clicked()
            {
                self.settings.prefabs.open = false;
            }
            if ui
                .small_button("⟳")
                .on_hover_text(trd(
                    "kfx_editor.read_the_library_folders_again",
                    "Read the library folders again",
                ))
                .clicked()
            {
                if let Some(st) = self.prefabs.as_mut() {
                    st.browser.rescan();
                    st.keys = None;
                }
            }
        });
        if let Some(p) = self.prefab_placing() {
            ui.add(
                egui::Label::new(
                    RichText::new(trf(
                        "kfx_editor.placing_name_click_the_map_r_rotate_f_flip_esc_t",
                        "Placing {name}: click the map, R rotate, F flip, Esc to stop",
                        &[("name", &p.meta.name)],
                    ))
                    .small()
                    .color(GOLD),
                )
                .wrap(),
            );
        }
        if self.doc.is_none() {
            ui.label(
                RichText::new(trd(
                    "kfx_editor.open_a_map_to_place_prefabs_and_to_see_them_with",
                    "Open a map to place prefabs and to see them with its textures.",
                ))
                .small()
                .color(TEXT_DIM),
            );
        }
        let selection = self.selection.map(|(a, b)| {
            Selection::Rect(SlabRect {
                x0: a.0,
                y0: a.1,
                x1: b.0,
                y1: b.1,
            })
        });
        let events = {
            let Some(st) = self.prefabs.as_mut() else {
                return;
            };
            let bctx = BrowserContext {
                gd: self.doc.as_ref().map(|d| d.gd.as_ref()),
                doc: self.doc.as_ref().map(|d| &d.doc),
                selection,
            };
            st.browser.show(ui, &bctx)
        };
        self.settings.prefabs.card_size =
            self.prefabs.as_ref().map_or(96.0, |s| s.browser.card_size);
        if self
            .prefabs
            .as_mut()
            .is_some_and(|s| std::mem::take(&mut s.browser.save_requested))
        {
            self.open_prefab_save_dialog();
        }
        for e in events {
            match e {
                BrowserEvent::BeginPlacing { prefab, options } => {
                    if let Err(e) = self.begin_prefab_placing(*prefab, *options) {
                        self.status = e;
                    }
                }
                BrowserEvent::LibraryChanged => {
                    if let Some(st) = self.prefabs.as_mut() {
                        st.keys = None;
                    }
                }
                BrowserEvent::Applied { .. } => {}
            }
        }
    }

    // ------------------------------------------------------------------------- choosing

    /// Select the first prefab whose name contains `name` (case-insensitive). While placing,
    /// the ghost becomes the new prefab.
    pub fn prefab_select(&mut self, name: &str) -> Result<(), String> {
        if !self.prefab_browser().select_by_name(name) {
            return Err(trf(
                "kfx_editor.no_prefab_named_like_name",
                "no prefab named like \"{name}\"",
                &[("name", &name)],
            ));
        }
        if self.prefab_placing().is_some() {
            self.prefab_begin_placing()?;
        }
        Ok(())
    }

    /// Orientation for the next placement: clockwise degrees (0, 90, 180, 270) and the east-west
    /// mirror (applied before the turn).
    pub fn prefab_set_orientation(&mut self, degrees: u32, flip: bool) -> Result<(), String> {
        let rot = Rotation::ALL
            .into_iter()
            .find(|r| r.degrees() == degrees % 360)
            .ok_or_else(|| format!("rotation {degrees}: use 0, 90, 180 or 270"))?;
        let b = self.prefab_browser();
        b.orientation.rotation = rot;
        b.orientation.flip = flip;
        self.prefab_refresh_ghost();
        Ok(())
    }

    /// Players of the prefab's slots A, B, ... for the next placement.
    pub fn prefab_set_owners(&mut self, players: &[u8]) -> Result<(), String> {
        if players.iter().any(|&p| p > 8) {
            return Err(trd(
                "kfx_editor.players_are_0_8_4_heroes_5_neutral",
                "players are 0 to 8 (4 heroes, 5 neutral)",
            ));
        }
        self.prefab_browser().owners = OwnerMap::with_first(players);
        self.prefab_refresh_ghost();
        Ok(())
    }

    /// How the prefab combines with the map: `replace`, `skip_empty` or `keep_existing`.
    pub fn prefab_set_mask(&mut self, mode: &str) -> Result<(), String> {
        let m = match mode {
            "replace" => MaskMode::Replace,
            "skip_empty" => MaskMode::SkipEmpty,
            "keep_existing" => MaskMode::KeepExisting,
            _ => {
                return Err(trf(
                    "kfx_editor.mask_mode_use_replace_skip_empty_or_keep_existin",
                    "mask {mode}: use replace, skip_empty or keep_existing",
                    &[("mode", &mode)],
                ))
            }
        };
        self.prefab_browser().mask = m;
        self.prefab_refresh_ghost();
        Ok(())
    }

    /// Start placing the browser's selected prefab with its options.
    pub fn prefab_begin_placing(&mut self) -> Result<(), String> {
        let b = self.prefab_browser();
        let p = b.selected_prefab().cloned().ok_or(trd(
            "kfx_editor.select_a_prefab_first",
            "select a prefab first",
        ))?;
        let o = b.options();
        self.begin_prefab_placing(p, o)
    }

    fn begin_prefab_placing(
        &mut self,
        prefab: Prefab,
        options: ApplyOptions,
    ) -> Result<(), String> {
        let gd = self
            .doc
            .as_ref()
            .map(|d| d.gd.clone())
            .ok_or(trd("kfx_editor.open_a_map_first", "open a map first"))?;
        prefab.check().map_err(|e| e.to_string())?;
        let clip: Clipboard = prefab
            .transformed(options.orientation)
            .to_clipboard(&gd, &options.owners);
        // the user's clipboard comes back when the placing ends (kept from an earlier placing)
        let saved = match self.prefabs.as_mut().and_then(|s| s.placing.take()) {
            Some(p) if self.clipboard.as_ref() == Some(&p.clip) => p.saved,
            _ => self.clipboard.take(),
        };
        let name = prefab.meta.name.clone();
        let o = options.orientation;
        self.clipboard = Some(clip.clone());
        self.prefab_state().placing = Some(Placing {
            prefab,
            options,
            clip,
            saved,
        });
        self.paste();
        self.status = trf(
            "kfx_editor.placing_prefab_name_rotation_flip_click_to_place",
            "Placing prefab {name} ({rotation}°{flip}): click to place, R rotate, F \
            flip, Esc to stop",
            &[
                ("name", &name),
                ("rotation", &o.rotation.degrees()),
                (
                    "flip",
                    &(if o.flip {
                        trd("kfx_editor.flipped", ", flipped")
                    } else {
                        String::new()
                    }),
                ),
            ],
        );
        Ok(())
    }

    /// The browser's options changed while placing: same prefab, new ghost.
    fn prefab_refresh_ghost(&mut self) {
        let Some(st) = self.prefabs.as_ref() else {
            return;
        };
        let Some(p) = st.placing.as_ref() else { return };
        if self.clipboard.as_ref() != Some(&p.clip) {
            return;
        }
        let prefab = p.prefab.clone();
        let mut o = st.browser.options();
        o.substitutions = p.options.substitutions.clone();
        let _ = self.begin_prefab_placing(prefab, o);
    }

    /// `paste.rotate` / `paste.flip` while a prefab is the clipboard: turn the prefab (the
    /// ghost is made again from it, so it matches what a click applies). False when the
    /// clipboard is not a prefab being placed.
    pub(super) fn prefab_turn(&mut self, flip: bool) -> bool {
        let Some(st) = self.prefabs.as_mut() else {
            return false;
        };
        let Some(p) = st.placing.as_mut() else {
            return false;
        };
        if self.clipboard.as_ref() != Some(&p.clip) {
            return false;
        }
        let o = &mut p.options.orientation;
        if flip {
            // flip after (flip f, rotation r) = (flip !f, rotation -r)
            o.rotation = match o.rotation {
                Rotation::R90 => Rotation::R270,
                Rotation::R270 => Rotation::R90,
                r => r,
            };
            o.flip = !o.flip;
        } else {
            o.rotation = o.rotation.next();
        }
        st.browser.orientation = *o;
        let (prefab, options) = (p.prefab.clone(), p.options.clone());
        if let Err(e) = self.begin_prefab_placing(prefab, options) {
            self.status = e;
        }
        true
    }

    /// Per frame: the placing ends with the paste mode (Escape) or when the clipboard changed
    /// (a copy); the user's clipboard comes back in the first case.
    fn prefab_placing_tick(&mut self) {
        let Some(st) = self.prefabs.as_mut() else {
            return;
        };
        let Some(p) = st.placing.as_ref() else { return };
        let ours = self.clipboard.as_ref() == Some(&p.clip);
        if ours && self.pasting {
            return;
        }
        let p = st.placing.take().unwrap();
        if ours {
            self.clipboard = p.saved;
        }
    }

    /// The prefab being placed, if any.
    pub fn prefab_placing(&self) -> Option<&Prefab> {
        let p = self.prefabs.as_ref()?.placing.as_ref()?;
        (self.clipboard.as_ref() == Some(&p.clip)).then_some(&p.prefab)
    }

    // ------------------------------------------------------------------------- applying

    /// A click while pasting: when the clipboard is a prefab being placed, apply it centred on
    /// slab `p` like a paste. False when it is ordinary clipboard content.
    pub(super) fn prefab_stamp(&mut self, p: (i32, i32)) -> bool {
        let Some(c) = self.prefab_placing().map(|_| self.clipboard.clone()) else {
            return false;
        };
        let Some(c) = c else { return false };
        let at = c.origin(p);
        if let Err(e) = self.prefab_apply_at(at) {
            self.status = e;
        }
        true
    }

    /// Place the selected prefab (or the one being placed) with its north-west corner on slab
    /// `at`, with the current orientation, owners and mask. One undo step; its script fragment
    /// is appended to the level scripts, paired with that step.
    pub fn prefab_place_at(&mut self, at: (i32, i32)) -> Result<Report, String> {
        self.prefab_ensure_placing()?;
        self.prefab_apply_at(at)
    }

    /// Placing the browser's selected prefab (started when another one, or none, is placed).
    pub fn prefab_ensure_placing(&mut self) -> Result<(), String> {
        let selected = self.prefab_browser().selected_prefab().cloned();
        if self.prefab_placing().is_none()
            || (selected.is_some() && self.prefab_placing() != selected.as_ref())
        {
            self.prefab_begin_placing()?;
        }
        Ok(())
    }

    fn prefab_apply_at(&mut self, at: (i32, i32)) -> Result<Report, String> {
        let (prefab, options) = {
            let p = self
                .prefabs
                .as_ref()
                .and_then(|s| s.placing.as_ref())
                .ok_or("no prefab is being placed")?;
            (p.prefab.clone(), p.options.clone())
        };
        let d = self.doc.as_ref().ok_or("no map")?;
        // the border ring and locked slabs are refused like for every other tool
        let pv = kfx_prefab::preview(&prefab, &d.gd, Some(&d.doc), at, &options);
        if let Some(c) = pv
            .cells
            .iter()
            .find(|c| c.writes && !self.editable(c.x, c.y))
        {
            return Err(trf(
                "kfx_editor.the_prefab_would_change_slab_x_y_of_the_map_s_bo",
                "The prefab would change slab ({x}, {y}) of the map's border ring: \
                turn on Map > Editable border or place it further in.",
                &[("x", &c.x), ("y", &c.y)],
            ));
        }
        if !pv.cells.iter().any(|c| c.writes)
            && pv
                .cells
                .iter()
                .all(|c| !self.doc.as_ref().is_some_and(|d| d.in_map(c.x, c.y)))
        {
            return Err(trd(
                "kfx_editor.the_prefab_lies_outside_the_map",
                "The prefab lies outside the map.",
            ));
        }
        let name = trf(
            "kfx_editor.prefab_name",
            "Prefab {name}",
            &[("name", &prefab.meta.name)],
        );
        let d = self.doc.as_mut().ok_or("no map")?;
        d.end_stroke();
        let (res, rf) = d.op(&name, |m, g| {
            match kfx_prefab::apply(m, g, &prefab, at, &options) {
                Ok((patch, rep)) => (Ok(rep), patch),
                Err(e) => (Err(e.to_string()), kfx_map::patch::Patch::new()),
            }
        });
        let report = res?;
        let doc_path = d.slb_path();
        let step = d
            .cursor()
            .checked_sub(1)
            .filter(|&i| d.history().get(i).is_some_and(|c| c.name == name));
        let changes = step
            .and_then(|i| d.history().get(i))
            .map_or(0, |c| c.change_count());
        // the script fragment, paired with the step
        let mut texts = vec![];
        let mut warn = String::new();
        for (kind, text) in [
            (ScriptKind::Dk, &report.script_dk),
            (ScriptKind::Lua, &report.script_lua),
        ] {
            let Some(text) = text.as_deref().filter(|t| !t.trim().is_empty()) else {
                continue;
            };
            match self.prefab_append_script(kind, text, &prefab.meta.name) {
                Ok(t) => texts.push(t),
                Err(e) => {
                    warn = trf(
                        "kfx_editor.the_script_fragment_was_not_added_e",
                        " The script fragment was not added: {e}",
                        &[("e", &e)],
                    )
                }
            }
        }
        if let (Some(step), false) = (step, texts.is_empty()) {
            self.prefab_state().paired.push(PairedFragment {
                doc: doc_path,
                step,
                name: name.clone(),
                changes,
                texts,
                in_script: true,
            });
        }
        self.after_change(rf);
        if let Some(st) = self.prefabs.as_mut() {
            st.browser.notify_applied(&prefab.meta.name, report.clone());
            let _ = st.browser.take_events();
        }
        let aps: Vec<String> = report
            .renumbered
            .iter()
            .map(|r| format!("{} {}", r.key, r.number))
            .collect();
        self.status = trf(
            "kfx_editor.placed_prefab_name_at_at_at2_slabs_written_slabs",
            "Placed prefab {name} at ({at}, {at2}): {slabs_written} slabs, \
            {things_added} things{aps}{script_lua}.{warn}",
            &[
                ("name", &prefab.meta.name),
                ("at", &at.0),
                ("at2", &at.1),
                ("slabs_written", &report.slabs_written),
                ("things_added", &report.things_added),
                (
                    "aps",
                    &(if aps.is_empty() {
                        String::new()
                    } else {
                        trf(
                            "kfx_editor.numbers_aps",
                            ", numbers {aps}",
                            &[("aps", &aps.join(", "))],
                        )
                    }),
                ),
                (
                    "script_lua",
                    &(if report.script_dk.is_some() || report.script_lua.is_some() {
                        trd(
                            "kfx_editor.script_fragment_added",
                            ", script fragment added (Ctrl+Z \
                            takes it out again with the map change)",
                        )
                    } else {
                        String::new()
                    }),
                ),
                ("warn", &warn),
            ],
        );
        for w in &report.warnings {
            self.status.push_str(&format!(" {w}"));
        }
        Ok(report)
    }

    /// Append a fragment to the DK (or Lua) script, creating the script when the level has
    /// none. Returns (tab, appended text).
    fn prefab_append_script(
        &mut self,
        kind: ScriptKind,
        fragment: &str,
        label: &str,
    ) -> Result<(String, String), String> {
        let s = self
            .scripts
            .as_mut()
            .ok_or("the level's scripts are not loaded")?;
        let ws = &mut s.ws;
        let tab = match kind {
            ScriptKind::Dk => ws.dk_name(),
            ScriptKind::Lua => ws.lua_name(),
        };
        let exists = ws.tab_infos().iter().any(|t| t.name == tab && t.exists);
        if !exists {
            match kind {
                ScriptKind::Dk => ws.create_dk_script(),
                ScriptKind::Lua => ws.create_lua_script(false, false),
            }
        }
        let e = ws.editor_mut(&tab).ok_or_else(|| format!("no {tab} tab"))?;
        let bytes = e.text_bytes();
        let existing = String::from_utf8_lossy(&bytes).into_owned();
        let full = append_fragment(&existing, fragment, label, kind);
        let added = full.get(existing.len()..).unwrap_or_default().to_string();
        if !e.replace_spans(&[(
            kfx_script::Span::new(bytes.len(), bytes.len()),
            added.clone(),
        )]) {
            return Err(trf(
                "kfx_editor.tab_is_read_only",
                "{tab} is read-only",
                &[("tab", &tab)],
            ));
        }
        e.apply_pending();
        Ok((tab, added))
    }

    /// Called first by every map change: put the paired fragments in or out of the scripts so
    /// that they follow their map step (undo, redo, history jumps).
    pub(super) fn prefab_sync_scripts(&mut self) {
        let Some(st) = self.prefabs.as_mut() else {
            return;
        };
        if st.paired.is_empty() {
            return;
        }
        let Some(d) = self.doc.as_ref() else {
            st.paired.clear();
            return;
        };
        let cursor = d.cursor();
        let hist = d.history();
        let path = d.slb_path();
        // records whose step is gone (history cut by a new change after an undo) are dropped
        st.paired.retain(|r| {
            r.doc == path
                && hist
                    .get(r.step)
                    .is_some_and(|c| c.name == r.name && c.change_count() == r.changes)
        });
        let mut notes = vec![];
        let Some(sc) = self.scripts.as_mut() else {
            return;
        };
        for r in st.paired.iter_mut() {
            let applied = cursor > r.step;
            if applied == r.in_script {
                continue;
            }
            for (tab, text) in &r.texts {
                let Some(e) = sc.ws.editor_mut(tab) else {
                    continue;
                };
                let bytes = e.text_bytes();
                let ok = if applied {
                    e.replace_spans(&[(
                        kfx_script::Span::new(bytes.len(), bytes.len()),
                        text.clone(),
                    )])
                } else {
                    let needle = text.as_bytes();
                    match bytes
                        .windows(needle.len().max(1))
                        .rposition(|w| w == needle)
                    {
                        Some(i) if !needle.is_empty() => e.replace_spans(&[(
                            kfx_script::Span::new(i, i + needle.len()),
                            String::new(),
                        )]),
                        _ => false,
                    }
                };
                if ok {
                    e.apply_pending();
                } else {
                    notes.push(trf(
                        "kfx_editor.the_script_fragment_of_name_in_tab_was_changed_a",
                        "the script fragment of {name} in {tab} was changed and is left as it is",
                        &[("name", &r.name), ("tab", &tab)],
                    ));
                }
            }
            r.in_script = applied;
        }
        if !notes.is_empty() {
            self.status = notes.join("; ");
        }
    }

    // ------------------------------------------------------------------------- saving

    /// Edit > Save selection as prefab.
    pub fn open_prefab_save_dialog(&mut self) {
        if self.selection.is_none() {
            self.status = trd(
                "kfx_editor.select_an_area_of_the_map_first_select_tool_then",
                "Select an area of the map first (Select tool), then save it as a prefab.",
            );
            return;
        }
        let st = self.prefab_state();
        if st.save.is_none() {
            st.save = Some(SaveDialog::default());
        }
    }

    /// Capture the selected rectangle as a new prefab file of the user (or campaign) library.
    pub fn prefab_save_selection(&mut self, dlg: &SaveDialog) -> Result<PathBuf, String> {
        let ((x0, y0), (x1, y1)) = self.selection.ok_or(trd(
            "kfx_editor.select_an_area_of_the_map_first_3ab9",
            "select an area of the map first",
        ))?;
        if dlg.name.trim().is_empty() {
            return Err(trd(
                "kfx_editor.the_prefab_needs_a_name",
                "the prefab needs a name",
            ));
        }
        let d = self.doc.as_ref().ok_or("no map")?;
        let opts = CaptureOptions {
            name: dlg.name.trim().to_string(),
            author: dlg.author.trim().to_string(),
            description: dlg.description.trim().to_string(),
            tags: dlg
                .tags
                .split(',')
                .map(|t| t.trim().to_lowercase())
                .filter(|t| !t.is_empty())
                .collect(),
            mask_empty: dlg.mask_empty,
            things: dlg.things,
            action_points: dlg.things,
            lights: dlg.things,
            script_dk: Some(dlg.script_dk.clone()).filter(|s| !s.trim().is_empty()),
            ..Default::default()
        };
        let mut p = kfx_prefab::capture(
            &d.doc,
            &d.gd,
            &Selection::Rect(SlabRect { x0, y0, x1, y1 }),
            &opts,
        )
        .map_err(|e| e.to_string())?;
        p.thumbnail = self
            .prefab_thumbnail_png(&p)
            .or_else(|| Some(kfx_prefab::render::thumbnail_png(&p, 96)));
        self.prefab_roots_tick();
        let st = self.prefab_state();
        let i = st
            .browser
            .library
            .save_new(dlg.destination, p)
            .map_err(|e| e.to_string())?;
        st.browser.forget_thumbnails();
        st.browser.selected = Some(i);
        st.keys = None;
        let path = st
            .browser
            .library
            .entries
            .get(i)
            .and_then(|e| e.path.clone())
            .unwrap_or_default();
        self.status = trf(
            "kfx_editor.saved_the_selection_as_prefab_name_path",
            "Saved the selection as prefab {name} ({path})",
            &[("name", &dlg.name.trim()), ("path", &path.display())],
        );
        Ok(path)
    }

    fn prefab_save_ui(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.prefabs.as_mut().and_then(|s| s.save.take()) else {
            return;
        };
        let mut open = true;
        let (mut save, mut cancel) = (false, false);
        let campaign = self
            .prefabs
            .as_ref()
            .is_some_and(|s| s.campaign_root.is_some());
        let size = self.selection.map(|(a, b)| (b.0 - a.0 + 1, b.1 - a.1 + 1));
        egui::Window::new(trd(
            "kfx_editor.save_selection_as_prefab",
            "Save selection as prefab",
        ))
        .open(&mut open)
        .resizable(true)
        .default_width(460.0)
        .collapsible(false)
        .show(ctx, |ui| {
            match size {
                Some((w, h)) => ui.label(trf(
                    "kfx_editor.the_selected_w_x_h_slabs_with_their_things_actio",
                    "The selected {w} x {h} slabs with their things, action points \
                    and lights.",
                    &[("w", &w), ("h", &h)],
                )),
                None => ui.colored_label(
                    Color32::LIGHT_RED,
                    trd(
                        "kfx_editor.select_an_area_of_the_map_first",
                        "Select an area of the map first.",
                    ),
                ),
            };
            egui::Grid::new("prefab-save")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label(trd("kfx_editor.name", "Name"));
                    ui.text_edit_singleline(&mut dlg.name);
                    ui.end_row();
                    ui.label(trd("kfx_editor.author", "Author"));
                    ui.text_edit_singleline(&mut dlg.author);
                    ui.end_row();
                    ui.label(trd("kfx_editor.description", "Description"));
                    ui.add(egui::TextEdit::multiline(&mut dlg.description).desired_rows(3));
                    ui.end_row();
                    ui.label(trd("kfx_editor.tags", "Tags"));
                    ui.add(
                        egui::TextEdit::singleline(&mut dlg.tags)
                            .hint_text(trd("kfx_editor.comma_separated", "comma separated")),
                    );
                    ui.end_row();
                    ui.label(trd("kfx_editor.dk_script", "DK script"));
                    ui.add(
                        egui::TextEdit::multiline(&mut dlg.script_dk)
                            .desired_rows(3)
                            .code_editor()
                            .hint_text(trd(
                                "kfx_editor.optional_script_lines_that_use_the_selection_s_a",
                                "optional script lines that use the selection's action points",
                            )),
                    )
                    .on_hover_text(trd(
                        "kfx_editor.action_point_and_hero_gate_numbers_of_this_map_b",
                        "Action point and hero gate numbers of this map become \
                        placeholders, renumbered wherever the prefab is placed.",
                    ));
                    ui.end_row();
                });
            ui.checkbox(
                &mut dlg.mask_empty,
                trd(
                    "kfx_editor.leave_plain_rock_and_earth_out_mask",
                    "Leave plain rock and earth out (mask)",
                ),
            );
            ui.checkbox(
                &mut dlg.things,
                trd(
                    "kfx_editor.include_things_action_points_and_lights",
                    "Include things, action points and lights",
                ),
            );
            ui.horizontal(|ui| {
                ui.label(trd("kfx_editor.save_to", "Save to"));
                ui.selectable_value(
                    &mut dlg.destination,
                    Origin::User,
                    trd("kfx_editor.user_library", "User library"),
                );
                ui.add_enabled_ui(campaign, |ui| {
                    ui.selectable_value(
                        &mut dlg.destination,
                        Origin::Campaign,
                        trd("kfx_editor.campaign_library", "Campaign library"),
                    );
                });
            });
            if let Some(e) = &dlg.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !dlg.name.trim().is_empty() && size.is_some(),
                        egui::Button::new(trd("kfx_editor.save_prefab", "Save prefab")),
                    )
                    .clicked()
                {
                    save = true;
                }
                if ui.button(trd("kfx_editor.cancel", "Cancel")).clicked() {
                    cancel = true;
                }
            });
        });
        if save {
            match self.prefab_save_selection(&dlg) {
                Ok(_) => return,
                Err(e) => dlg.error = Some(e),
            }
        }
        if open && !cancel {
            if let Some(st) = self.prefabs.as_mut() {
                st.save = Some(dlg);
            }
        }
    }

    /// One line per library entry and the last placement (headless `prefab_print`).
    pub fn prefab_summary(&mut self) -> String {
        let b = self.prefab_browser();
        let mut s = String::new();
        for (i, e) in b.library.entries.iter().enumerate() {
            s.push_str(&format!(
                "{}{} [{}] {} x {}{}\n",
                if b.selected == Some(i) { "* " } else { "  " },
                e.prefab.meta.name,
                e.origin.label(),
                e.prefab.w,
                e.prefab.h,
                if e.prefab.scripts.is_empty() {
                    ""
                } else {
                    ", script fragment"
                }
            ));
        }
        s.push_str(&format!(
            "orientation {}°{}, mask {}, owners {:?}\n",
            b.orientation.rotation.degrees(),
            if b.orientation.flip { " flipped" } else { "" },
            mask_name(b.mask),
            b.owners.slots
        ));
        if let Some((n, r)) = &b.last_report {
            s.push_str(&format!("last placed {n}: {}\n", r.summary()));
        }
        s
    }
}

/// `--actions` entries of the prefabs and the translation grid (see docs/manual/cli.md).
pub fn run_actions(
    app: &mut EditorApp,
    obj: &serde_json::Map<String, serde_json::Value>,
    rel: &dyn Fn(&str) -> PathBuf,
) -> Result<(), String> {
    let get = |k: &str| obj.get(k);
    let s = |v: &serde_json::Value| v.as_str().map(str::to_string);
    if let Some(v) = get("prefabs") {
        app.open_prefabs(v.as_bool().unwrap_or(true));
        if let Some(d) = get("docked").and_then(|x| x.as_bool()) {
            app.settings.prefabs.docked = d;
        }
    }
    if let Some(v) = get("prefab_select").and_then(s) {
        app.prefab_select(&v)?;
    }
    if get("prefab_rotate").is_some() || get("prefab_flip").is_some() {
        let o = app.prefab_browser().orientation;
        let deg = get("prefab_rotate")
            .and_then(|x| x.as_u64())
            .map(|x| x as u32)
            .unwrap_or(o.rotation.degrees());
        let flip = get("prefab_flip")
            .and_then(|x| x.as_bool())
            .unwrap_or(o.flip);
        app.prefab_set_orientation(deg, flip)?;
    }
    if let Some(v) = get("prefab_owners").and_then(|x| x.as_array()) {
        let players: Vec<u8> = v
            .iter()
            .filter_map(|x| x.as_u64())
            .map(|x| x.min(255) as u8)
            .collect();
        app.prefab_set_owners(&players)?;
    }
    if let Some(v) = get("prefab_mask").and_then(s) {
        app.prefab_set_mask(&v)?;
    }
    if get("prefab_ghost").is_some_and(|v| v.as_bool() != Some(false)) {
        app.prefab_begin_placing()?;
    }
    if let Some(a) = get("prefab_place").and_then(|v| v.as_array()) {
        let n = |i: usize| {
            a.get(i)
                .and_then(|x| x.as_i64())
                .map(|x| x as i32)
                .ok_or("prefab_place needs [x, y]")
        };
        let mut at = (n(0)?, n(1)?);
        if get("centre")
            .or(get("center"))
            .is_some_and(|v| v.as_bool() == Some(true))
        {
            app.prefab_ensure_placing()?;
            if let Some(c) = app.clipboard.as_ref() {
                at = c.origin(at);
            }
        }
        let r = app.prefab_place_at(at)?;
        eprintln!("prefab placed at ({}, {}): {}", at.0, at.1, r.summary());
        for x in &r.renumbered {
            eprintln!("  {:?} {} -> {}", x.kind, x.key, x.number);
        }
    }
    if let Some(v) = get("prefab_save").and_then(|x| x.as_object()) {
        if let Some(r) = v.get("rect").and_then(|x| x.as_array()) {
            let n = |i: usize| r.get(i).and_then(|x| x.as_i64()).unwrap_or(0) as i32;
            app.select_rect((n(0), n(1)), (n(2), n(3)));
        }
        let txt = |k: &str| {
            v.get(k)
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let tags = match v.get("tags") {
            Some(serde_json::Value::Array(a)) => a
                .iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(","),
            Some(x) => x.as_str().unwrap_or_default().to_string(),
            None => String::new(),
        };
        let dlg = SaveDialog {
            name: txt("name"),
            author: txt("author"),
            description: txt("description"),
            tags,
            mask_empty: v
                .get("mask_empty")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            things: v.get("things").and_then(|x| x.as_bool()).unwrap_or(true),
            destination: if txt("to") == "campaign" {
                Origin::Campaign
            } else {
                Origin::User
            },
            script_dk: txt("script_dk"),
            error: None,
        };
        let p = app.prefab_save_selection(&dlg)?;
        eprintln!("prefab saved: {}", p.display());
    }
    if get("prefab_save_dialog").is_some() {
        app.open_prefab_save_dialog();
    }
    if get("prefab_print").is_some() {
        eprint!("{}", app.prefab_summary());
    }
    super::texts::run_actions(app, obj, rel)
}

#[cfg(test)]
impl EditorApp {
    /// Thumbnails drawn by the terrain renderer so far.
    pub fn prefab_thumb_count(&self) -> usize {
        self.prefabs
            .as_ref()
            .and_then(|s| s.thumbs.lock().ok().map(|c| c.len()))
            .unwrap_or(0)
    }
    pub fn prefab_selected_name(&self) -> Option<String> {
        let b = &self.prefabs.as_ref()?.browser;
        b.selected_prefab().map(|p| p.meta.name.clone())
    }
    /// Orientation of the prefab being placed: (degrees, flipped).
    pub fn prefab_placing_orientation(&self) -> Option<(u32, bool)> {
        let p = self.prefabs.as_ref()?.placing.as_ref()?;
        Some((
            p.options.orientation.rotation.degrees(),
            p.options.orientation.flip,
        ))
    }
    pub fn clipboard_for_test(&self) -> Option<&Clipboard> {
        self.clipboard.as_ref()
    }
    pub fn pasting_for_test(&self) -> bool {
        self.pasting
    }
    pub fn prefab_save_dialog_open(&self) -> bool {
        self.prefabs.as_ref().is_some_and(|s| s.save.is_some())
    }
    pub fn texts_open_for_test(&self) -> bool {
        self.texts.as_ref().is_some_and(|t| t.open)
    }
}
