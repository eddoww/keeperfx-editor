//! Map browser: the install's campaigns, map packs and multiplayer packs as a tree with level
//! names, a filter, a lazily made top-down thumbnail with map facts, Edit / Play, a random
//! map, the folder in the file manager, and a new level inside a map pack.
use crate::assets::PLAYERS;
use eframe::egui::{self, Color32, RichText};
use kfx_formats::campaign::{resolve, PackKind};
use kfx_formats::levelinfo::{LevelInfo, LevelKind, Lof};
use kfx_map::gamedata::GameData;
use kfx_map::things::{self, class};
use kfx_prefab::i18n::{trd, trf, tx, Tx};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct LevelEntry {
    pub number: u32,
    pub name: String,
    pub size: (i32, i32),
    pub slb: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct PackEntry {
    pub kind: PackKind,
    pub file: String,
    pub name: String,
    pub levels: Vec<LevelEntry>,
    pub levels_dir: Option<PathBuf>,
    pub error: Option<String>,
}

/// Every `campgns/*.cfg`, `levels/*.cfg` and `multiplayer/*.cfg` of the install, resolved.
pub fn scan(install: &Path) -> Vec<PackEntry> {
    let mut out = vec![];
    for kind in [
        PackKind::Campaign,
        PackKind::Mappack,
        PackKind::MultiplayerMappack,
    ] {
        let Some(dir) = kfx_gfx::install::resolve(install, kind.folder()).filter(|d| d.is_dir())
        else {
            continue;
        };
        let mut cfgs: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cfg")))
                    .collect()
            })
            .unwrap_or_default();
        cfgs.sort();
        for p in cfgs {
            let file = p
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            match resolve(install, &p) {
                Ok(r) => {
                    // NAME_ID / NAME_TEXT_ID through the pack's string tables (lazily: most packs
                    // name their levels with NAME_TEXT)
                    let camp = kfx_formats::campaign::Campaign::load(&p).ok();
                    let mut res: Option<Option<kfx_formats::strings::StringResolver>> = None;
                    let mut by_id = |id: i64| -> Option<String> {
                        let r = res.get_or_insert_with(|| {
                            crate::files::pack_strings(install, &p, None, None)
                        });
                        r.as_ref()
                            .map(|r| r.get(id))
                            .filter(|s| !s.trim().is_empty())
                    };
                    let levels = r
                        .levels
                        .iter()
                        .map(|l| {
                            let id = camp
                                .as_ref()
                                .and_then(|c| c.map_info(l.number))
                                .and_then(|i| i.name_id)
                                .and_then(|i| i.trim().parse::<i64>().ok());
                            let name = l
                                .name
                                .clone()
                                .filter(|n| !n.trim().is_empty())
                                .or_else(|| id.and_then(&mut by_id))
                                .or_else(|| {
                                    r.levels_dir.as_deref().and_then(|d| {
                                        crate::files::level_name(install, d, l.number)
                                    })
                                })
                                .unwrap_or_default();
                            LevelEntry {
                                number: l.number,
                                name,
                                size: l.mapsize,
                                slb: l.slb.clone(),
                            }
                        })
                        .collect();
                    let name = if !r.name.is_empty() {
                        r.name.clone()
                    } else {
                        r.common
                            .name_text_id
                            .as_deref()
                            .and_then(|i| i.trim().parse::<i64>().ok())
                            .and_then(&mut by_id)
                            .unwrap_or_else(|| file.clone())
                    };
                    out.push(PackEntry {
                        kind,
                        name,
                        file,
                        levels,
                        levels_dir: r.levels_dir.clone(),
                        error: None,
                    })
                }
                Err(e) => out.push(PackEntry {
                    kind,
                    name: file.clone(),
                    file,
                    levels: vec![],
                    levels_dir: None,
                    error: Some(e.to_string()),
                }),
            }
        }
    }
    out
}

fn kind_label(k: PackKind) -> Tx {
    match k {
        PackKind::Campaign => tx("kfx_editor.campaigns", "Campaigns"),
        PackKind::Mappack => tx("kfx_editor.map_packs", "Map packs"),
        PackKind::MultiplayerMappack => tx("kfx_editor.multiplayer", "Multiplayer"),
    }
}

pub fn matches(filter: &str, pack: &PackEntry, l: &LevelEntry) -> bool {
    if filter.is_empty() {
        return true;
    }
    let f = filter.to_lowercase();
    pack.name.to_lowercase().contains(&f)
        || pack.file.to_lowercase().contains(&f)
        || l.name.to_lowercase().contains(&f)
        || format!("{:05}", l.number).contains(&f)
}

/// Facts shown under the thumbnail.
#[derive(Clone, Debug, Default)]
pub struct MapInfo {
    pub w: u16,
    pub h: u16,
    /// (owner, hearts)
    pub keepers: Vec<(u8, usize)>,
    pub creatures: usize,
    pub things: usize,
    pub action_points: usize,
    pub lights: usize,
    pub has_txt: bool,
    pub has_lua: bool,
    /// `FX` (`.tngfx`/...) or `legacy` (`.tng`/...).
    pub format: &'static str,
    pub texture: u8,
}

impl MapInfo {
    /// The things line of the browser's facts.
    fn things_text(&self) -> String {
        trf(
            "kfx_editor.things_creatures_creatures_action_points_action",
            "{things} ({creatures} creatures), \
            {action_points} action points, {lights} lights",
            &[
                ("things", &self.things),
                ("creatures", &self.creatures),
                ("action_points", &self.action_points),
                ("lights", &self.lights),
            ],
        )
    }
}

/// Load a map for the browser: its facts and a top-down image (one pixel per slab).
pub fn map_info(
    install: &Path,
    slb: &Path,
    gd: &GameData,
) -> Result<(MapInfo, egui::ColorImage), String> {
    let level = crate::document::map_number(slb).ok_or("not a map file")?;
    let folder = slb.parent().ok_or("no folder")?;
    let loc = crate::document::locate(install, folder, level)?;
    let m =
        kfx_map::mapdoc::MapDoc::load(folder, level, loc.dims, gd).map_err(|e| e.to_string())?;
    let mut keepers: Vec<(u8, usize)> = vec![];
    for t in &m.things.items {
        if things::thing_class(t) == Some(class::OBJECT)
            && t.subtype
                .is_some_and(|s| kfx_map::validate::is_heart_model(gd, s))
        {
            let o = t.owner.unwrap_or(5).clamp(0, 8) as u8;
            match keepers.iter_mut().find(|k| k.0 == o) {
                Some(k) => k.1 += 1,
                None => keepers.push((o, 1)),
            }
        }
    }
    keepers.sort();
    let info = MapInfo {
        w: m.dims.w,
        h: m.dims.h,
        keepers,
        creatures: m
            .things
            .items
            .iter()
            .filter(|t| things::thing_class(t) == Some(class::CREATURE))
            .count(),
        things: m.things.items.len(),
        action_points: m.action_points.items.len(),
        lights: m.lights.items.len(),
        has_txt: kfx_formats::level_bin::find_file(folder, level, "txt").is_some(),
        has_lua: kfx_formats::level_bin::find_file(folder, level, "lua").is_some(),
        format: if m.loaded_format == kfx_map::mapdoc::ThingFormat::Fx {
            "FX"
        } else {
            "legacy"
        },
        texture: m.texture_id,
    };
    Ok((info, crate::minimap::image(&m, gd)))
}

/// Write a `.lof` for a new level of a map pack (the pack lists its levels by `.lof` / `.lif`).
pub fn create_pack_level(
    dir: &Path,
    level: u32,
    multiplayer: bool,
    name: &str,
) -> Result<PathBuf, String> {
    let info = LevelInfo {
        name_text: Some(name.to_string()),
        kinds: vec![if multiplayer {
            LevelKind::Multi
        } else {
            LevelKind::Free
        }],
        mapsize: Some((85, 85)),
        players: multiplayer.then_some(2),
        ..Default::default()
    };
    let p = dir.join(format!("map{level:05}.lof"));
    Lof::new(&info)
        .save(&p)
        .map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(p)
}

/// What the browser asks the editor to do.
#[derive(Clone, Debug, PartialEq)]
pub enum BrowserAction {
    Open(PathBuf),
    Play(PathBuf),
    OpenFolder(PathBuf),
    /// A `.lof` was written for level `level` of `folder`: open the New map dialog for it.
    NewLevel {
        folder: PathBuf,
        level: u32,
    },
    /// Edit the campaign / map pack (its `.cfg`) in the campaign editor.
    EditPack(PathBuf),
}

#[derive(Default)]
pub struct MapBrowser {
    pub open: bool,
    pub filter: String,
    pub packs: Option<Vec<PackEntry>>,
    pub selected: Option<PathBuf>,
    /// (pack kind, file) of the selected pack.
    pub selected_pack: Option<(PackKind, String)>,
    thumb: Option<(PathBuf, Result<(MapInfo, egui::TextureHandle), String>)>,
    rng: u64,
    pub message: Option<String>,
}

impl MapBrowser {
    /// Select a map (headless and the random button).
    pub fn select(&mut self, p: Option<PathBuf>) {
        self.selected = p;
    }

    /// Pick a random level among the visible ones.
    pub fn random(&mut self) -> Option<PathBuf> {
        let packs = self.packs.as_ref()?;
        let all: Vec<(PathBuf, PackKind, String)> = packs
            .iter()
            .flat_map(|p| {
                p.levels
                    .iter()
                    .filter(|l| matches(&self.filter, p, l))
                    .filter_map(|l| l.slb.clone().map(|s| (s, p.kind, p.file.clone())))
            })
            .collect();
        if all.is_empty() {
            return None;
        }
        self.rng = self
            .rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407)
            ^ std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(1);
        let (p, k, f) = all[(self.rng >> 33) as usize % all.len()].clone();
        self.selected = Some(p.clone());
        self.selected_pack = Some((k, f));
        Some(p)
    }

    /// Show the window; returns what to do.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        install: Option<&Path>,
        gd: Option<&GameData>,
    ) -> Option<BrowserAction> {
        if !self.open {
            return None;
        }
        if self.packs.is_none() {
            self.packs = install.map(scan);
        }
        // thumbnail of the selection (made once per selection)
        if let (Some(sel), Some(install), Some(gd)) = (&self.selected, install, gd) {
            if self.thumb.as_ref().map(|t| &t.0) != Some(sel) {
                let r = map_info(install, sel, gd).map(|(i, img)| {
                    (
                        i,
                        ctx.load_texture("browser-thumb", img, egui::TextureOptions::NEAREST),
                    )
                });
                self.thumb = Some((sel.clone(), r));
            }
        }
        let mut open = self.open;
        let mut result = None;
        let no_slb_tip = trd(
            "kfx_editor.no_slb_file_in_the_levels_folder",
            "no .slb file in the levels folder",
        );
        egui::Window::new(trd("kfx_editor.map_browser", "Map browser"))
            .open(&mut open)
            .default_size([900.0, 620.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(trd("kfx_editor.filter_bdb7", "Filter:"));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text(trd(
                                "kfx_editor.name_number_or_pack",
                                "name, number or pack",
                            ))
                            .desired_width(240.0),
                    );
                    if ui.button(trd("kfx_editor.rescan", "Rescan")).clicked() {
                        self.packs = install.map(scan);
                        self.thumb = None;
                    }
                    if ui
                        .button(trd("kfx_editor.random_map", "🎲 Random map"))
                        .clicked()
                    {
                        self.random();
                    }
                });
                if let Some(m) = &self.message {
                    ui.colored_label(crate::theme::GOLD, m);
                }
                ui.separator();
                let Some(packs) = &self.packs else {
                    ui.label(trd(
                        "kfx_editor.no_install_folder_configured",
                        "No install folder configured.",
                    ));
                    return;
                };
                let h = ui.available_height() - 4.0;
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(470.0);
                        egui::ScrollArea::vertical()
                            .id_salt("browser-tree")
                            .max_height(h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for kind in [
                                    PackKind::Campaign,
                                    PackKind::Mappack,
                                    PackKind::MultiplayerMappack,
                                ] {
                                    let in_kind: Vec<&PackEntry> =
                                        packs.iter().filter(|p| p.kind == kind).collect();
                                    let n: usize = in_kind
                                        .iter()
                                        .map(|p| {
                                            p.levels
                                                .iter()
                                                .filter(|l| matches(&self.filter, p, l))
                                                .count()
                                        })
                                        .sum();
                                    egui::CollapsingHeader::new(
                                        RichText::new(format!("{} ({n})", kind_label(kind)))
                                            .color(crate::theme::GOLD)
                                            .strong(),
                                    )
                                    .id_salt(("kind", kind.folder()))
                                    .default_open(true)
                                    .show(ui, |ui| {
                                        for pack in in_kind {
                                            let shown: Vec<&LevelEntry> = pack
                                                .levels
                                                .iter()
                                                .filter(|l| matches(&self.filter, pack, l))
                                                .collect();
                                            if shown.is_empty()
                                                && pack.error.is_none()
                                                && !self.filter.is_empty()
                                            {
                                                continue;
                                            }
                                            let sel_pack = self.selected_pack.as_ref().is_some_and(
                                                |(k, f)| *k == pack.kind && *f == pack.file,
                                            );
                                            let title = format!(
                                                "{}  [{}]  {} levels",
                                                pack.name,
                                                pack.file,
                                                shown.len()
                                            );
                                            let hdr = egui::CollapsingHeader::new(if sel_pack {
                                                RichText::new(title).strong()
                                            } else {
                                                RichText::new(title)
                                            })
                                            .id_salt(("pack", pack.kind.folder(), &pack.file))
                                            .default_open(self.selected.as_ref().is_some_and(
                                                |sel| {
                                                    pack.levels
                                                        .iter()
                                                        .any(|l| l.slb.as_ref() == Some(sel))
                                                },
                                            ))
                                            .open((!self.filter.is_empty()).then_some(true))
                                            .show(ui, |ui| {
                                                if let Some(e) = &pack.error {
                                                    ui.colored_label(Color32::LIGHT_RED, e);
                                                }
                                                egui::Grid::new((
                                                    "grid",
                                                    pack.kind.folder(),
                                                    &pack.file,
                                                ))
                                                .striped(true)
                                                .num_columns(3)
                                                .show(ui, |ui| {
                                                    for l in shown {
                                                        let sel = l.slb.is_some()
                                                            && self.selected == l.slb;
                                                        let r = ui.add_enabled(
                                                            l.slb.is_some(),
                                                            egui::Button::selectable(
                                                                sel,
                                                                trf(
                                                                    "kfx_editor.map_number",
                                                                    "map{number}",
                                                                    &[(
                                                                        "number",
                                                                        &format!("{:05}", l.number),
                                                                    )],
                                                                ),
                                                            ),
                                                        );
                                                        let r = if l.slb.is_none() {
                                                            r.on_disabled_hover_text(
                                                                no_slb_tip.as_str(),
                                                            )
                                                        } else {
                                                            r
                                                        };
                                                        if r.clicked() {
                                                            self.selected = l.slb.clone();
                                                            self.selected_pack = Some((
                                                                pack.kind,
                                                                pack.file.clone(),
                                                            ));
                                                        }
                                                        if r.double_clicked() {
                                                            result = l
                                                                .slb
                                                                .clone()
                                                                .map(BrowserAction::Open);
                                                        }
                                                        ui.label(&l.name);
                                                        ui.label(
                                                            RichText::new(trf(
                                                                "kfx_editor.size_x_size2",
                                                                "{size}x{size2}",
                                                                &[
                                                                    ("size", &l.size.0),
                                                                    ("size2", &l.size.1),
                                                                ],
                                                            ))
                                                            .small(),
                                                        );
                                                        ui.end_row();
                                                    }
                                                });
                                            });
                                            if hdr.header_response.clicked() {
                                                self.selected_pack =
                                                    Some((pack.kind, pack.file.clone()));
                                            }
                                        }
                                    });
                                }
                            });
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.set_min_width(360.0);
                        match &self.thumb {
                            Some((p, Ok((info, tex)))) if Some(p) == self.selected.as_ref() => {
                                let sz = tex.size_vec2();
                                let scale = (340.0 / sz.x).min(300.0 / sz.y);
                                let (rect, _) =
                                    ui.allocate_exact_size(sz * scale, egui::Sense::hover());
                                ui.painter().image(
                                    tex.id(),
                                    rect,
                                    egui::Rect::from_min_max(
                                        egui::pos2(0.0, 0.0),
                                        egui::pos2(1.0, 1.0),
                                    ),
                                    Color32::WHITE,
                                );
                                ui.label(
                                    RichText::new(
                                        p.file_name().unwrap_or_default().to_string_lossy(),
                                    )
                                    .strong(),
                                );
                                egui::Grid::new("browser-info")
                                    .num_columns(2)
                                    .show(ui, |ui| {
                                        ui.label(trd("kfx_editor.size", "Size"));
                                        ui.label(trf(
                                            "kfx_editor.w_x_h_slabs",
                                            "{w} x {h} slabs",
                                            &[("w", &info.w), ("h", &info.h)],
                                        ));
                                        ui.end_row();
                                        ui.label(trd("kfx_editor.keepers", "Keepers"));
                                        ui.horizontal_wrapped(|ui| {
                                            if info.keepers.is_empty() {
                                                ui.label(trd(
                                                    "kfx_editor.none_no_dungeon_hearts",
                                                    "none (no dungeon hearts)",
                                                ));
                                            }
                                            for (o, n) in &info.keepers {
                                                let (name, c) = PLAYERS[*o as usize];
                                                let col = Color32::from_rgb(c[0], c[1], c[2]);
                                                let (r, _) = ui.allocate_exact_size(
                                                    egui::vec2(12.0, 12.0),
                                                    egui::Sense::hover(),
                                                );
                                                ui.painter().circle_filled(r.center(), 5.0, col);
                                                ui.label(
                                                    RichText::new(if *n > 1 {
                                                        trf(
                                                            "kfx_editor.name_x_n",
                                                            "{name} x{n}",
                                                            &[("name", &name), ("n", &n)],
                                                        )
                                                    } else {
                                                        name.to_string()
                                                    })
                                                    .color(col),
                                                );
                                            }
                                        });
                                        ui.end_row();
                                        ui.label(trd("kfx_editor.things", "Things"));
                                        ui.label(info.things_text());
                                        ui.end_row();
                                        ui.label(trd("kfx_editor.scripts", "Scripts"));
                                        ui.label(match (info.has_txt, info.has_lua) {
                                            (true, true) => trd(
                                                "kfx_editor.dk_script_and_lua",
                                                "DK script and Lua",
                                            ),
                                            (true, false) => {
                                                trd("kfx_editor.dk_script", "DK script")
                                            }
                                            (false, true) => trd("kfx_editor.lua", "Lua"),
                                            _ => trd("kfx_editor.none_557b", "none"),
                                        });
                                        ui.end_row();
                                        ui.label(trd("kfx_editor.format", "Format"));
                                        ui.label(trf(
                                            "kfx_editor.info_things_files_texture_set_texture",
                                            "{info} things files, texture set {texture}",
                                            &[("info", &info.format), ("texture", &info.texture)],
                                        ));
                                        ui.end_row();
                                    });
                            }
                            Some((p, Err(e))) if Some(p) == self.selected.as_ref() => {
                                ui.colored_label(Color32::LIGHT_RED, e);
                            }
                            _ => {
                                ui.label(
                                    RichText::new(trd(
                                        "kfx_editor.select_a_level_to_see_it_here",
                                        "Select a level to see it here",
                                    ))
                                    .color(crate::theme::TEXT_DIM),
                                );
                            }
                        }
                        ui.separator();
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add_enabled(
                                    self.selected.is_some(),
                                    egui::Button::new(trd("kfx_editor.edit_1270", "✏ Edit")),
                                )
                                .clicked()
                            {
                                result = self.selected.clone().map(BrowserAction::Open);
                            }
                            if ui
                                .add_enabled(
                                    self.selected.is_some(),
                                    egui::Button::new(trd("kfx_editor.play", "▶ Play")),
                                )
                                .clicked()
                            {
                                result = self.selected.clone().map(BrowserAction::Play);
                            }
                            let folder = self
                                .selected
                                .as_ref()
                                .and_then(|p| p.parent().map(Path::to_path_buf));
                            if ui
                                .add_enabled(
                                    folder.is_some(),
                                    egui::Button::new(trd(
                                        "kfx_editor.open_folder",
                                        "📁 Open folder",
                                    )),
                                )
                                .clicked()
                            {
                                result = folder.map(BrowserAction::OpenFolder);
                            }
                        });
                        let pack = self
                            .selected_pack
                            .as_ref()
                            .and_then(|(k, f)| packs.iter().find(|p| p.kind == *k && p.file == *f));
                        if let Some(pack) = pack {
                            ui.separator();
                            ui.label(
                                RichText::new(trf(
                                    "kfx_editor.pack_name",
                                    "Pack: {name}",
                                    &[("name", &pack.name)],
                                ))
                                .strong(),
                            );
                            let cfg = install
                                .and_then(|i| kfx_gfx::install::resolve(i, pack.kind.folder()))
                                .map(|d| d.join(&pack.file))
                                .filter(|p| p.is_file());
                            let what = if pack.kind == PackKind::Campaign {
                                trd("kfx_editor.edit_this_campaign", "✏ Edit this campaign")
                            } else {
                                trd("kfx_editor.edit_this_map_pack", "✏ Edit this map pack")
                            };
                            if ui
                                .add_enabled(cfg.is_some(), egui::Button::new(what))
                                .on_hover_text(trd(
                                    "kfx_editor.levels_land_view_texts_and_checks_in_the_campaig",
                                    "Levels, land view, texts and checks in the campaign \
                                    editor",
                                ))
                                .clicked()
                            {
                                result = cfg.map(BrowserAction::EditPack);
                            }
                            let can = pack.kind != PackKind::Campaign && pack.levels_dir.is_some();
                            let r = ui.add_enabled(
                                can,
                                egui::Button::new(trd(
                                    "kfx_editor.new_level_in_this_pack",
                                    "➕ New level in this pack",
                                )),
                            );
                            let r = r.on_disabled_hover_text(trd(
                                "kfx_editor.campaign_levels_are_listed_in_the_campaign_file",
                                "Campaign levels are listed in the campaign file; add \
                                new levels to map packs (levels/ or multiplayer/)",
                            ));
                            if r.clicked() {
                                if let Some(dir) = &pack.levels_dir {
                                    let n = crate::playtest::next_free_level(dir, 1);
                                    match create_pack_level(
                                        dir,
                                        n,
                                        pack.kind == PackKind::MultiplayerMappack,
                                        &format!("New level {n}"),
                                    ) {
                                        Ok(_) => {
                                            result = Some(BrowserAction::NewLevel {
                                                folder: dir.clone(),
                                                level: n,
                                            });
                                        }
                                        Err(e) => self.message = Some(e),
                                    }
                                }
                            }
                        }
                    });
                });
            });
        if matches!(result, Some(BrowserAction::NewLevel { .. })) {
            self.packs = install.map(scan);
        }
        self.open = open
            && !matches!(
                result,
                Some(BrowserAction::Open(_)) | Some(BrowserAction::NewLevel { .. })
            );
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_real_install() {
        let Some(root) = kfx_formats::testutil::install_dir() else {
            return;
        };
        let packs = scan(&root);
        let orig = packs
            .iter()
            .find(|p| p.file.eq_ignore_ascii_case("keeporig.cfg"))
            .expect("keeporig");
        assert!(orig.levels.iter().any(|l| l.number == 1 && l.slb.is_some()));
        println!(
            "keeporig level 1: {:?}",
            orig.levels.iter().find(|l| l.number == 1).map(|l| &l.name)
        );
        let e = LevelEntry {
            number: 4008,
            name: "Big One".into(),
            size: (121, 121),
            slb: None,
        };
        assert!(matches("4008", orig, &e) && matches("big", orig, &e) && !matches("zzz", orig, &e));
        // facts of a shipped map
        let layers = crate::document::base_layers(&root);
        let gd = GameData::load(&layers).unwrap();
        let slb = orig
            .levels
            .iter()
            .find(|l| l.number == 1)
            .and_then(|l| l.slb.clone())
            .unwrap();
        let (info, img) = map_info(&root, &slb, &gd).unwrap();
        assert_eq!((info.w, info.h), (85, 85));
        assert!(info.keepers.iter().any(|k| k.0 == 0), "{:?}", info.keepers);
        assert!(info.has_txt);
        assert_eq!(img.size, [85, 85]);
    }

    #[test]
    fn new_pack_level_lof() {
        let d = kfx_formats::testutil::scratch_dir(&format!(
            "kfx-editor/browser_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = create_pack_level(&d, 3, false, "New level 3").unwrap();
        let info = Lof::load(&p).unwrap().info();
        assert_eq!(info.name_text.as_deref(), Some("New level 3"));
        assert_eq!(info.mapsize, Some((85, 85)));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
