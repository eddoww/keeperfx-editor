//! The campaign editor inside the map editor: the Campaign menu, the campaign / map pack editor
//! window ([`kfx_campaign_ui::CampaignEditor`]), the "New campaign / map pack" wizard on top of
//! [`kfx_campaign_ui::create_pack`], "Add current level to campaign", and the widget's open-level
//! and play-level requests (opened through the editor's unsaved-changes prompt, played through
//! the editor's own play path, `app/play.rs`).
//!
//! The window's open state and the last campaign are kept in `Settings.layout.campaign`; the
//! Workshop window (`app/workshop_win.rs`) shares that block and the [`Hub`].
use super::*;
use kfx_campaign_ui::files::valid_pack_name;
use kfx_campaign_ui::ui::Tab as CampTab;
use kfx_campaign_ui::{AddLevel, CampaignEditor, Event as CampEvent, LevelSource, NewPack, Pack};
use kfx_formats::campaign::PackKind;
use kfx_formats::levelinfo::LevelKind;
use kfx_formats::strings::Lang;
use kfx_prefab::i18n::{trd, trf};
use serde::{Deserialize, Serialize};

/// Open windows of the campaign editor and the Workshop (kept with the window layout).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct CampaignLayout {
    pub campaign_open: bool,
    /// The campaign / map pack `.cfg` last open in the campaign editor (reopened with the window).
    pub campaign_cfg: Option<PathBuf>,
    pub workshop_open: bool,
    /// `browse`, `installed` or `publish`.
    pub workshop_tab: String,
}

/// Workshop preferences.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct WorkshopSettings {
    /// The user has read that the Workshop window contacts keeperfx.net (asked before the first
    /// request).
    pub acknowledged: bool,
    /// Use only the cache, never the network.
    pub offline: bool,
}

/// A campaign or map pack of the install, for the pickers.
#[derive(Clone, Debug)]
pub struct PackChoice {
    pub cfg: PathBuf,
    pub kind: PackKind,
    /// `NAME (campgns/keeporig.cfg)`.
    pub label: String,
    /// The pack has a land view folder (campaigns: a wizard source).
    pub has_land: bool,
}

/// Every `campgns/*.cfg`, `levels/*.cfg` and `multiplayer/*.cfg` of `install`, campaigns first.
pub fn install_packs(install: &Path) -> Vec<PackChoice> {
    let mut out = vec![];
    for kind in [
        PackKind::Campaign,
        PackKind::Mappack,
        PackKind::MultiplayerMappack,
    ] {
        let dir = kfx_campaign_ui::pack::resolve_loose(install, kind.folder());
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut cfgs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cfg")))
            .collect();
        cfgs.sort();
        for cfg in cfgs {
            let file = cfg
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            let pack = Pack::load(install, &cfg).ok();
            let name = pack
                .as_ref()
                .and_then(|p| p.common().name)
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| file.clone());
            let has_land = pack
                .as_ref()
                .and_then(|p| p.land_dir())
                .is_some_and(|d| d.is_dir());
            out.push(PackChoice {
                label: format!("{name}  ({}/{file})", kind.folder()),
                cfg,
                kind,
                has_land,
            });
        }
    }
    out
}

/// The install a pack cfg belongs to: the folder holding its `campgns/` / `levels/` /
/// `multiplayer/` folder.
pub fn pack_install(cfg: &Path) -> Option<PathBuf> {
    kfx_formats::campaign::kind_of(cfg)?;
    cfg.parent()?.parent().map(Path::to_path_buf)
}

/// The "New campaign / map pack" wizard.
pub struct Wizard {
    /// File name without `.cfg`; also the start of the pack's folder names.
    pub name: String,
    pub display_name: String,
    pub kind: PackKind,
    /// The campaign whose land view is copied (campaigns) or referenced (map packs).
    pub land_from: Option<PathBuf>,
    /// Extra string table languages (English is always made).
    pub languages: Vec<Lang>,
    /// Add the open level (as saved) as the first level.
    pub add_current: bool,
    /// Where the pack is made: the editor's install (tests and `--actions` may point elsewhere).
    pub install: PathBuf,
    pub error: Option<String>,
    sources: Vec<PackChoice>,
}

impl Wizard {
    pub fn new(install: &Path, add_current: bool) -> Wizard {
        let sources: Vec<PackChoice> = install_packs(install)
            .into_iter()
            .filter(|p| p.kind == PackKind::Campaign && p.has_land)
            .collect();
        let land_from = sources
            .iter()
            .find(|s| {
                s.cfg
                    .file_stem()
                    .is_some_and(|f| f.eq_ignore_ascii_case("keeporig"))
            })
            .or(sources.first())
            .map(|s| s.cfg.clone());
        Wizard {
            name: String::new(),
            display_name: String::new(),
            kind: PackKind::Campaign,
            land_from,
            languages: vec![],
            add_current,
            install: install.to_path_buf(),
            error: None,
            sources,
        }
    }

    /// Why the pack cannot be made with these fields, `None` when it can.
    pub fn problem(&self) -> Option<String> {
        if let Err(e) = valid_pack_name(&self.name) {
            return Some(trf(
                "kfx_editor.file_name_e",
                "File name: {e}.",
                &[("e", &e)],
            ));
        }
        if self.display_name.trim().is_empty() {
            return Some(
                trd(
                    "kfx_editor.give_the_pack_a_name_to_show_in_the_game_s_menu",
                    "Give the pack a name to show in the game's menu.",
                )
                .into(),
            );
        }
        let group = kfx_campaign_ui::pack::resolve_loose(&self.install, self.kind.folder());
        if kfx_formats::testutil::resolve_ci(&group, &format!("{}.cfg", self.name)).is_some() {
            return Some(trf(
                "kfx_editor.kind_name_cfg_already_exists",
                "{kind}/{name}.cfg already exists.",
                &[("kind", &self.kind.folder()), ("name", &self.name)],
            ));
        }
        None
    }

    fn spec(&self) -> NewPack {
        NewPack {
            name: self.name.clone(),
            display_name: self.display_name.trim().to_string(),
            kind: self.kind,
            land_from: self.land_from.clone(),
            languages: self.languages.clone(),
        }
    }
}

/// "Add current level to campaign".
pub struct AddDialog {
    pub kind: LevelKind,
}

/// State of the campaign editor and Workshop windows.
#[derive(Default)]
pub struct Hub {
    pub editor: Option<CampaignEditor>,
    /// The last campaign was tried once after start (it may be gone).
    tried_last: bool,
    /// Packs of the install for the pickers (made when first needed, cleared after changes).
    packs: Option<Vec<PackChoice>>,
    pub wizard: Option<Wizard>,
    pub add: Option<AddDialog>,
    /// Another pack was chosen while the open one has unsaved changes.
    switch_to: Option<PathBuf>,
    /// Asked on exit: the campaign has unsaved changes.
    exit_prompt: bool,
    allow_close: bool,
    // workshop (app/workshop_win.rs)
    pub workshop: Option<kfx_workshop::ui::WorkshopState>,
    /// Used instead of the default when the Workshop state is made (tests, `--actions`).
    pub workshop_config: Option<kfx_workshop::ui::WorkshopConfig>,
    pub workshop_client: Option<Arc<kfx_workshop::client::Client>>,
    /// Never use the network and never start programs (headless runs, tests).
    pub dry_run: bool,
    /// URLs the Workshop asked to open (shown in the status; logged in dry runs).
    pub opened_urls: Vec<String>,
}

enum Act {
    Open(PathBuf),
    OwnPack,
    Wizard,
    Add,
    SaveAndSwitch,
    DiscardAndSwitch,
    CancelSwitch,
    Close,
}

impl EditorApp {
    /// The campaign editor of the window, if a pack is open in it.
    #[cfg(test)]
    pub fn campaign_editor(&mut self) -> Option<&mut CampaignEditor> {
        self.camp.editor.as_mut()
    }

    /// The campaign has unsaved changes (cfg or texts).
    pub fn campaign_dirty(&self) -> bool {
        self.camp
            .editor
            .as_ref()
            .is_some_and(|e| e.pack.is_dirty() || e.texts.is_dirty())
    }

    fn camp_packs(&mut self) -> Vec<PackChoice> {
        if self.camp.packs.is_none() {
            self.camp.packs = Some(self.install().map(install_packs).unwrap_or_default());
        }
        self.camp.packs.clone().unwrap_or_default()
    }

    /// Show or hide the campaign editor window.
    pub fn show_campaign_window(&mut self, open: bool) {
        self.settings.layout.campaign.campaign_open = open;
    }

    /// Open a campaign / map pack `.cfg` in the campaign editor (and show the window). With
    /// unsaved changes in the open pack, the window asks first.
    pub fn open_campaign_cfg(&mut self, cfg: &Path) -> Result<(), String> {
        self.settings.layout.campaign.campaign_open = true;
        if self
            .camp
            .editor
            .as_ref()
            .is_some_and(|e| extras::same_dir(&e.pack.cfg_path, cfg))
        {
            return Ok(());
        }
        if self.campaign_dirty() {
            self.camp.switch_to = Some(cfg.to_path_buf());
            return Err(trd(
                "kfx_editor.the_open_campaign_has_unsaved_changes_save_or_di",
                "The open campaign has unsaved changes: save or discard them first",
            ));
        }
        self.load_campaign(cfg)
    }

    fn load_campaign(&mut self, cfg: &Path) -> Result<(), String> {
        let install = pack_install(cfg).ok_or_else(|| {
            trf(
                "kfx_editor.cfg_is_not_in_a_campgns_levels_or_multiplayer_fo",
                "{cfg} is not in a campgns, levels or multiplayer folder",
                &[("cfg", &cfg.display())],
            )
        })?;
        let pack = Pack::load(&install, cfg).map_err(|e| format!("{}: {e}", cfg.display()))?;
        let mut ed = CampaignEditor::new(pack);
        ed.package_path = crate::settings::config_dir()
            .map(|d| {
                d.join("packages")
                    .join(format!("{}.zip", ed.pack.stem()))
                    .display()
                    .to_string()
            })
            .unwrap_or_default();
        self.camp.editor = Some(ed);
        self.camp.switch_to = None;
        self.settings.layout.campaign.campaign_cfg = Some(cfg.to_path_buf());
        self.settings.layout.campaign.campaign_open = true;
        self.status = trf(
            "kfx_editor.campaign_editor_cfg",
            "Campaign editor: {cfg}",
            &[("cfg", &cfg.display())],
        );
        Ok(())
    }

    /// Edit the campaign / map pack of the open level.
    pub fn open_own_campaign(&mut self) -> Result<(), String> {
        let cfg = self
            .doc
            .as_ref()
            .and_then(|d| d.pack_cfg.as_ref().map(|(p, _)| p.clone()))
            .ok_or(trd(
                "kfx_editor.the_open_map_is_not_in_a_campaign_or_map_pack_a921",
                "The open map is not in a campaign or map pack",
            ))?;
        self.open_campaign_cfg(&cfg)
    }

    /// Open the wizard (fields empty).
    pub fn open_campaign_wizard(&mut self) {
        let Some(install) = self.install().map(Path::to_path_buf) else {
            self.status = trd(
                "kfx_editor.set_the_keeperfx_folder_first",
                "Set the KeeperFX folder first",
            );
            return;
        };
        let can_add = self.doc_on_disk && self.doc.is_some();
        self.camp.wizard = Some(Wizard::new(&install, can_add));
    }

    /// The wizard (tests).
    #[cfg(test)]
    pub fn campaign_wizard(&mut self) -> Option<&mut Wizard> {
        self.camp.wizard.as_mut()
    }

    /// Make the pack the wizard describes, open it in the campaign editor and (when asked) add
    /// the open level to it.
    pub fn campaign_wizard_create(&mut self) -> Result<(), String> {
        let Some(w) = self.camp.wizard.as_mut() else {
            return Err(trd(
                "kfx_editor.the_wizard_is_not_open",
                "the wizard is not open",
            ));
        };
        if let Some(p) = w.problem() {
            w.error = Some(p.clone());
            return Err(p);
        }
        let spec = w.spec();
        let install = w.install.clone();
        let add_current = w.add_current;
        let pack = match kfx_campaign_ui::create_pack(&install, &spec) {
            Ok(p) => p,
            Err(e) => {
                let m = trf(
                    "kfx_editor.the_pack_was_not_made_e",
                    "The pack was not made: {e}",
                    &[("e", &e)],
                );
                w.error = Some(m.clone());
                return Err(m);
            }
        };
        let cfg = pack.cfg_path.clone();
        self.camp.wizard = None;
        self.camp.packs = None;
        self.browser.packs = None;
        // the new pack replaces what is open (a dirty pack stays open and is asked about)
        if self.campaign_dirty() {
            self.camp.switch_to = Some(cfg.clone());
            self.settings.layout.campaign.campaign_open = true;
            self.status = trf(
                "kfx_editor.made_cfg_save_or_discard_the_open_campaign_to_sw",
                "Made {cfg}; save or discard the open campaign to switch to it",
                &[("cfg", &cfg.display())],
            );
            return Ok(());
        }
        self.load_campaign(&cfg)?;
        let mut note = String::new();
        if add_current && self.doc.is_some() {
            let kind = if spec.kind == PackKind::MultiplayerMappack {
                LevelKind::Multi
            } else if spec.kind == PackKind::Campaign {
                LevelKind::Single
            } else {
                LevelKind::Free
            };
            match self.add_current_level(kind) {
                Ok(n) => {
                    if let Some(ed) = self.camp.editor.as_mut() {
                        ed.save();
                    }
                    note = trf(
                        "kfx_editor.with_the_open_level_as_level_n",
                        " with the open level as level {n}",
                        &[("n", &n)],
                    );
                }
                Err(e) => {
                    note = trf(
                        "kfx_editor.the_open_level_was_not_added_e",
                        " (the open level was not added: {e})",
                        &[("e", &e)],
                    )
                }
            }
        }
        self.status = trf(
            "kfx_editor.made_cfg_note",
            "Made {cfg}{note}",
            &[("cfg", &cfg.display()), ("note", &note)],
        );
        Ok(())
    }

    /// Add the open level (its files on disk) to the pack open in the campaign editor. A level
    /// already in the pack's levels folder keeps its number; one from elsewhere is copied with
    /// the next free number. The pack's cfg changes in memory (Save writes it).
    pub fn add_current_level(&mut self, kind: LevelKind) -> Result<u32, String> {
        let d = self
            .doc
            .as_ref()
            .ok_or(trd("kfx_editor.no_map_is_open", "No map is open"))?;
        if !self.doc_on_disk {
            return Err(trd(
                "kfx_editor.save_the_map_first_its_saved_files_are_what_is_a",
                "Save the map first: its saved files are what is added",
            ));
        }
        if d.is_dirty() || self.scripts_dirty() {
            return Err(trd(
                "kfx_editor.save_the_map_first_its_saved_files_are_what_is_a",
                "Save the map first: its saved files are what is added",
            ));
        }
        let (folder, level) = (d.doc.folder.clone(), d.doc.level);
        let ed = self.camp.editor.as_mut().ok_or(trd(
            "kfx_editor.open_a_campaign_or_map_pack_in_the_campaign_edit",
            "Open a campaign or map pack in the campaign editor first",
        ))?;
        let mut req = AddLevel::new(LevelSource::new(&folder, level), kind);
        if ed
            .pack
            .levels_dir()
            .is_some_and(|l| extras::same_dir(&l, &folder))
        {
            if ed.pack.entries().iter().any(|e| e.number == level) {
                return Err(trf(
                    "kfx_editor.level_level_is_already_in_pack",
                    "Level {level} is already in {pack}",
                    &[("level", &level), ("pack", &ed.pack.stem())],
                ));
            }
            // already in the levels folder: listed under its own number, the files stay
            req.number = Some(level);
            req.overwrite = true;
        }
        let added = ed.pack.add_level(&req).map_err(|e| e.to_string())?;
        ed.selected = Some(added.number);
        ed.revalidate();
        ed.status = trf(
            "kfx_editor.added_map_level_as_level_number_save_writes_the",
            "added map{level} as level {number}; Save writes the campaign file",
            &[
                ("level", &format!("{:05}", level)),
                ("number", &added.number),
            ],
        );
        self.status = trf(
            "kfx_editor.added_map_level_to_pack_as_level_number",
            "Added map{level} to {pack} as level {number}",
            &[
                ("level", &format!("{:05}", level)),
                ("pack", &ed.pack.stem()),
                ("number", &added.number),
            ],
        );
        Ok(added.number)
    }

    /// Set a level's ensign position the way dragging it on the land view does (the zoom point
    /// follows while it was on the ensign).
    pub fn campaign_set_ensign(&mut self, level: u32, pos: (i32, i32)) -> Result<(), String> {
        let ed = self
            .camp
            .editor
            .as_mut()
            .ok_or(trd("kfx_editor.no_campaign_is_open", "no campaign is open"))?;
        if !ed.pack.entries().iter().any(|e| e.number == level) {
            return Err(trf(
                "kfx_editor.level_level_is_not_in_the_pack",
                "level {level} is not in the pack",
                &[("level", &level)],
            ));
        }
        let mut info = ed.pack.level_info(level);
        let old = info.ensign_pos;
        info.ensign_pos = Some(pos);
        if ed.link_zoom && (info.ensign_zoom.is_none() || info.ensign_zoom == old) {
            info.ensign_zoom = Some(pos);
        }
        ed.pack.set_level_info(level, &info);
        ed.selected = Some(level);
        ed.revalidate();
        Ok(())
    }

    // ------------------------------------------------------------------------------- UI

    /// The Campaign menu.
    pub(super) fn campaign_menu(&mut self, ui: &mut egui::Ui) {
        let has_data = self.install().is_some();
        let k = self.keymap.clone();
        if ui
            .add_enabled(
                has_data,
                egui::Button::new(trd("kfx_editor.campaign_editor_0918", "Campaign editor…"))
                    .shortcut_text(k.label("campaign.editor")),
            )
            .clicked()
        {
            self.settings.layout.campaign.campaign_open = true;
            ui.close();
        }
        let own = self.doc.as_ref().and_then(|d| d.pack_cfg.clone());
        let r = ui
            .add_enabled(
                own.is_some(),
                egui::Button::new(trd(
                    "kfx_editor.campaign_of_this_level",
                    "Campaign of this level…",
                )),
            )
            .on_disabled_hover_text(trd(
                "kfx_editor.the_open_map_is_not_in_a_campaign_or_map_pack",
                "The open map is not in a campaign or map pack.",
            ));
        if r.clicked() {
            if let Err(e) = self.open_own_campaign() {
                self.status = e;
            }
            ui.close();
        }
        ui.add_enabled_ui(has_data, |ui| {
            ui.menu_button(
                trd(
                    "kfx_editor.open_campaign_or_map_pack",
                    "Open campaign or map pack",
                ),
                |ui| {
                    let packs = self.camp_packs();
                    egui::ScrollArea::vertical()
                        .max_height(420.0)
                        .show(ui, |ui| {
                            for p in packs {
                                if ui.button(&p.label).clicked() {
                                    if let Err(e) = self.open_campaign_cfg(&p.cfg) {
                                        self.status = e;
                                    }
                                    ui.close();
                                }
                            }
                        });
                },
            );
        });
        if ui
            .add_enabled(
                has_data,
                egui::Button::new(trd(
                    "kfx_editor.new_campaign_map_pack_1f7d",
                    "New campaign / map pack…",
                )),
            )
            .clicked()
        {
            self.open_campaign_wizard();
            ui.close();
        }
        let can_add = self.doc.is_some() && self.camp.editor.is_some();
        let r = ui
            .add_enabled(
                can_add,
                egui::Button::new(trd(
                    "kfx_editor.add_current_level_to_campaign_5daa",
                    "Add current level to campaign…",
                )),
            )
            .on_disabled_hover_text(trd(
                "kfx_editor.open_a_map_and_a_campaign_in_the_campaign_editor",
                "Open a map and a campaign in the campaign editor first.",
            ));
        if r.clicked() {
            self.open_add_dialog();
            ui.close();
        }
        ui.separator();
        if ui
            .add(
                egui::Button::new(trd("kfx_editor.workshop_b168", "Workshop…"))
                    .shortcut_text(k.label("workshop.open")),
            )
            .on_hover_text(trd(
                "kfx_editor.browse_install_and_package_keeperfx_workshop_ite",
                "Browse, install and package KeeperFX Workshop items.",
            ))
            .clicked()
        {
            self.settings.layout.campaign.workshop_open = true;
            ui.close();
        }
    }

    fn open_add_dialog(&mut self) {
        let campaign = self
            .camp
            .editor
            .as_ref()
            .is_some_and(|e| e.pack.is_campaign());
        self.camp.add = Some(AddDialog {
            kind: if campaign {
                LevelKind::Single
            } else {
                LevelKind::Free
            },
        });
    }

    /// Shortcuts of the campaign editor and the Workshop (global).
    pub(super) fn campaign_shortcuts(&mut self, ctx: &egui::Context) {
        let k = self.keymap.clone();
        if k.pressed(ctx, "campaign.editor") && self.install().is_some() {
            let l = &mut self.settings.layout.campaign;
            l.campaign_open = !l.campaign_open;
        }
        if k.pressed(ctx, "workshop.open") {
            let l = &mut self.settings.layout.campaign;
            l.workshop_open = !l.workshop_open;
        }
    }

    /// Every window of this module and the Workshop's, once per frame.
    pub(super) fn campaign_ui(&mut self, ctx: &egui::Context) {
        self.campaign_exit_guard(ctx);
        self.campaign_window(ctx);
        self.campaign_wizard_ui(ctx);
        self.campaign_add_ui(ctx);
        self.workshop_window(ctx);
    }

    /// Closing the editor with unsaved campaign changes asks first (after the map's own prompt).
    fn campaign_exit_guard(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested())
            && !self.camp.allow_close
            && self.pending.is_none()
            && self.campaign_dirty()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.camp.exit_prompt = true;
        }
        if !self.camp.exit_prompt {
            return;
        }
        let name = self
            .camp
            .editor
            .as_ref()
            .map(|e| e.pack.stem())
            .unwrap_or_default();
        let mut answer = None;
        egui::Modal::new(egui::Id::unique("kfx-campaign-unsaved")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading(
                RichText::new(trd(
                    "kfx_editor.unsaved_campaign_changes",
                    "Unsaved campaign changes",
                ))
                .color(GOLD),
            );
            ui.label(trf(
                "kfx_editor.the_campaign_name_has_unsaved_changes_save_them",
                "The campaign {name} has unsaved changes. Save them before closing?",
                &[("name", &name)],
            ));
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .button(trd("kfx_editor.save_the_campaign", "Save the campaign"))
                    .clicked()
                {
                    answer = Some(0);
                }
                if ui
                    .button(trd(
                        "kfx_editor.discard_campaign_changes",
                        "Discard campaign changes",
                    ))
                    .clicked()
                {
                    answer = Some(1);
                }
                if ui.button(trd("kfx_editor.cancel", "Cancel")).clicked() {
                    answer = Some(2);
                }
            });
        });
        match answer {
            Some(0) => {
                if let Some(ed) = self.camp.editor.as_mut() {
                    ed.save();
                }
                self.camp.exit_prompt = false;
                if !self.campaign_dirty() {
                    self.camp.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Some(1) => {
                self.camp.exit_prompt = false;
                self.camp.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(2) => self.camp.exit_prompt = false,
            _ => {}
        }
    }

    fn campaign_window(&mut self, ctx: &egui::Context) {
        if !self.settings.layout.campaign.campaign_open {
            return;
        }
        if self.camp.editor.is_none() && !self.camp.tried_last {
            self.camp.tried_last = true;
            if let Some(cfg) = self
                .settings
                .layout
                .campaign
                .campaign_cfg
                .clone()
                .filter(|p| p.is_file())
            {
                if let Err(e) = self.load_campaign(&cfg) {
                    self.status = e;
                }
            }
        }
        let packs = self.camp_packs();
        let own = self
            .doc
            .as_ref()
            .and_then(|d| d.pack_cfg.as_ref().map(|(p, _)| p.clone()));
        let can_add = self.doc.is_some() && self.camp.editor.is_some();
        let mut open = true;
        let mut act: Option<Act> = None;
        let mut events: Vec<CampEvent> = vec![];
        let max = ctx.content_rect().size() - egui::vec2(40.0, 80.0);
        let size =
            egui::vec2(1120.0f32.min(max.x), 720.0f32.min(max.y)).max(egui::vec2(400.0, 300.0));
        egui::Window::new(trd("kfx_editor.campaign_editor", "Campaign editor"))
            .id(egui::Id::unique("kfx-campaign-editor"))
            .open(&mut open)
            .default_size(size)
            .default_pos(ctx.content_rect().center() - size / 2.0)
            .min_size([400.0, 300.0])
            .show(ctx, |ui| {
                let cur = self.camp.editor.as_ref().map(|e| e.pack.cfg_path.clone());
                ui.horizontal(|ui| {
                    ui.label(trd("kfx_editor.pack", "Pack:"));
                    let label = cur
                        .as_ref()
                        .and_then(|c| {
                            packs
                                .iter()
                                .find(|p| extras::same_dir(&p.cfg, c))
                                .map(|p| p.label.clone())
                                .or_else(|| Some(c.display().to_string()))
                        })
                        .unwrap_or_else(|| {
                            trd(
                                "kfx_editor.choose_a_campaign_or_map_pack",
                                "Choose a campaign or map pack",
                            )
                            .into()
                        });
                    egui::ComboBox::from_id_salt("kfx-camp-pick")
                        .selected_text(label)
                        .width(340.0)
                        .show_ui(ui, |ui| {
                            for p in &packs {
                                let on = cur.as_ref().is_some_and(|c| extras::same_dir(c, &p.cfg));
                                if ui.selectable_label(on, &p.label).clicked() {
                                    act = Some(Act::Open(p.cfg.clone()));
                                }
                            }
                        });
                    if ui
                        .add_enabled(
                            own.is_some(),
                            egui::Button::new(trd(
                                "kfx_editor.this_level_s_campaign",
                                "This level's campaign",
                            )),
                        )
                        .on_disabled_hover_text(trd(
                            "kfx_editor.the_open_map_is_not_in_a_campaign_or_map_pack",
                            "The open map is not in a campaign or map pack.",
                        ))
                        .clicked()
                    {
                        act = Some(Act::OwnPack);
                    }
                    if ui
                        .button(trd(
                            "kfx_editor.new_campaign_map_pack_1f7d",
                            "New campaign / map pack…",
                        ))
                        .clicked()
                    {
                        act = Some(Act::Wizard);
                    }
                    if ui
                        .add_enabled(
                            can_add,
                            egui::Button::new(trd(
                                "kfx_editor.add_current_level",
                                "Add current level…",
                            )),
                        )
                        .on_disabled_hover_text(trd(
                            "kfx_editor.open_a_map_and_a_pack_first",
                            "Open a map and a pack first.",
                        ))
                        .clicked()
                    {
                        act = Some(Act::Add);
                    }
                    if ui
                        .add_enabled(
                            cur.is_some(),
                            egui::Button::new(trd("kfx_editor.close_pack", "Close pack")),
                        )
                        .clicked()
                    {
                        act = Some(Act::Close);
                    }
                });
                if let Some(to) = &self.camp.switch_to {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(
                            egui::Color32::from_rgb(235, 170, 70),
                            trf(
                                "kfx_editor.the_open_pack_has_unsaved_changes_switch_to_to",
                                "The open pack has unsaved changes. Switch to {to}?",
                                &[("to", &to.file_name().unwrap_or_default().to_string_lossy())],
                            ),
                        );
                        if ui
                            .button(trd("kfx_editor.save_and_switch", "Save and switch"))
                            .clicked()
                        {
                            act = Some(Act::SaveAndSwitch);
                        }
                        if ui
                            .button(trd("kfx_editor.discard_and_switch", "Discard and switch"))
                            .clicked()
                        {
                            act = Some(Act::DiscardAndSwitch);
                        }
                        if ui.button(trd("kfx_editor.stay", "Stay")).clicked() {
                            act = Some(Act::CancelSwitch);
                        }
                    });
                }
                ui.separator();
                match self.camp.editor.as_mut() {
                    Some(ed) => {
                        // the widget lays out panels in the space it is given
                        ui.set_min_size(ui.available_size());
                        events = ed.show(ui);
                    }
                    None => {
                        ui.add_space(20.0);
                        ui.label(trd(
                            "kfx_editor.choose_a_campaign_or_map_pack_above_or_make_a_ne",
                            "Choose a campaign or map pack above, or make a new one with \
                            the wizard.",
                        ));
                        ui.label(
                            RichText::new(trd(
                                "kfx_editor.campaigns_list_their_levels_and_land_view_in_cam",
                                "Campaigns list their levels and land view in \
                                campgns/<name>.cfg; map packs (levels/, multiplayer/) list theirs \
                                with \
                                .lof files.",
                            ))
                            .color(TEXT_DIM),
                        );
                    }
                }
            });
        if !open {
            self.settings.layout.campaign.campaign_open = false;
        }
        match act {
            Some(Act::Open(p)) => {
                if let Err(e) = self.open_campaign_cfg(&p) {
                    self.status = e;
                }
            }
            Some(Act::OwnPack) => {
                if let Err(e) = self.open_own_campaign() {
                    self.status = e;
                }
            }
            Some(Act::Wizard) => self.open_campaign_wizard(),
            Some(Act::Add) => self.open_add_dialog(),
            Some(Act::SaveAndSwitch) => {
                if let Some(ed) = self.camp.editor.as_mut() {
                    ed.save();
                }
                if !self.campaign_dirty() {
                    if let Some(to) = self.camp.switch_to.take() {
                        if let Err(e) = self.load_campaign(&to) {
                            self.status = e;
                        }
                    }
                }
            }
            Some(Act::DiscardAndSwitch) => {
                if let Some(to) = self.camp.switch_to.take() {
                    self.camp.editor = None;
                    if let Err(e) = self.load_campaign(&to) {
                        self.status = e;
                    }
                }
            }
            Some(Act::CancelSwitch) => self.camp.switch_to = None,
            Some(Act::Close) => {
                if self.campaign_dirty() {
                    // closing a pack with unsaved changes needs the same choice
                    self.status = trd(
                        "kfx_editor.the_pack_has_unsaved_changes_save_it_or_discard",
                        "The pack has unsaved changes: Save it (or discard by \
                        switching packs) first",
                    );
                } else {
                    self.camp.editor = None;
                    self.settings.layout.campaign.campaign_cfg = None;
                }
            }
            None => {}
        }
        for e in events {
            self.campaign_event(e);
        }
    }

    fn campaign_event(&mut self, e: CampEvent) {
        match e {
            CampEvent::OpenLevel { folder, number } => {
                match level_bin::find_file(&folder, number, "slb") {
                    Some(slb) => self.request(Pending::Open(slb)),
                    None => {
                        self.status = trf(
                            "kfx_editor.map_number_slb_is_not_in_folder",
                            "map{number}.slb is not in {folder}",
                            &[
                                ("number", &format!("{:05}", number)),
                                ("folder", &folder.display()),
                            ],
                        )
                    }
                }
            }
            CampEvent::PlayLevel { campaign, number } => {
                let Some(ed) = self.camp.editor.as_ref() else {
                    return;
                };
                if ed.pack.is_dirty() {
                    self.status = trf(
                        "kfx_editor.save_campaign_first_the_game_reads_the_campaign",
                        "Save {campaign} first: the game reads the campaign file from disk",
                        &[("campaign", &campaign)],
                    );
                    return;
                }
                match ed
                    .pack
                    .levels_dir()
                    .and_then(|d| level_bin::find_file(&d, number, "slb"))
                {
                    Some(slb) => self.play_file(&slb),
                    None => {
                        self.status = trf(
                            "kfx_editor.map_number_slb_of_campaign_not_found",
                            "map{number}.slb of {campaign} not found",
                            &[
                                ("number", &format!("{:05}", number)),
                                ("campaign", &campaign),
                            ],
                        )
                    }
                }
            }
            CampEvent::Saved => {
                // level names and lists shown elsewhere come from the files
                self.browser.packs = None;
                self.camp.packs = None;
            }
            CampEvent::Changed => {}
        }
    }

    fn campaign_wizard_ui(&mut self, ctx: &egui::Context) {
        let Some(w) = self.camp.wizard.as_mut() else {
            return;
        };
        let mut open = true;
        let mut create = false;
        let mut cancel = false;
        let can_add = self.doc.is_some() && self.doc_on_disk;
        let level = self.doc.as_ref().map(|d| d.doc.level).unwrap_or(0);
        egui::Window::new(trd(
            "kfx_editor.new_campaign_map_pack",
            "New campaign / map pack",
        ))
        .id(egui::Id::unique("kfx-campaign-wizard"))
        .open(&mut open)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .collapsible(false)
        .resizable(false)
        .default_width(560.0)
        .show(ctx, |ui| {
            egui::Grid::new("kfx-wiz")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label(trd("kfx_editor.kind", "Kind"));
                    ui.horizontal(|ui| {
                        ui.radio_value(
                            &mut w.kind,
                            PackKind::Campaign,
                            trd("kfx_editor.campaign", "Campaign"),
                        )
                        .on_hover_text(trd(
                            "kfx_editor.levels_played_in_order_on_a_land_view_campgns",
                            "Levels played in order on a land view (campgns/).",
                        ));
                        ui.radio_value(
                            &mut w.kind,
                            PackKind::Mappack,
                            trd("kfx_editor.map_pack", "Map pack"),
                        )
                        .on_hover_text(trd(
                            "kfx_editor.free_play_levels_picked_from_a_list_levels",
                            "Free play levels picked from a list (levels/).",
                        ));
                        ui.radio_value(
                            &mut w.kind,
                            PackKind::MultiplayerMappack,
                            trd("kfx_editor.multiplayer_pack", "Multiplayer pack"),
                        )
                        .on_hover_text(trd(
                            "kfx_editor.multiplayer_levels_multiplayer",
                            "Multiplayer levels (multiplayer/).",
                        ));
                    });
                    ui.end_row();
                    ui.label(trd("kfx_editor.name_in_the_menu", "Name in the menu"));
                    ui.add(
                        egui::TextEdit::singleline(&mut w.display_name)
                            .hint_text(trd("kfx_editor.my_campaign", "My campaign"))
                            .desired_width(300.0),
                    );
                    ui.end_row();
                    ui.label(trd("kfx_editor.file_name", "File name"));
                    ui.horizontal(|ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut w.name)
                                .hint_text("mycampaign")
                                .desired_width(200.0),
                        );
                        if r.changed() {
                            w.error = None;
                        }
                        ui.label(RichText::new(".cfg").color(TEXT_DIM));
                        if w.name.is_empty()
                            && !w.display_name.is_empty()
                            && ui
                                .small_button(trd("kfx_editor.from_the_name", "From the name"))
                                .clicked()
                        {
                            w.name = w
                                .display_name
                                .chars()
                                .filter(|c| c.is_ascii_alphanumeric())
                                .take(16)
                                .collect::<String>()
                                .to_ascii_lowercase();
                        }
                    });
                    ui.end_row();
                    ui.label(trd("kfx_editor.land_view", "Land view"));
                    let label = w
                        .land_from
                        .as_ref()
                        .and_then(|c| w.sources.iter().find(|s| &s.cfg == c))
                        .map(|s| s.label.clone())
                        .unwrap_or_else(|| {
                            trd(
                                "kfx_editor.none_an_empty_land_folder",
                                "None (an empty land folder)",
                            )
                            .into()
                        });
                    egui::ComboBox::from_id_salt("kfx-wiz-land")
                        .selected_text(label)
                        .width(300.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut w.land_from,
                                None,
                                trd(
                                    "kfx_editor.none_an_empty_land_folder",
                                    "None (an empty land folder)",
                                ),
                            );
                            for s in &w.sources {
                                ui.selectable_value(
                                    &mut w.land_from,
                                    Some(s.cfg.clone()),
                                    &s.label,
                                );
                            }
                        });
                    ui.end_row();
                    ui.label(trd("kfx_editor.languages", "Languages"));
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(trd(
                                "kfx_editor.english_is_always_made_tick_others_to_get_their",
                                "English is always made; tick others to get their text tables.",
                            ))
                            .small()
                            .color(TEXT_DIM),
                        );
                        ui.horizontal_wrapped(|ui| {
                            ui.set_max_width(400.0);
                            for l in Lang::ALL.iter().skip(1) {
                                let mut on = w.languages.contains(l);
                                if ui.checkbox(&mut on, l.code()).changed() {
                                    if on {
                                        w.languages.push(*l);
                                    } else {
                                        w.languages.retain(|x| x != l);
                                    }
                                }
                            }
                        });
                    });
                    ui.end_row();
                    ui.label(trd("kfx_editor.first_level", "First level"));
                    ui.add_enabled_ui(can_add, |ui| {
                        ui.checkbox(
                            &mut w.add_current,
                            if can_add {
                                trf(
                                    "kfx_editor.add_the_open_level_map_level_as_its_first_level",
                                    "Add the open level (map{level}) as its first level",
                                    &[("level", &format!("{:05}", level))],
                                )
                            } else {
                                trd(
                                    "kfx_editor.add_the_open_level_open_and_save_a_map_first",
                                    "Add the open level (open and save a map first)",
                                )
                            },
                        );
                    });
                    ui.end_row();
                });
            ui.separator();
            if !w.name.is_empty() && valid_pack_name(&w.name).is_ok() {
                let g = w.kind.folder();
                let n = &w.name;
                let mut made = trf(
                    "kfx_editor.makes_g_n_cfg_and_g_n_levels_and_text_tables",
                    "Makes {g}/{n}.cfg and {g}/{n}/ (levels and text tables)",
                    &[("g", &g), ("n", &n)],
                );
                if w.kind == PackKind::Campaign {
                    made.push_str(&format!(
                        ", {g}/{n}_lnd/ (land view) and {g}/{n}_eng/ (speech)"
                    ));
                }
                made.push_str(&format!(" in {}.", w.install.display()));
                ui.label(RichText::new(made).small().color(TEXT_DIM));
            }
            let problem = w.problem();
            if let Some(e) = w.error.clone().or(problem.clone()) {
                ui.colored_label(egui::Color32::from_rgb(235, 170, 70), e);
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        problem.is_none(),
                        egui::Button::new(trd("kfx_editor.create", "Create")),
                    )
                    .clicked()
                {
                    create = true;
                }
                if ui.button(trd("kfx_editor.cancel", "Cancel")).clicked() {
                    cancel = true;
                }
            });
            w.add_current &= can_add;
        });
        if !open || cancel {
            self.camp.wizard = None;
        } else if create {
            if let Err(e) = self.campaign_wizard_create() {
                self.status = e;
            }
        }
    }

    fn campaign_add_ui(&mut self, ctx: &egui::Context) {
        let Some(dlg) = self.camp.add.as_mut() else {
            return;
        };
        let Some(ed) = self.camp.editor.as_ref() else {
            self.camp.add = None;
            return;
        };
        let level = self.doc.as_ref().map(|d| d.doc.level).unwrap_or(0);
        let dirty = self.doc.as_ref().is_some_and(|d| d.is_dirty()) || !self.doc_on_disk;
        let pack = ed.pack.common().name.unwrap_or_else(|| ed.pack.stem());
        let kinds: &[LevelKind] = if ed.pack.is_campaign() {
            &[LevelKind::Single, LevelKind::Bonus, LevelKind::Extra]
        } else {
            &[
                LevelKind::Free,
                LevelKind::Single,
                LevelKind::Multi,
                LevelKind::Bonus,
                LevelKind::Extra,
            ]
        };
        let mut open = true;
        let mut add = false;
        egui::Window::new(trd(
            "kfx_editor.add_current_level_to_campaign",
            "Add current level to campaign",
        ))
        .id(egui::Id::unique("kfx-campaign-add"))
        .open(&mut open)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(trf(
                "kfx_editor.add_map_level_to_pack",
                "Add map{level} to {pack}.",
                &[("level", &format!("{:05}", level)), ("pack", &pack)],
            ));
            ui.horizontal(|ui| {
                ui.label(trd("kfx_editor.as", "As"));
                for k in kinds {
                    ui.radio_value(&mut dlg.kind, *k, k.name());
                }
            });
            ui.label(
                RichText::new(trd(
                    "kfx_editor.a_level_from_another_folder_is_copied_into_the_p",
                    "A level from another folder is copied into the pack with the next \
                    free number; one already in the pack's folder keeps its number.",
                ))
                .small()
                .color(TEXT_DIM),
            );
            if dirty {
                ui.colored_label(
                    egui::Color32::from_rgb(235, 170, 70),
                    trd(
                        "kfx_editor.save_the_map_first_its_saved_files_are_what_is_a_4043",
                        "Save the map first: its saved files are what is added.",
                    ),
                );
            }
            if ui
                .add_enabled(!dirty, egui::Button::new(trd("kfx_editor.add", "Add")))
                .clicked()
            {
                add = true;
            }
        });
        let kind = dlg.kind;
        if !open {
            self.camp.add = None;
        }
        if add {
            match self.add_current_level(kind) {
                Ok(_) => self.camp.add = None,
                Err(e) => self.status = e,
            }
        }
    }
}

// ----------------------------------------------------------------------------------- --actions

fn tab_of(s: &str) -> Option<CampTab> {
    Some(match s {
        "overview" => CampTab::Overview,
        "levels" => CampTab::Levels,
        "land" | "land_view" | "landview" => CampTab::LandView,
        "texts" => CampTab::Texts,
        "health" => CampTab::Health,
        _ => return None,
    })
}

fn pack_kind(s: &str) -> Option<PackKind> {
    Some(match s {
        "campaign" => PackKind::Campaign,
        "mappack" | "map_pack" | "levels" => PackKind::Mappack,
        "multiplayer" | "multi" => PackKind::MultiplayerMappack,
        _ => return None,
    })
}

/// `--actions` of the campaign editor and the Workshop (see docs/manual/campaigns.md and
/// workshop.md). Nothing here contacts the network: the Workshop is always offline in a
/// headless run.
pub fn run_actions(
    app: &mut EditorApp,
    obj: &serde_json::Map<String, serde_json::Value>,
    rel: &dyn Fn(&str) -> PathBuf,
) -> Result<(), String> {
    let get = |k: &str| obj.get(k);
    if let Some(v) = get("campaign").and_then(|v| v.as_object()) {
        app.camp.dry_run = true;
        match v.get("open") {
            Some(serde_json::Value::String(s)) if s == "own" => app.open_own_campaign()?,
            Some(serde_json::Value::String(s)) => {
                let p = rel(s);
                let p = if p.is_file() {
                    p
                } else {
                    app.install()
                        .map(|i| kfx_campaign_ui::pack::resolve_loose(i, s))
                        .filter(|p| p.is_file())
                        .ok_or_else(|| format!("no campaign file {s}"))?
                };
                app.open_campaign_cfg(&p)?;
            }
            Some(serde_json::Value::Bool(b)) => app.show_campaign_window(*b),
            _ => {}
        }
        let ed = || "no campaign is open".to_string();
        if let Some(t) = v.get("tab").and_then(|x| x.as_str()) {
            let t = tab_of(t).ok_or_else(|| format!("unknown campaign tab {t}"))?;
            app.camp.editor.as_mut().ok_or_else(ed)?.tab = t;
        }
        if let Some(n) = v.get("select").and_then(|x| x.as_u64()) {
            app.camp.editor.as_mut().ok_or_else(ed)?.selected = Some(n as u32);
        }
        if let Some(a) = v.get("ensign").and_then(|x| x.as_array()) {
            let n: Vec<i64> = a.iter().filter_map(|x| x.as_i64()).collect();
            if n.len() != 3 {
                return Err(trd(
                    "kfx_editor.ensign_needs_level_x_y",
                    "ensign needs [level, x, y]",
                ));
            }
            app.campaign_set_ensign(n[0] as u32, (n[1] as i32, n[2] as i32))?;
        }
        if let Some(a) = v.get("ensign_by").and_then(|x| x.as_array()) {
            // move by a picture offset, like a drag
            let n: Vec<i64> = a.iter().filter_map(|x| x.as_i64()).collect();
            if n.len() != 3 {
                return Err(trd(
                    "kfx_editor.ensign_by_needs_level_dx_dy",
                    "ensign_by needs [level, dx, dy]",
                ));
            }
            let e = app.camp.editor.as_ref().ok_or_else(ed)?;
            let (x, y) = e
                .pack
                .level_info(n[0] as u32)
                .ensign_pos
                .unwrap_or((640, 480));
            app.campaign_set_ensign(n[0] as u32, (x + n[1] as i32, y + n[2] as i32))?;
        }
        if let Some(k) = v.get("add_current").and_then(|x| x.as_str()) {
            let kind = LevelKind::from_name(k).ok_or_else(|| format!("unknown level kind {k}"))?;
            app.add_current_level(kind)?;
        }
        if v.get("save").and_then(|x| x.as_bool()) == Some(true) {
            let e = app.camp.editor.as_mut().ok_or_else(ed)?;
            e.save();
            if e.pack.is_dirty() {
                return Err(e.status.clone());
            }
        }
        if v.get("close").and_then(|x| x.as_bool()) == Some(true) {
            app.show_campaign_window(false);
        }
    }
    if let Some(v) = get("campaign_wizard").and_then(|v| v.as_object()) {
        app.open_campaign_wizard();
        if let Some(i) = v.get("install").and_then(|x| x.as_str()) {
            let root = rel(i);
            let add = app.camp.wizard.as_ref().is_some_and(|w| w.add_current);
            app.camp.wizard = Some(Wizard::new(&root, add));
        }
        let w = app
            .camp
            .wizard
            .as_mut()
            .ok_or("the wizard needs a KeeperFX folder")?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        if let Some(x) = s("name") {
            w.name = x;
        }
        if let Some(x) = s("display_name") {
            w.display_name = x;
        }
        if let Some(x) = s("kind") {
            w.kind = pack_kind(&x).ok_or_else(|| format!("unknown pack kind {x}"))?;
        }
        if let Some(x) = v.get("land_from") {
            w.land_from = match x.as_str() {
                None | Some("") | Some("none") => None,
                Some(n) => Some(
                    w.sources
                        .iter()
                        .find(|p| p.cfg.file_stem().is_some_and(|f| f.eq_ignore_ascii_case(n)))
                        .ok_or_else(|| format!("no campaign {n} with a land view"))?
                        .cfg
                        .clone(),
                ),
            };
        }
        if let Some(a) = v.get("languages").and_then(|x| x.as_array()) {
            w.languages = a
                .iter()
                .filter_map(|x| x.as_str())
                .filter_map(Lang::from_code)
                .filter(|l| *l != Lang::English)
                .collect();
        }
        if let Some(b) = v.get("add_current").and_then(|x| x.as_bool()) {
            w.add_current = b;
        }
        if v.get("create").and_then(|x| x.as_bool()) == Some(true) {
            app.campaign_wizard_create()?;
        }
    }
    crate::app::workshop_win::run_actions(app, obj, rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_of_the_install_and_their_install() {
        let Some(inst) = kfx_formats::testutil::install_dir() else {
            return;
        };
        let packs = install_packs(&inst);
        let orig = packs
            .iter()
            .find(|p| {
                p.cfg
                    .file_stem()
                    .is_some_and(|f| f.eq_ignore_ascii_case("keeporig"))
            })
            .expect("keeporig");
        assert_eq!(orig.kind, PackKind::Campaign);
        assert!(orig.has_land);
        assert!(packs.iter().any(|p| p.kind == PackKind::Mappack));
        assert_eq!(pack_install(&orig.cfg).as_deref(), Some(inst.as_path()));
        assert_eq!(pack_install(Path::new("/x/notapack/a.cfg")), None);
    }

    #[test]
    fn wizard_problems() {
        let d =
            kfx_formats::testutil::scratch_dir(&format!("kfx-editor/wiz-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("campgns")).unwrap();
        std::fs::write(d.join("campgns/taken.cfg"), "[common]\n").unwrap();
        let mut w = Wizard::new(&d, false);
        assert!(w.problem().unwrap().contains("File name"));
        w.name = "bad name".into();
        assert!(w.problem().unwrap().contains("File name"));
        w.name = "taken".into();
        w.display_name = "Taken".into();
        assert!(w.problem().unwrap().contains("already exists"));
        w.name = "fresh".into();
        assert_eq!(w.problem(), None);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
