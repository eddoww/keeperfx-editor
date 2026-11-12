//! New map (with generated terrain and keepers), image to map, export image, preferences,
//! keyboard shortcuts, help.
use super::*;
use crate::imageio::{self, ColorRule};
use crate::newmap::{self, NewMapSpec, Plan, ScriptChoice};
use crate::settings::Settings as S;
use crate::shortcuts::{self, Keymap, Shortcut, ACTIONS};
use kfx_map::transform::SymmetryMode;
use kfx_prefab::i18n::{trd, trf, tx, Tx};

/// The New map dialog.
pub struct NewMapDialog {
    pub spec: NewMapSpec,
    pub level: u32,
    pub folder: String,
    pub error: Option<String>,
    preview: Option<egui::TextureHandle>,
    planned: Option<(NewMapSpec, Plan)>,
    /// The generator's preview (generate mode).
    pub(super) gen: super::generator::GenPreview,
}

impl NewMapDialog {
    pub fn new(folder: String, level: u32) -> NewMapDialog {
        NewMapDialog {
            spec: NewMapSpec::default(),
            level,
            folder,
            error: None,
            preview: None,
            planned: None,
            gen: Default::default(),
        }
    }
}

/// The Image to map dialog.
pub struct ImageDialog {
    pub path: String,
    pub img: Option<kfx_gfx::Rgba>,
    pub rules: Vec<ColorRule>,
    pub error: Option<String>,
    preview: Option<egui::TextureHandle>,
    dirty: bool,
}

/// The Export image dialog.
pub struct ExportDialog {
    pub path: String,
    pub iso: bool,
    pub px_per_slab: u32,
    pub things: bool,
    pub grid: bool,
    pub ownership: bool,
    pub result: Option<String>,
}

/// Preferences tabs.
const PREF_TABS: [(&str, Tx); 9] = [
    ("Files", tx("kfx_editor.files", "Files")),
    ("Play", tx("kfx_editor.play_d3fa", "Play")),
    ("Placement", tx("kfx_editor.placement", "Placement")),
    ("Interface", tx("kfx_editor.interface", "Interface")),
    ("Camera", tx("kfx_editor.camera", "Camera")),
    ("Graphics", tx("kfx_editor.graphics", "Graphics")),
    ("Decoration", tx("kfx_editor.decoration", "Decoration")),
    ("Shortcuts", tx("kfx_editor.shortcuts", "Shortcuts")),
    ("Updates", tx("kfx_editor.updates", "Updates")),
];

pub const WIKI: &[(Tx, &str)] = &[
    (
        tx("kfx_editor.keeperfx_wiki", "KeeperFX wiki"),
        "https://github.com/dkfans/keeperfx/wiki",
    ),
    (
        tx("kfx_editor.level_script_commands", "Level script commands"),
        "https://github.com/dkfans/keeperfx/wiki/Level-Script-commands",
    ),
    (
        tx("kfx_editor.lua_level_scripts", "Lua level scripts"),
        "https://github.com/dkfans/keeperfx/wiki/Lua-Scripting",
    ),
    (
        tx(
            "kfx_editor.campaign_and_map_pack_files",
            "Campaign and map pack files",
        ),
        "https://github.com/dkfans/keeperfx/wiki/Campaign-files",
    ),
    (
        tx("kfx_editor.making_maps", "Making maps"),
        "https://github.com/dkfans/keeperfx/wiki/Making-Maps",
    ),
    (
        tx("kfx_editor.command_line_options", "Command line options"),
        "https://github.com/dkfans/keeperfx/wiki/Command-Line-Options",
    ),
];

fn sym_name(m: Option<SymmetryMode>) -> Tx {
    match m {
        None => tx("kfx_editor.none_c95b", "None"),
        Some(SymmetryMode::NorthSouth) => tx("kfx_editor.north_south", "North-south"),
        Some(SymmetryMode::EastWest) => tx("kfx_editor.east_west", "East-west"),
        Some(SymmetryMode::FourWay) => tx("kfx_editor.4_way", "4-way"),
        Some(SymmetryMode::Rot2) => tx("kfx_editor.rotational_2", "Rotational 2"),
        Some(SymmetryMode::Rot4) => tx("kfx_editor.rotational_4", "Rotational 4"),
    }
}

impl EditorApp {
    // --------------------------------------------------------------------------- new map

    pub(super) fn open_new_dialog(&mut self) {
        // the open map's folder unless it belongs to a campaign (shipped levels): then the
        // "Personal levels" pack
        let personal = self
            .install()
            .and_then(|i| kfx_gfx::install::resolve(i, "levels/personal"));
        let own = self.doc.as_ref().map(|d| d.doc.folder.clone()).filter(|f| {
            let install = self.install().map(Path::to_path_buf).unwrap_or_default();
            !matches!(
                crate::playtest::play_target(&install, f, 1),
                crate::playtest::PlayTarget::Pack {
                    kind: kfx_formats::campaign::PackKind::Campaign,
                    ..
                }
            )
        });
        let folder = own.or(personal).unwrap_or_default();
        let level = crate::playtest::next_free_level(&folder, 1);
        self.new_dialog = Some(NewMapDialog::new(folder.display().to_string(), level));
    }

    pub(super) fn base_gd(&mut self) -> Option<Arc<GameData>> {
        let install = self.install()?.to_path_buf();
        self.gamedata(&document::base_layers(&install)).ok()
    }

    /// Create a new map from a spec (the New map dialog's Create; also scripted).
    pub fn new_map_spec(
        &mut self,
        folder: &Path,
        level: u32,
        spec: &NewMapSpec,
    ) -> Result<(), String> {
        let Some(install) = self.install().map(Path::to_path_buf) else {
            return Err(trd("kfx_editor.no_install_folder", "no install folder"));
        };
        if !(1..=170).contains(&spec.w) || !(1..=170).contains(&spec.h) {
            return Err(trd(
                "kfx_editor.map_size_must_be_1_170_slabs_each_way",
                "map size must be 1..170 slabs each way",
            ));
        }
        let c = document::level_config(&install, folder, level, &self.active_mods());
        let gd = self.gamedata_stack(&c.stack)?;
        if spec.generate {
            return self.new_generated_map(folder, level, spec, gd, c);
        }
        let p = newmap::plan(spec, &gd);
        let mut doc = newmap::build(
            &p,
            spec,
            folder,
            level,
            gd.clone(),
            c.stack,
            self.settings.decoration,
        );
        doc.layers = c.layers;
        doc.pack = c.pack;
        doc.pack_cfg = c.pack_cfg;
        if spec.kind.is_some() || !spec.name.trim().is_empty() {
            let mut info = kfx_formats::levelinfo::LevelInfo {
                kinds: spec.kind.into_iter().collect(),
                ..Default::default()
            };
            if !spec.name.trim().is_empty() {
                info.name_text = Some(spec.name.trim().to_string());
            }
            if spec.kind == Some(kfx_formats::levelinfo::LevelKind::Multi) {
                info.players = Some(p.bases.len().max(1) as i32);
            }
            doc.pending_info = Some(info);
        }
        self.scripts = None;
        self.set_document(doc, false);
        self.apply_decor_prefs();
        match spec.script {
            ScriptChoice::Dk => {
                let name = kfx_script_ui::workspace::dk_file_name(level);
                self.set_script_text_dirty(&name, newmap::dk_script(level, &p, &gd).as_bytes());
            }
            ScriptChoice::Lua => {
                let name = kfx_script_ui::workspace::lua_file_name(level);
                self.set_script_text_dirty(&name, newmap::lua_script(level, &p, &gd).as_bytes());
            }
            ScriptChoice::None => {}
        }
        let (w, h) = (p.w, p.h);
        self.status = trf(
            "kfx_editor.new_map_level_w_x_h_bases_keepers_in_folder_not",
            "New map{level} ({w}x{h}, {bases} keepers) in {folder}: not saved yet{warnings}",
            &[
                ("level", &format!("{:05}", level)),
                ("w", &w),
                ("h", &h),
                ("bases", &p.bases.len()),
                ("folder", &folder.display()),
                (
                    "warnings",
                    &(if p.warnings.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", p.warnings.join("; "))
                    }),
                ),
            ],
        );
        Ok(())
    }

    /// Create a new map with the constrained generator (`spec.generate`).
    fn new_generated_map(
        &mut self,
        folder: &Path,
        level: u32,
        spec: &NewMapSpec,
        gd: Arc<GameData>,
        c: document::LevelConfig,
    ) -> Result<(), String> {
        if !(20..=170).contains(&spec.w) || !(20..=170).contains(&spec.h) {
            return Err(trd(
                "kfx_editor.generated_maps_are_20_170_slabs_each_way",
                "generated maps are 20..170 slabs each way",
            ));
        }
        let g = self.generated_for(spec, folder, level, &gd)?;
        let mut doc = newmap::build_generated(
            &g,
            spec,
            folder,
            level,
            gd.clone(),
            c.stack,
            self.settings.decoration,
        );
        doc.layers = c.layers;
        doc.pack = c.pack;
        doc.pack_cfg = c.pack_cfg;
        let keepers = g.plan.bases.len();
        if spec.kind.is_some() || !spec.name.trim().is_empty() {
            let mut info = kfx_formats::levelinfo::LevelInfo {
                kinds: spec.kind.into_iter().collect(),
                ..Default::default()
            };
            if !spec.name.trim().is_empty() {
                info.name_text = Some(spec.name.trim().to_string());
            }
            if spec.kind == Some(kfx_formats::levelinfo::LevelKind::Multi) {
                info.players = Some(keepers.max(1) as i32);
            }
            doc.pending_info = Some(info);
        }
        self.scripts = None;
        self.set_document(doc, false);
        self.apply_decor_prefs();
        match spec.script {
            ScriptChoice::Dk => {
                let name = kfx_script_ui::workspace::dk_file_name(level);
                self.set_script_text_dirty(&name, newmap::dk_starter(level, &g.script).as_bytes());
            }
            ScriptChoice::Lua => {
                let name = kfx_script_ui::workspace::lua_file_name(level);
                self.set_script_text_dirty(&name, newmap::lua_starter(level, &g.script).as_bytes());
            }
            ScriptChoice::None => {}
        }
        let r = &g.report;
        let imb = r
            .imbalance
            .map(|i| {
                trf(
                    "kfx_editor.imbalance_i",
                    ", imbalance {i}",
                    &[("i", &format!("{:.2}", i))],
                )
            })
            .unwrap_or_default();
        self.status = trf(
            "kfx_editor.new_generated_map_level_w_x_h_keepers_keepers_st",
            "New generated map{level} ({w}x{h}, {keepers} keepers, {style} style, \
            seed {seed}{imb}) in {folder}: not saved yet",
            &[
                ("level", &format!("{:05}", level)),
                ("w", &g.doc.dims.w),
                ("h", &g.doc.dims.h),
                ("keepers", &keepers),
                ("style", &newmap::style_label(spec.gen.style)),
                ("seed", &spec.seed),
                ("imb", &imb),
                ("folder", &folder.display()),
            ],
        );
        Ok(())
    }

    /// Create a new map (the old scripted form).
    #[allow(clippy::too_many_arguments)]
    pub fn new_map(
        &mut self,
        folder: &Path,
        level: u32,
        w: u32,
        h: u32,
        border_rock: bool,
        fill_earth: bool,
        texture: u8,
    ) -> Result<(), String> {
        let spec = NewMapSpec {
            w,
            h,
            border_rock,
            fill: if fill_earth {
                "EARTH".into()
            } else {
                "ROCK".into()
            },
            texture,
            ..Default::default()
        };
        self.new_map_spec(folder, level, &spec)
    }

    pub(super) fn new_map_ui(&mut self, ctx: &egui::Context) {
        if self.new_dialog.is_none() {
            return;
        }
        let gd = self.base_gd();
        let names = panels::texture_pack_names();
        let dlg = self.new_dialog.as_mut().unwrap();
        // live preview (generate mode: the generator's map, made on a thread)
        if let (Some(gd), true) = (&gd, dlg.spec.generate) {
            let p = newmap::gen_params(
                &dlg.spec,
                Path::new(dlg.folder.trim()),
                dlg.level,
                self.settings.decoration,
            );
            dlg.gen.tick(ctx, gd, &p, false);
        } else if let Some(gd) = &gd {
            if dlg.planned.as_ref().map(|(s, _)| s) != Some(&dlg.spec) {
                let p = newmap::plan(&dlg.spec, gd);
                let img = newmap::preview(&p, gd);
                match &mut dlg.preview {
                    Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                    None => {
                        dlg.preview = Some(ctx.load_texture(
                            "newmap-preview",
                            img,
                            egui::TextureOptions::NEAREST,
                        ))
                    }
                }
                dlg.planned = Some((dlg.spec.clone(), p));
            }
        }
        let mut ok = false;
        let mut cancel = false;
        let kind_tip = trd(
            "kfx_editor.written_to_the_level_s_lof_with_the_first_save_s",
            "Written to the level's .lof with the first save \
            (SINGLE campaign level, MULTI multiplayer, FREE free play…)",
        );
        egui::Modal::new(egui::Id::unique("new-map")).show(ctx, |ui| {
            ui.set_width(900.0);
            ui.heading(RichText::new(trd("kfx_editor.new_map", "New map")).color(GOLD));
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(520.0);
                    let s = &mut dlg.spec;
                    egui::Grid::new("new-map-grid")
                        .num_columns(2)
                        .spacing([10.0, 5.0])
                        .show(ui, |ui| {
                            ui.label(trd("kfx_editor.size_slabs", "Size (slabs)"));
                            ui.horizontal(|ui| {
                                ui.add(egui::DragValue::new(&mut s.w).range(1..=170));
                                ui.label("x");
                                ui.add(egui::DragValue::new(&mut s.h).range(1..=170));
                                for n in [42u32, 85, 128, 170] {
                                    if ui.small_button(format!("{n}")).clicked() {
                                        s.w = n;
                                        s.h = n;
                                    }
                                }
                            });
                            ui.end_row();
                            ui.label(trd("kfx_editor.folder", "Folder"));
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut dlg.folder)
                                        .desired_width(330.0),
                                );
                                if ui.button(trd("kfx_editor.browse", "Browse…")).clicked() {
                                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                                        dlg.folder = p.display().to_string();
                                    }
                                }
                            });
                            ui.end_row();
                            ui.label(trd("kfx_editor.level_number", "Level number"));
                            ui.horizontal(|ui| {
                                ui.add(egui::DragValue::new(&mut dlg.level).range(1..=32767));
                                if ui
                                    .small_button(trd("kfx_editor.next_free", "Next free"))
                                    .clicked()
                                {
                                    dlg.level = crate::playtest::next_free_level(
                                        Path::new(dlg.folder.trim()),
                                        1,
                                    );
                                }
                            });
                            ui.end_row();
                            ui.label(trd("kfx_editor.texture_set", "Texture set"));
                            egui::ComboBox::from_id_salt("nm-tex")
                                .selected_text(format!(
                                    "{}: {}",
                                    s.texture,
                                    panels::texture_set_name(&names, s.texture as u32)
                                ))
                                .show_ui(ui, |ui| {
                                    for t in 0..16u8 {
                                        ui.selectable_value(
                                            &mut s.texture,
                                            t,
                                            format!(
                                                "{t}: {}",
                                                panels::texture_set_name(&names, t as u32)
                                            ),
                                        );
                                    }
                                });
                            ui.end_row();
                            ui.label(trd("kfx_editor.terrain", "Terrain"));
                            ui.horizontal(|ui| {
                                ui.radio_value(
                                    &mut s.generate,
                                    false,
                                    trd("kfx_editor.fill", "Fill"),
                                )
                                .on_hover_text(trd(
                                    "kfx_editor.one_slab_kind_everywhere_with_keepers_placed_on",
                                    "One slab kind everywhere, with keepers placed on a \
                                    ring.",
                                ));
                                ui.radio_value(
                                    &mut s.generate,
                                    true,
                                    trd("kfx_editor.generate", "Generate"),
                                )
                                .on_hover_text(trd(
                                    "kfx_editor.a_constrained_random_map_every_keeper_gets_a_rea",
                                    "A constrained random map: every keeper gets a \
                                    reachable portal and gold, routes to the others, and a fair \
                                    share.",
                                ));
                            });
                            ui.end_row();
                            if s.generate {
                                ui.label(trd("kfx_editor.seed", "Seed"));
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::DragValue::new(&mut s.seed)
                                            .range(1..=u32::MAX as u64),
                                    );
                                    if ui
                                        .button(trd("kfx_editor.regenerate_792e", "🎲 Regenerate"))
                                        .on_hover_text(trd(
                                            "kfx_editor.generate_again_with_a_new_seed",
                                            "Generate again with a new seed.",
                                        ))
                                        .clicked()
                                    {
                                        s.seed = super::generator::next_seed(s.seed);
                                    }
                                });
                                ui.end_row();
                                super::generator::gen_options_ui(ui, s);
                            } else {
                                ui.label(trd("kfx_editor.border", "Border"));
                                ui.horizontal(|ui| {
                                    ui.radio_value(
                                        &mut s.border_rock,
                                        true,
                                        trd("kfx_editor.impenetrable_rock", "Impenetrable rock"),
                                    );
                                    ui.radio_value(
                                        &mut s.border_rock,
                                        false,
                                        trd("kfx_editor.none_c95b", "None"),
                                    );
                                });
                                ui.end_row();
                                ui.label(trd("kfx_editor.fill", "Fill"));
                                egui::ComboBox::from_id_salt("nm-fill")
                                    .selected_text(s.fill.clone())
                                    .show_ui(ui, |ui| {
                                        for f in ["EARTH", "ROCK", "PATH", "GOLD", "WATER", "LAVA"]
                                        {
                                            ui.selectable_value(&mut s.fill, f.to_string(), f);
                                        }
                                    });
                                ui.end_row();
                                ui.label(trd("kfx_editor.symmetry", "Symmetry"));
                                egui::ComboBox::from_id_salt("nm-sym")
                                    .selected_text(sym_name(s.symmetry))
                                    .show_ui(ui, |ui| {
                                        for m in [
                                            None,
                                            Some(SymmetryMode::NorthSouth),
                                            Some(SymmetryMode::EastWest),
                                            Some(SymmetryMode::FourWay),
                                            Some(SymmetryMode::Rot2),
                                            Some(SymmetryMode::Rot4),
                                        ] {
                                            ui.selectable_value(&mut s.symmetry, m, sym_name(m));
                                        }
                                    });
                                ui.end_row();
                            }
                            ui.label(trd("kfx_editor.keepers", "Keepers"));
                            ui.horizontal(|ui| {
                                ui.add(egui::Slider::new(&mut s.players, 0..=7));
                                ui.label(trd("kfx_editor.imps_each", "imps each"));
                                ui.add(egui::DragValue::new(&mut s.imps).range(0..=10));
                            });
                            ui.end_row();
                            if !s.generate {
                                ui.label("");
                                ui.horizontal(|ui| {
                                    ui.checkbox(
                                        &mut s.portals,
                                        trd(
                                            "kfx_editor.portal_near_each_heart",
                                            "Portal near each heart",
                                        ),
                                    );
                                    ui.checkbox(
                                        &mut s.gold_near,
                                        trd(
                                            "kfx_editor.gold_near_each_heart",
                                            "Gold near each heart",
                                        ),
                                    );
                                });
                                ui.end_row();
                                ui.label(trd("kfx_editor.hero_gates", "Hero gates"));
                                ui.add(egui::Slider::new(&mut s.hero_gates, 0..=8));
                                ui.end_row();
                            }
                            ui.label(trd("kfx_editor.kind", "Kind"));
                            ui.horizontal(|ui| {
                                use kfx_formats::levelinfo::LevelKind as K;
                                egui::ComboBox::from_id_salt("nm-kind")
                                    .selected_text(
                                        s.kind.map(|k| k.name().to_string()).unwrap_or_else(|| {
                                            trd("kfx_editor.not_set", "(not set)")
                                        }),
                                    )
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut s.kind,
                                            None,
                                            trd("kfx_editor.not_set", "(not set)"),
                                        );
                                        for k in K::ALL {
                                            ui.selectable_value(&mut s.kind, Some(k), k.name());
                                        }
                                    })
                                    .response
                                    .on_hover_text(kind_tip.as_str());
                                ui.label(trd("kfx_editor.name", "Name"));
                                ui.add(
                                    egui::TextEdit::singleline(&mut s.name)
                                        .desired_width(180.0)
                                        .hint_text(trd(
                                            "kfx_editor.level_name_lof",
                                            "level name (.lof)",
                                        )),
                                );
                            });
                            ui.end_row();
                            ui.label(trd("kfx_editor.script", "Script"));
                            ui.horizontal(|ui| {
                                ui.radio_value(
                                    &mut s.script,
                                    ScriptChoice::None,
                                    trd("kfx_editor.none_c95b", "None"),
                                );
                                ui.radio_value(
                                    &mut s.script,
                                    ScriptChoice::Dk,
                                    trd("kfx_editor.dk_script", "DK script"),
                                );
                                ui.radio_value(
                                    &mut s.script,
                                    ScriptChoice::Lua,
                                    trd("kfx_editor.lua", "Lua"),
                                );
                            });
                            ui.end_row();
                        });
                });
                // (a separator here grew the modal by its own height every frame, so its
                // buttons moved under the pointer: found by the UI tests)
                ui.add_space(16.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(trd("kfx_editor.preview", "Preview")).color(GOLD));
                    if dlg.spec.generate {
                        super::generator::gen_preview_ui(ui, &dlg.gen);
                    } else if let Some(t) = &dlg.preview {
                        let sz = t.size_vec2();
                        let scale = (340.0 / sz.x).min(340.0 / sz.y);
                        let (rect, _) = ui.allocate_exact_size(sz * scale, egui::Sense::hover());
                        ui.painter().image(
                            t.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                        if let Some((_, p)) = &dlg.planned {
                            let px = |x: i32, y: i32| {
                                rect.min
                                    + EVec2::new((x as f32 + 0.5) * scale, (y as f32 + 0.5) * scale)
                            };
                            for b in &p.bases {
                                let c = PLAYERS[b.owner as usize].1;
                                ui.painter().circle_stroke(
                                    px(b.heart.0, b.heart.1),
                                    2.5 * scale + 3.0,
                                    egui::Stroke::new(2.0, Color32::from_rgb(c[0], c[1], c[2])),
                                );
                                if let Some(po) = b.portal {
                                    ui.painter().rect_stroke(
                                        egui::Rect::from_center_size(
                                            px(po.0, po.1),
                                            EVec2::splat(3.0 * scale + 2.0),
                                        ),
                                        0.0,
                                        egui::Stroke::new(1.5, Color32::WHITE),
                                        egui::StrokeKind::Outside,
                                    );
                                }
                            }
                            for (n, (x, y)) in &p.gates {
                                ui.painter().text(
                                    px(*x, *y),
                                    egui::Align2::CENTER_CENTER,
                                    format!("G{n}"),
                                    egui::FontId::proportional(10.0),
                                    Color32::WHITE,
                                );
                            }
                            ui.label(
                                RichText::new(trf(
                                    "kfx_editor.w_x_h_bases_keepers_gates_hero_gates_hearts_ring",
                                    "{w}x{h} · {bases} keepers · {gates} hero gates · \
                                    hearts ringed in the keeper colour, portals boxed",
                                    &[
                                        ("w", &p.w),
                                        ("h", &p.h),
                                        ("bases", &p.bases.len()),
                                        ("gates", &p.gates.len()),
                                    ],
                                ))
                                .small()
                                .color(TEXT_DIM),
                            );
                            for w in &p.warnings {
                                ui.colored_label(Color32::from_rgb(240, 180, 80), w);
                            }
                        }
                    }
                });
            });
            let exists = !dlg.folder.trim().is_empty()
                && !document::level_files(Path::new(dlg.folder.trim()), dlg.level).is_empty();
            if exists {
                ui.colored_label(
                    Color32::from_rgb(240, 180, 80),
                    trf(
                        "kfx_editor.map_level_already_exists_in_that_folder_saving_w",
                        "map{level} already exists in that folder; saving will overwrite \
                        it (a backup is made first).",
                        &[("level", &format!("{:05}", dlg.level))],
                    ),
                );
            }
            if let Some(e) = &dlg.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !dlg.folder.trim().is_empty(),
                        egui::Button::new(trd("kfx_editor.create", "Create")),
                    )
                    .clicked()
                {
                    ok = true;
                }
                if ui.button(trd("kfx_editor.cancel", "Cancel")).clicked() {
                    cancel = true;
                }
            });
        });
        if cancel {
            self.new_dialog = None;
        } else if ok {
            let d = self.new_dialog.take().unwrap();
            let folder = PathBuf::from(d.folder.trim());
            if let Err(e) = self.new_map_spec(&folder, d.level, &d.spec) {
                self.new_dialog = Some(NewMapDialog {
                    error: Some(e),
                    ..d
                });
            }
        }
    }

    /// The New map dialog's spec (headless actions).
    pub fn new_dialog_spec(&mut self) -> Option<&mut NewMapSpec> {
        self.new_dialog.as_mut().map(|d| &mut d.spec)
    }

    // --------------------------------------------------------------------------- image to map

    pub fn open_image_dialog(&mut self, path: Option<&Path>) {
        let rules = self.gd().map(imageio::default_rules).unwrap_or_default();
        let mut d = ImageDialog {
            path: path.map(|p| p.display().to_string()).unwrap_or_default(),
            img: None,
            rules,
            error: None,
            preview: None,
            dirty: true,
        };
        if let Some(p) = path {
            match std::fs::read(p)
                .map_err(|e| e.to_string())
                .and_then(|b| kfx_gfx::Rgba::from_png(&b).map_err(|e| e.to_string()))
            {
                Ok(i) => d.img = Some(i),
                Err(e) => d.error = Some(format!("{}: {e}", p.display())),
            }
        }
        self.image_dialog = Some(d);
    }

    /// Apply the image dialog's mapping to the map as one undo step.
    pub fn apply_image(&mut self) -> Result<usize, String> {
        let dlg = self.image_dialog.as_ref().ok_or("no image dialog")?;
        let img = dlg.img.as_ref().ok_or("load an image first")?;
        let d = self.doc.as_ref().ok_or("no map")?;
        let dims = d.dims();
        let cells = imageio::map_image(img, dims.w as usize, dims.h as usize, &dlg.rules);
        let mut edits = vec![];
        for (i, c) in cells.iter().enumerate() {
            let Some((k, o)) = *c else { continue };
            let (x, y) = ((i % dims.w as usize) as i32, (i / dims.w as usize) as i32);
            if d.state_at(x, y)
                .is_some_and(|s| s.kind == k && s.owner == o)
                || assets::is_door(&d.gd, k as usize)
            {
                continue;
            }
            edits.push((x, y, Edit::Place { kind: k, owner: o }));
        }
        let n = edits.len();
        self.one_step(&trd("kfx_editor.image_to_map", "Image to map"), &edits);
        self.status = trf(
            "kfx_editor.image_to_map_n_slabs_changed_one_undo_step",
            "Image to map: {n} slabs changed (one undo step)",
            &[("n", &n)],
        );
        Ok(n)
    }

    pub(super) fn image_ui(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.image_dialog else {
            return;
        };
        let Some(doc) = &self.doc else {
            self.image_dialog = None;
            return;
        };
        let gd = doc.gd.clone();
        let dims = doc.dims();
        if dlg.dirty {
            dlg.dirty = false;
            if let Some(img) = &dlg.img {
                let cells = imageio::map_image(img, dims.w as usize, dims.h as usize, &dlg.rules);
                let keep = |i: usize| (doc.doc.kinds[i], doc.doc.owners[i]);
                let pi = imageio::preview(&cells, dims.w as usize, dims.h as usize, &gd, keep);
                match &mut dlg.preview {
                    Some(t) => t.set(pi, egui::TextureOptions::NEAREST),
                    None => {
                        dlg.preview =
                            Some(ctx.load_texture("img-preview", pi, egui::TextureOptions::NEAREST))
                    }
                }
            }
        }
        let mut apply = false;
        let mut close = false;
        let mut load = None;
        let slabs: Vec<(u16, String)> = (0..gd.slab_count())
            .filter(|&k| !assets::is_door(&gd, k))
            .map(|k| (k as u16, gd.slab_name(k).unwrap_or("?").to_string()))
            .collect();
        egui::Window::new(trd("kfx_editor.image_to_map", "Image to map"))
            .collapsible(false)
            .default_size([900.0, 560.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(trd("kfx_editor.png", "PNG"));
                    ui.add(egui::TextEdit::singleline(&mut dlg.path).desired_width(420.0));
                    if ui.button(trd("kfx_editor.browse", "Browse…")).clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter(trd("kfx_editor.png_image", "PNG image"), &["png", "PNG"])
                            .pick_file()
                        {
                            dlg.path = p.display().to_string();
                            load = Some(p);
                        }
                    }
                    if ui.button(trd("kfx_editor.load", "Load")).clicked() {
                        load = Some(PathBuf::from(dlg.path.trim()));
                    }
                });
                if let Some(e) = &dlg.error {
                    ui.colored_label(Color32::LIGHT_RED, e);
                }
                ui.label(
                    RichText::new(trf(
                        "kfx_editor.the_image_is_scaled_to_the_map_w_x_h_slabs_one_s",
                        "The image is scaled to the map ({w}x{h} slabs, one sample per \
                        slab); every sample takes the slab and owner of the nearest colour below. \
                        Transparent pixels keep the map.",
                        &[("w", &dims.w), ("h", &dims.h)],
                    ))
                    .small()
                    .color(TEXT_DIM),
                );
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(470.0);
                        let mut remove = None;
                        egui::ScrollArea::vertical()
                            .max_height(400.0)
                            .show(ui, |ui| {
                                egui::Grid::new("img-rules")
                                    .num_columns(4)
                                    .striped(true)
                                    .show(ui, |ui| {
                                        for (i, r) in dlg.rules.iter_mut().enumerate() {
                                            let mut c = Color32::from_rgb(
                                                r.color[0], r.color[1], r.color[2],
                                            );
                                            if ui.color_edit_button_srgba(&mut c).changed() {
                                                r.color = [c.r(), c.g(), c.b()];
                                                dlg.dirty = true;
                                            }
                                            egui::ComboBox::from_id_salt(("img-slab", i))
                                                .width(170.0)
                                                .selected_text(
                                                    gd.slab_name(r.kind as usize).unwrap_or("?"),
                                                )
                                                .show_ui(ui, |ui| {
                                                    for (k, n) in &slabs {
                                                        if ui
                                                            .selectable_value(&mut r.kind, *k, n)
                                                            .changed()
                                                        {
                                                            dlg.dirty = true;
                                                        }
                                                    }
                                                });
                                            egui::ComboBox::from_id_salt(("img-owner", i))
                                                .width(90.0)
                                                .selected_text(PLAYERS[r.owner.min(8) as usize].0)
                                                .show_ui(ui, |ui| {
                                                    for o in 0..9u8 {
                                                        if ui
                                                            .selectable_value(
                                                                &mut r.owner,
                                                                o,
                                                                PLAYERS[o as usize].0,
                                                            )
                                                            .changed()
                                                        {
                                                            dlg.dirty = true;
                                                        }
                                                    }
                                                });
                                            if ui.small_button("✖").clicked() {
                                                remove = Some(i);
                                            }
                                            ui.end_row();
                                        }
                                    });
                            });
                        if let Some(i) = remove {
                            dlg.rules.remove(i);
                            dlg.dirty = true;
                        }
                        ui.horizontal(|ui| {
                            if ui.button(trd("kfx_editor.colour", "+ Colour")).clicked() {
                                dlg.rules.push(ColorRule {
                                    color: [128, 128, 128],
                                    kind: kfx_map::autotile::kinds::EARTH as u16,
                                    owner: PLAYER_NEUTRAL,
                                });
                                dlg.dirty = true;
                            }
                            if ui
                                .button(trd("kfx_editor.reset_table", "Reset table"))
                                .clicked()
                            {
                                dlg.rules = imageio::default_rules(&gd);
                                dlg.dirty = true;
                            }
                        });
                    });
                    ui.vertical(|ui| {
                        ui.label(RichText::new(trd("kfx_editor.result", "Result")).color(GOLD));
                        match (&dlg.preview, &dlg.img) {
                            (Some(t), Some(_)) => {
                                let sz = t.size_vec2();
                                let scale = (360.0 / sz.x).min(360.0 / sz.y);
                                let (rect, _) =
                                    ui.allocate_exact_size(sz * scale, egui::Sense::hover());
                                ui.painter().image(
                                    t.id(),
                                    rect,
                                    egui::Rect::from_min_max(
                                        egui::pos2(0.0, 0.0),
                                        egui::pos2(1.0, 1.0),
                                    ),
                                    Color32::WHITE,
                                );
                            }
                            _ => {
                                ui.label(
                                    RichText::new(trd("kfx_editor.load_a_png", "Load a PNG"))
                                        .color(TEXT_DIM),
                                );
                            }
                        }
                    });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            dlg.img.is_some(),
                            egui::Button::new(trd(
                                "kfx_editor.apply_one_undo_step",
                                "Apply (one undo step)",
                            )),
                        )
                        .clicked()
                    {
                        apply = true;
                    }
                    if ui.button(trd("kfx_editor.close", "Close")).clicked() {
                        close = true;
                    }
                });
            });
        if let Some(p) = load {
            let rules = dlg.rules.clone();
            self.open_image_dialog(Some(&p));
            if let Some(d) = self.image_dialog.as_mut() {
                d.rules = rules;
            }
        }
        if apply {
            if let Err(e) = self.apply_image() {
                self.status = e;
            }
        }
        if close {
            self.image_dialog = None;
        }
    }

    // --------------------------------------------------------------------------- export

    pub fn open_export_dialog(&mut self) {
        let path = self
            .doc
            .as_ref()
            .map(|d| {
                d.doc
                    .folder
                    .join(format!("map{:05}.png", d.doc.level))
                    .display()
                    .to_string()
            })
            .unwrap_or_default();
        self.export_dialog = Some(ExportDialog {
            path,
            iso: true,
            px_per_slab: 32,
            things: true,
            grid: false,
            ownership: false,
            result: None,
        });
    }

    pub(super) fn export_ui(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.export_dialog else {
            return;
        };
        let mut go = false;
        let mut close = false;
        let dims = self
            .doc
            .as_ref()
            .map(|d| d.dims())
            .unwrap_or(MapDims::new(85, 85));
        egui::Window::new(trd("kfx_editor.export_image", "Export image"))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("export").num_columns(2).show(ui, |ui| {
                    ui.label(trd("kfx_editor.file", "File"));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut dlg.path).desired_width(360.0));
                        if ui.button(trd("kfx_editor.browse", "Browse…")).clicked() {
                            if let Some(p) = rfd::FileDialog::new()
                                .add_filter(trd("kfx_editor.png_image", "PNG image"), &["png"])
                                .save_file()
                            {
                                dlg.path = p.display().to_string();
                            }
                        }
                    });
                    ui.end_row();
                    ui.label(trd("kfx_editor.view", "View"));
                    ui.horizontal(|ui| {
                        ui.radio_value(
                            &mut dlg.iso,
                            true,
                            trd("kfx_editor.isometric_game_view", "Isometric (game view)"),
                        );
                        ui.radio_value(&mut dlg.iso, false, trd("kfx_editor.top_down", "Top-down"));
                    });
                    ui.end_row();
                    ui.label(trd("kfx_editor.pixels_per_slab", "Pixels per slab"));
                    ui.add(egui::Slider::new(&mut dlg.px_per_slab, 3..=96));
                    ui.end_row();
                    ui.label(trd("kfx_editor.show", "Show"));
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut dlg.things, trd("kfx_editor.things", "Things"));
                        ui.checkbox(&mut dlg.grid, trd("kfx_editor.slab_grid", "Slab grid"));
                        ui.checkbox(
                            &mut dlg.ownership,
                            trd("kfx_editor.ownership_tint", "Ownership tint"),
                        );
                    });
                    ui.end_row();
                });
                let (w, h) = export_size(dims, dlg.iso, dlg.px_per_slab);
                ui.label(
                    RichText::new(trf(
                        "kfx_editor.about_w_x_h_pixels_rendered_in_tiles_when_larger",
                        "About {w} x {h} pixels; rendered in tiles when larger than the GPU \
                        allows.",
                        &[("w", &w), ("h", &h)],
                    ))
                    .small()
                    .color(TEXT_DIM),
                );
                if let Some(r) = &dlg.result {
                    ui.label(r);
                }
                ui.horizontal(|ui| {
                    if ui.button(trd("kfx_editor.export", "Export")).clicked() {
                        go = true;
                    }
                    if ui.button(trd("kfx_editor.close", "Close")).clicked() {
                        close = true;
                    }
                });
            });
        if go {
            let d = self.export_dialog.as_ref().unwrap();
            let (path, iso, pps, things, grid, own) = (
                PathBuf::from(d.path.trim()),
                d.iso,
                d.px_per_slab,
                d.things,
                d.grid,
                d.ownership,
            );
            let r = self.export_image(&path, iso, pps, things, grid, own);
            if let Some(d) = self.export_dialog.as_mut() {
                d.result = Some(match r {
                    Ok((w, h, tiles)) => trf(
                        "kfx_editor.wrote_path_w_x_h_tiles_tiles",
                        "Wrote {path} ({w} x {h}, {tiles} tiles)",
                        &[
                            ("path", &path.display()),
                            ("w", &w),
                            ("h", &h),
                            ("tiles", &tiles),
                        ],
                    ),
                    Err(e) => trf(
                        "kfx_editor.export_failed_e",
                        "Export failed: {e}",
                        &[("e", &e)],
                    ),
                });
            }
        }
        if close {
            self.export_dialog = None;
        }
    }

    /// Render the whole map into a PNG at `pps` pixels per slab (isometric game view or
    /// top-down), in tiles no larger than the GPU's texture limit. Returns (width, height,
    /// tiles).
    pub fn export_image(
        &mut self,
        path: &Path,
        iso: bool,
        pps: u32,
        things: bool,
        grid: bool,
        ownership: bool,
    ) -> Result<(u32, u32, usize), String> {
        let d = self.doc.as_ref().ok_or("no map")?;
        let dims = d.dims();
        let max_h = d
            .doc
            .columns
            .iter()
            .map(|c| assets::floor_height(c.solidmask))
            .max()
            .unwrap_or(5)
            .max(1) as f32;
        let (sw, sh) = (dims.stl_w() as f32, dims.stl_h() as f32);
        let mut cam = Camera {
            target: Vec3::new(sw / 2.0, 0.0, sh / 2.0),
            perspective: false,
            ..Default::default()
        };
        if !iso {
            cam.yaw = 0.0;
            cam.pitch = (-89.95f32).to_radians();
        }
        cam.zoom = (pps as f32 / 3.0) / crate::camera::PX_PER_UNIT;
        // bounds of the map box in pixels around the target
        let probe = Vec2::new(1000.0, 1000.0);
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for x in [0.0, sw] {
            for z in [0.0, sh] {
                for y in [0.0, max_h] {
                    if let Some(p) = cam.project(Vec3::new(x, y, z), probe) {
                        lo = lo.min(p);
                        hi = hi.max(p);
                    }
                }
            }
        }
        let margin = 8.0;
        let (lo, hi) = (lo - Vec2::splat(margin), hi + Vec2::splat(margin));
        let (w, h) = ((hi.x - lo.x).ceil() as u32, (hi.y - lo.y).ceil() as u32);
        if w as u64 * h as u64 > 400_000_000 {
            return Err(trf(
                "kfx_editor.w_x_h_pixels_is_too_large",
                "{w} x {h} pixels is too large",
                &[("w", &w), ("h", &h)],
            ));
        }
        let limit = self
            .gpu
            .device
            .limits()
            .max_texture_dimension_2d
            .clamp(256, 4096);
        let s = crate::camera::PX_PER_UNIT * cam.zoom;
        let back = cam.back();
        let right = Vec3::Y.cross(back).normalize_or(Vec3::X);
        let up = back.cross(right);
        // world point at the full image's pixel (0, 0) is target + offset of lo - probe/2
        let origin =
            cam.target + right * ((lo.x - probe.x / 2.0) / s) - up * ((lo.y - probe.y / 2.0) / s);
        let mut out = kfx_gfx::Rgba::new(w, h);
        let saved = (
            self.camera,
            self.view.flags,
            self.hover.take(),
            self.selected.clone(),
            self.fixed_frame,
        );
        self.selected.clear();
        self.fixed_frame = Some(self.fixed_frame.unwrap_or(0));
        let l = &self.settings.layout;
        self.view.flags = ViewFlags {
            ownership,
            grid,
            lighting: l.lighting,
            terrain: true,
        };
        let mut tiles = 0;
        let mut ty = 0;
        let result = (|| -> Result<(), String> {
            while ty < h {
                let th = limit.min(h - ty);
                let mut tx = 0;
                while tx < w {
                    let tw = limit.min(w - tx);
                    let centre =
                        Vec2::new(tx as f32 + tw as f32 / 2.0, ty as f32 + th as f32 / 2.0);
                    let mut c = cam;
                    c.target = origin + right * (centre.x / s) - up * (centre.y / s);
                    self.camera = c;
                    let vp = Vec2::new(tw as f32, th as f32);
                    let mut lines = vec![];
                    let sprites = if things {
                        self.build_things(vp, &mut lines)
                    } else {
                        vec![]
                    };
                    self.view
                        .upload_sprites(&self.gpu.queue, &mut self.sprite_atlas);
                    self.view.render(
                        &self.gpu.device,
                        &self.gpu.queue,
                        &c,
                        (tw, th),
                        self.fixed_frame.unwrap_or(0),
                        &lines,
                        &[],
                        &[],
                        &sprites,
                    );
                    let (rw, rh, px) = self
                        .view
                        .read_back(&self.gpu.device, &self.gpu.queue)
                        .ok_or("read back failed")?;
                    let tile = kfx_gfx::Rgba {
                        w: rw,
                        h: rh,
                        data: px,
                    };
                    out.blit(&tile, tx as i32, ty as i32);
                    tiles += 1;
                    tx += tw;
                }
                ty += th;
            }
            Ok(())
        })();
        (
            self.camera,
            self.view.flags,
            self.hover,
            self.selected,
            self.fixed_frame,
        ) = saved;
        // the viewport texture has to be registered again at its size
        self.view_size = (0, 0);
        result?;
        // the clear colour is opaque: keep it (a dark background like the game)
        let png = out.to_png().map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, png).map_err(|e| format!("{}: {e}", path.display()))?;
        self.status = trf(
            "kfx_editor.exported_path_w_x_h",
            "Exported {path} ({w} x {h})",
            &[("path", &path.display()), ("w", &w), ("h", &h)],
        );
        Ok((w, h, tiles))
    }

    // --------------------------------------------------------------------------- preferences

    pub(super) fn prefs_window(&mut self, ctx: &egui::Context) {
        if !self.show_prefs {
            return;
        }
        let mut open = true;
        let mut changed = false;
        let mut reset = false;
        let before = self.settings.clone();
        egui::Window::new(trd("kfx_editor.preferences", "Preferences"))
            .open(&mut open)
            .resizable(true)
            .default_size([640.0, 520.0])
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (t, label) in PREF_TABS {
                        if ui.selectable_label(self.prefs_tab == t, label).clicked() {
                            self.prefs_tab = t.to_string();
                        }
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(440.0)
                    .show(ui, |ui| match self.prefs_tab.as_str() {
                        "Play" => self.prefs_play(ui),
                        "Placement" => self.prefs_placement(ui),
                        "Interface" => self.prefs_ui(ui),
                        "Camera" => self.prefs_camera(ui),
                        "Graphics" => self.prefs_graphics(ui),
                        "Decoration" => changed |= self.prefs_decoration(ui),
                        "Shortcuts" => self.shortcuts_editor(ui),
                        "Updates" => self.prefs_updates(ui),
                        _ => self.prefs_files(ui),
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .button(trd("kfx_editor.reset_to_defaults", "Reset to defaults"))
                        .on_hover_text(trd(
                            "kfx_editor.everything_except_the_install_folder_the_recent",
                            "Everything except the install folder, the recent maps and the \
                            window size",
                        ))
                        .clicked()
                    {
                        reset = true;
                    }
                    ui.label(
                        RichText::new(
                            S::path()
                                .map(|p| {
                                    trf(
                                        "kfx_editor.saved_in_p",
                                        "Saved in {p}",
                                        &[("p", &p.display())],
                                    )
                                })
                                .unwrap_or_default(),
                        )
                        .small()
                        .color(TEXT_DIM),
                    );
                });
            });
        self.show_prefs = open;
        if reset {
            self.settings.reset();
            self.keymap = Keymap::new(&self.settings.shortcuts);
            changed = true;
        }
        if changed || self.settings != before {
            self.apply_settings(&before);
            self.save_settings();
        }
    }

    /// Apply changed settings that need more than reading them each frame.
    pub(super) fn apply_settings(&mut self, before: &crate::settings::Settings) {
        if before.decoration != self.settings.decoration
            || before.decor_families != self.settings.decor_families
        {
            self.apply_decor_prefs();
        }
        if before.graphics.msaa != self.settings.graphics.msaa {
            self.rebuild_renderer();
        }
        if before.ui.script_font_size != self.settings.ui.script_font_size
            || before.ui.script_light != self.settings.ui.script_light
        {
            if let Some(s) = self.scripts.as_mut() {
                s.ws.set_font_size(self.settings.ui.script_font_size);
                s.ws.set_light(self.settings.ui.script_light);
            }
        }
        if before.place != self.settings.place {
            self.place_defaults = self.settings.place.clone();
        }
        self.apply_ui_settings();
    }

    /// UI scale and font size.
    pub(crate) fn apply_ui_settings(&mut self) {
        let u = &self.settings.ui;
        crate::lang::apply(&u.language);
        self.ctx.set_zoom_factor(u.scale.clamp(0.5, 3.0));
        let fs = u.font_size.clamp(8.0, 28.0);
        self.ctx.global_style_mut(|s| {
            for (ts, f) in s.text_styles.iter_mut() {
                f.size = match ts {
                    egui::TextStyle::Small => fs * 0.75,
                    egui::TextStyle::Heading => fs * 1.4,
                    egui::TextStyle::Monospace => fs * 0.95,
                    _ => fs,
                };
            }
        });
    }

    /// New renderer (MSAA changed): upload the document again.
    fn rebuild_renderer(&mut self) {
        let samples = if self.settings.graphics.msaa {
            crate::render::SAMPLES
        } else {
            1
        };
        self.view = ViewRenderer::with_samples(&self.gpu.device, samples);
        self.view_size = (0, 0);
        if let Some(t) = self.view_tex.take() {
            self.gpu.egui_renderer.write().free_texture(&t);
        }
        if self.doc.is_some() {
            self.rebuild_view();
            self.sprite_atlas.clear();
        }
    }

    fn prefs_files(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new(trd("kfx_editor.files", "Files")).color(GOLD));
        ui.horizontal(|ui| {
            ui.label(trd("kfx_editor.keeperfx_folder_e6ff", "KeeperFX folder:"));
            ui.label(
                RichText::new(
                    self.install()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| trd("kfx_editor.none", "(none)").into()),
                )
                .strong(),
            );
            if ui.button(trd("kfx_editor.change", "Change…")).clicked() {
                self.first_run.open = true;
                self.first_run.text = self
                    .install()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
            }
        });
        if let Some(i) = self.install().map(Path::to_path_buf) {
            let c = super::extras::install_checks(&i);
            ui.label(RichText::new(c.summary()).small().color(if c.writable {
                TEXT_DIM
            } else {
                Color32::from_rgb(240, 160, 80)
            }));
        }
        ui.label(
            RichText::new(
                crate::settings::config_dir()
                    .map(|d| {
                        trf(
                            "kfx_editor.settings_backups_and_autosaves_d",
                            "Settings, backups and autosaves: {d}",
                            &[("d", &d.display())],
                        )
                    })
                    .unwrap_or_default(),
            )
            .small()
            .color(TEXT_DIM),
        );
        ui.separator();
        let s = &mut self.settings;
        ui.checkbox(
            &mut s.rnc_notice,
            trd(
                "kfx_editor.tell_me_when_a_map_has_rnc_compressed_files",
                "Tell me when a map has RNC-compressed files",
            ),
        )
        .on_hover_text(trd(
            "kfx_editor.they_are_always_read_saving_writes_them_uncompre",
            "They are always read; saving writes them uncompressed",
        ));
        ui.horizontal(|ui| {
            ui.label(trd(
                "kfx_editor.crash_recovery_autosave_every",
                "Crash-recovery autosave every",
            ));
            ui.add(
                egui::DragValue::new(&mut s.autosave_secs)
                    .range(0..=3600)
                    .suffix(trd("kfx_editor.s", " s")),
            );
            ui.label(
                RichText::new(trd(
                    "kfx_editor.0_off_only_while_there_are_unsaved_changes",
                    "(0 = off; only while there are unsaved changes)",
                ))
                .small()
                .color(TEXT_DIM),
            );
        });
        ui.horizontal(|ui| {
            ui.label(trd("kfx_editor.keep_backups_up_to", "Keep backups up to"));
            ui.add(
                egui::DragValue::new(&mut s.backup_limit_mb)
                    .range(0..=100_000)
                    .suffix(trd("kfx_editor.mib", " MiB")),
            );
            ui.label(
                RichText::new(trd(
                    "kfx_editor.oldest_removed_first_0_keep_all",
                    "(oldest removed first; 0 = keep all)",
                ))
                .small()
                .color(TEXT_DIM),
            );
        });
    }

    fn prefs_play(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new(trd("kfx_editor.play_testing", "Play-testing")).color(GOLD));
        let detected = self.install().and_then(crate::playtest::detect_exe);
        let p = &mut self.settings.play;
        ui.horizontal(|ui| {
            ui.label(trd("kfx_editor.game_executable", "Game executable"));
            let mut t = p
                .exe
                .as_ref()
                .map(|e| e.display().to_string())
                .unwrap_or_default();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut t)
                        .desired_width(320.0)
                        .hint_text(
                            detected
                                .as_ref()
                                .map(|d| d.display().to_string())
                                .unwrap_or_else(|| trd("kfx_editor.not_found", "not found").into()),
                        ),
                )
                .changed()
            {
                p.exe = (!t.trim().is_empty()).then(|| PathBuf::from(t.trim()));
            }
            if ui.button(trd("kfx_editor.browse", "Browse…")).clicked() {
                if let Some(f) = rfd::FileDialog::new().pick_file() {
                    p.exe = Some(f);
                }
            }
        });
        ui.label(
            RichText::new(match (&p.exe, &detected) {
                (Some(e), _) if e.is_file() => {
                    trf("kfx_editor.using_e", "Using {e}", &[("e", &e.display())])
                }
                (Some(e), _) => trf(
                    "kfx_editor.e_does_not_exist",
                    "{e} does not exist",
                    &[("e", &e.display())],
                ),
                (None, Some(d)) => trf(
                    "kfx_editor.detected_d",
                    "Detected {d}",
                    &[("d", &d.display())],
                ),
                (None, None) => trd(
                    "kfx_editor.no_keeperfx_keeperfx_exe_in_the_install",
                    "No keeperfx / keeperfx.exe in the install",
                ),
            })
            .small()
            .color(TEXT_DIM),
        );
        ui.horizontal(|ui| {
            ui.label(trd("kfx_editor.wrapper_command", "Wrapper command"));
            ui.add(
                egui::TextEdit::singleline(&mut p.wrapper)
                    .desired_width(240.0)
                    .hint_text(if cfg!(windows) {
                        String::new()
                    } else {
                        trd(
                            "kfx_editor.wine_automatic_for_exe",
                            "wine (automatic for .exe)",
                        )
                    }),
            );
        });
        ui.horizontal(|ui| {
            ui.label(trd("kfx_editor.extra_arguments", "Extra arguments"));
            ui.add(
                egui::TextEdit::singleline(&mut p.extra_args)
                    .desired_width(320.0)
                    .hint_text(trd("kfx_editor.fps_30_nomods", "-fps 30 -nomods")),
            );
        });
        ui.checkbox(
            &mut p.no_intro,
            trd(
                "kfx_editor.skip_the_intro_nointro",
                "Skip the intro (-nointro)",
            ),
        );
        ui.checkbox(
            &mut p.skip_heart_zoom,
            trd(
                "kfx_editor.skip_the_heart_zoom_skipheartzoom",
                "Skip the heart zoom (-skipheartzoom)",
            ),
        );
        ui.horizontal(|ui| {
            ui.checkbox(&mut p.windowed, trd("kfx_editor.windowed", "Windowed"));
            ui.add_enabled(
                p.windowed,
                egui::TextEdit::singleline(&mut p.window_res).desired_width(90.0),
            );
            ui.label(
                RichText::new(trd(
                    "kfx_editor.cfg_ingame_res_wxhw32",
                    "(-cfg INGAME_RES=WxHw32)",
                ))
                .small()
                .color(TEXT_DIM),
            );
        });
        ui.checkbox(
            &mut p.cheats,
            trd("kfx_editor.cheats_alex", "Cheats (-alex)"),
        );
        ui.checkbox(
            &mut p.no_sound,
            trd("kfx_editor.no_sound_nosound", "No sound (-nosound)"),
        );
        ui.checkbox(
            &mut p.one_player_mp,
            trd(
                "kfx_editor.multiplayer_maps_computer_keepers_for_the_other",
                "Multiplayer maps: computer keepers for the other players (-1player)",
            ),
        );
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut p.alt_input, "-altinput")
                .on_hover_text(trd(
                    "kfx_editor.do_not_grab_the_mouse",
                    "Do not grab the mouse",
                ));
            ui.checkbox(&mut p.show_game_turns, "-show_game_turns");
            ui.checkbox(&mut p.frame_step, "-framestep");
            ui.checkbox(&mut p.timer, "-timer");
            ui.checkbox(&mut p.dbg_pathfind, "-dbgpathfind");
            ui.checkbox(&mut p.dbg_shots, "-dbgshots");
            ui.checkbox(&mut p.no_mods, "-nomods")
                .on_hover_text(trd("kfx_editor.start_without_mods", "Start without mods"));
        });
        ui.checkbox(
            &mut p.save_before_play,
            trd(
                "kfx_editor.play_f5_saves_unsaved_changes_first",
                "Play (F5) saves unsaved changes first",
            ),
        );
        ui.separator();
        ui.label(
            RichText::new(trd(
                "kfx_editor.command_for_the_open_map",
                "Command for the open map:",
            ))
            .small(),
        );
        let cmd = match self.play_launch() {
            Ok((l, _)) => self.live_prepare_launch(l).display(),
            Err(e) => format!("({e})"),
        };
        ui.label(RichText::new(cmd).monospace().small().color(TEXT_DIM));
        self.prefs_audiolive(ui);
    }

    fn prefs_placement(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new(trd("kfx_editor.placement_rules", "Placement rules")).color(GOLD));
        ui.checkbox(
            &mut self.settings.ownable_natural,
            trd(
                "kfx_editor.ownable_natural_terrain",
                "Ownable natural terrain",
            ),
        )
        .on_hover_text(trd(
            "kfx_editor.earth_rock_gold_and_liquids_take_the_painted_own",
            "Earth, rock, gold and liquids take the painted owner (else they stay neutral)",
        ));
        ui.horizontal(|ui| {
            ui.label(trd("kfx_editor.fortify_wall_art", "Fortify wall art"));
            ui.radio_value(
                &mut self.settings.wall_art,
                crate::fortify::WallArt::Grouped,
                trd(
                    "kfx_editor.grouped_the_game_s_rule",
                    "Grouped (the game's rule)",
                ),
            )
            .on_hover_text(trd(
                "kfx_editor.torch_walls_every_fifth_slab_drapes_beside_claim",
                "Torch walls every fifth slab, drapes beside claimed ground, then \
                bands of twins / woman / pair walls by the distance from the heart",
            )); // choose_pretty_type, src/map_blocks.c:2579
            ui.radio_value(
                &mut self.settings.wall_art,
                crate::fortify::WallArt::Random,
                trd("kfx_editor.random", "Random"),
            );
        });
        ui.label(
            RichText::new(trd(
                "kfx_editor.rounded_corners_a_corner_of_a_block_between_two",
                "Rounded corners (a corner of a block between two sides of another \
                terrain takes that terrain; decoration only)",
            ))
            .small()
            .color(TEXT_DIM),
        );
        ui.horizontal_wrapped(|ui| {
            for (label, v) in self.settings.rounding.options() {
                ui.checkbox(v, label);
            }
        });
        ui.checkbox(
            &mut self.settings.place_anywhere,
            trd("kfx_editor.place_things_anywhere", "Place things anywhere"),
        )
        .on_hover_text(trd(
            "kfx_editor.allow_things_inside_walls_and_rock_columns_of_5_9caa",
            "Allow things inside walls and rock (columns of 5 cubes or more); \
            lights, action points and doors are always allowed",
        ));
        ui.separator();
        ui.heading(
            RichText::new(trd("kfx_editor.placement_defaults", "Placement defaults")).color(GOLD),
        );
        let d = &mut self.settings.place;
        egui::Grid::new("place-prefs")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label(trd("kfx_editor.creature_level", "Creature level"));
                ui.add(egui::Slider::new(&mut d.level, 1..=10));
                ui.end_row();
                ui.label(trd("kfx_editor.facing", "Facing"));
                ui.add(egui::DragValue::new(&mut d.orientation).range(0..=2047));
                ui.end_row();
                ui.label(trd("kfx_editor.creature_gold", "Creature gold"));
                ui.add(egui::DragValue::new(&mut d.creature_gold).range(0..=1_000_000_000));
                ui.end_row();
                ui.label(trd("kfx_editor.creature_health", "Creature health %"));
                ui.add(egui::DragValue::new(&mut d.health).range(0..=100));
                ui.end_row();
                ui.label(trd("kfx_editor.gold_value", "Gold value"));
                ui.add(egui::DragValue::new(&mut d.gold_value).range(0..=1_000_000_000));
                ui.end_row();
                ui.label(trd("kfx_editor.doors_locked", "Doors locked"));
                ui.checkbox(&mut d.locked, "");
                ui.end_row();
                ui.label(trd(
                    "kfx_editor.light_range_intensity_height",
                    "Light range / intensity / height",
                ));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut d.light_range)
                            .range(0.0..=255.0)
                            .speed(0.1),
                    );
                    ui.add(egui::DragValue::new(&mut d.light_intensity).range(0..=255));
                    ui.add(
                        egui::DragValue::new(&mut d.light_height)
                            .range(0.0..=8.0)
                            .speed(0.1),
                    );
                });
                ui.end_row();
                ui.label(trd("kfx_editor.action_point_range", "Action point range"));
                ui.add(
                    egui::DragValue::new(&mut d.ap_range)
                        .range(0.0..=255.0)
                        .speed(0.1),
                );
                ui.end_row();
                ui.label(trd(
                    "kfx_editor.effect_generator_range",
                    "Effect generator range",
                ));
                ui.add(
                    egui::DragValue::new(&mut d.effect_range)
                        .range(0.0..=255.0)
                        .speed(0.1),
                );
                ui.end_row();
            });
        ui.checkbox(
            &mut self.editable_border,
            trd(
                "kfx_editor.editable_border_this_session",
                "Editable border (this session)",
            ),
        );
    }

    fn prefs_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new(trd("kfx_editor.interface", "Interface")).color(GOLD));
        let u = &mut self.settings.ui;
        egui::Grid::new("ui-prefs").num_columns(2).show(ui, |ui| {
            ui.label(trd("kfx_editor.interface_scale", "Interface scale"));
            ui.add(egui::Slider::new(&mut u.scale, 0.6..=2.5).step_by(0.05));
            ui.end_row();
            ui.label(trd("kfx_editor.font_size", "Font size"));
            ui.add(egui::Slider::new(&mut u.font_size, 9.0..=24.0).step_by(0.5));
            ui.end_row();
            ui.label(trd(
                "kfx_editor.script_editor_font_size",
                "Script editor font size",
            ));
            ui.add(egui::Slider::new(&mut u.script_font_size, 8.0..=32.0).step_by(0.5));
            ui.end_row();
            ui.label(trd("kfx_editor.script_editor_theme", "Script editor theme"));
            ui.horizontal(|ui| {
                ui.radio_value(&mut u.script_light, false, trd("kfx_editor.dark", "Dark"));
                ui.radio_value(&mut u.script_light, true, trd("kfx_editor.light", "Light"));
            });
            ui.end_row();
            ui.label(trd("kfx_editor.language", "Language"));
            let langs = crate::lang::languages();
            let cur = langs
                .iter()
                .find(|l| l.0 == u.language)
                .map(|l| l.1.clone())
                .unwrap_or_else(|| u.language.clone());
            egui::ComboBox::from_id_salt("ui-language")
                .selected_text(cur)
                .show_ui(ui, |ui| {
                    for (code, name) in &langs {
                        ui.selectable_value(&mut u.language, code.clone(), name);
                    }
                })
                .response
                .on_hover_text(trd(
                    "kfx_editor.language_hint",
                    "Catalogues are read from data/i18n \
                    next to the editor and from the i18n folder of the editor's settings folder; \
                    texts a catalogue lacks stay in English.",
                ));
            ui.end_row();
        });
    }

    fn prefs_camera(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new(trd("kfx_editor.camera", "Camera")).color(GOLD));
        let c = &mut self.settings.camera;
        egui::Grid::new("cam-prefs").num_columns(2).show(ui, |ui| {
            ui.label(trd(
                "kfx_editor.pan_speed_keys_edges",
                "Pan speed (keys, edges)",
            ));
            ui.add(
                egui::Slider::new(&mut c.pan_speed, 100.0..=3000.0)
                    .suffix(trd("kfx_editor.px_s", " px/s")),
            );
            ui.end_row();
            ui.label(trd("kfx_editor.zoom_speed", "Zoom speed"));
            ui.add(egui::Slider::new(&mut c.zoom_speed, 0.2..=4.0));
            ui.end_row();
            ui.label(trd("kfx_editor.orbit_speed", "Orbit speed"));
            ui.add(egui::Slider::new(&mut c.orbit_speed, 0.2..=4.0));
            ui.end_row();
            ui.label(trd("kfx_editor.invert", "Invert"));
            ui.horizontal(|ui| {
                ui.checkbox(&mut c.invert_orbit, trd("kfx_editor.orbit", "orbit"));
                ui.checkbox(
                    &mut c.invert_zoom,
                    trd("kfx_editor.wheel_zoom", "wheel zoom"),
                );
            });
            ui.end_row();
            ui.label(trd("kfx_editor.fly_view_speed", "Fly view speed"));
            ui.add(
                egui::Slider::new(&mut c.fly_speed, 1.0..=60.0)
                    .suffix(trd("kfx_editor.subtiles_s", " subtiles/s")),
            );
            ui.end_row();
            ui.label(trd("kfx_editor.fly_view_collision", "Fly view collision"));
            ui.checkbox(
                &mut c.fly_collision,
                trd(
                    "kfx_editor.stop_at_walls_stay_above_the_floor",
                    "Stop at walls, stay above the floor",
                ),
            );
            ui.end_row();
            ui.label(trd("kfx_editor.edge_panning", "Edge panning"));
            ui.checkbox(
                &mut c.edge_pan,
                trd(
                    "kfx_editor.pan_when_the_pointer_is_at_the_edge_of_the_view",
                    "Pan when the pointer is at the edge of the view",
                ),
            );
            ui.end_row();
            ui.label(trd("kfx_editor.smoothing", "Smoothing"));
            ui.add(egui::Slider::new(&mut c.smoothing, 0.0..=0.95));
            ui.end_row();
        });
    }

    fn prefs_graphics(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new(trd("kfx_editor.graphics", "Graphics")).color(GOLD));
        let g = &mut self.settings.graphics;
        ui.checkbox(
            &mut g.msaa,
            trd(
                "kfx_editor.anti_aliasing_4x_msaa",
                "Anti-aliasing (4x MSAA)",
            ),
        );
        ui.checkbox(
            &mut g.smooth_scaling,
            trd(
                "kfx_editor.smooth_scaling_of_the_view_and_palette_icons_lin",
                "Smooth scaling of the view and palette icons (linear filtering)",
            ),
        );
        let l = &mut self.settings.layout;
        ui.checkbox(&mut l.lighting, trd("kfx_editor.lighting", "Lighting"));
        ui.checkbox(
            &mut l.layers.animate,
            trd("kfx_editor.animate_sprites", "Animate sprites"),
        );
        ui.checkbox(&mut l.show_grid, trd("kfx_editor.slab_grid", "Slab grid"));
        ui.checkbox(
            &mut l.show_ownership,
            trd("kfx_editor.ownership_tint", "Ownership tint"),
        );
        ui.label(
            RichText::new(trd(
                "kfx_editor.terrain_textures_are_read_texel_by_texel_like_th",
                "Terrain textures are read texel by texel like the game (no filtering).",
            ))
            .small()
            .color(TEXT_DIM),
        );
    }

    fn prefs_decoration(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.heading(RichText::new(trd("kfx_editor.decoration", "Decoration")).color(GOLD));
        ui.label(trd(
            "kfx_editor.placed_and_regenerated_slabs_get_random_cube_var",
            "Placed and regenerated slabs get random cube variants (earth, path \
            stones, gold, gems, bricks…), deterministic per map.",
        ));
        changed |= ui
            .checkbox(
                &mut self.settings.decoration,
                trd("kfx_editor.decorate_placed_slabs", "Decorate placed slabs"),
            )
            .changed();
        if let Some(d) = &self.doc {
            let dec = &d.decorator;
            let fams: Vec<(String, bool)> = dec
                .families
                .cubes
                .iter()
                .enumerate()
                .map(|(k, f)| (f.name.clone(), dec.cube_enabled[k]))
                .collect();
            ui.label(
                RichText::new(trd(
                    "kfx_editor.families_from_cubes_cfg",
                    "Families (from cubes.cfg)",
                ))
                .small()
                .color(TEXT_DIM),
            );
            egui::Grid::new("fam").num_columns(3).show(ui, |ui| {
                for (n, (name, on)) in fams.iter().enumerate() {
                    let mut v = *on;
                    if ui.checkbox(&mut v, name).changed() {
                        self.settings
                            .decor_families
                            .retain(|(x, _)| !x.eq_ignore_ascii_case(name));
                        self.settings.decor_families.push((name.clone(), v));
                        changed = true;
                    }
                    if n % 3 == 2 {
                        ui.end_row();
                    }
                }
            });
        } else {
            ui.label(
                RichText::new(trd(
                    "kfx_editor.open_a_map_to_see_its_decoration_families",
                    "Open a map to see its decoration families",
                ))
                .color(TEXT_DIM),
            );
        }
        changed
    }

    /// The shortcuts editor (Preferences tab and its own window).
    pub(super) fn shortcuts_editor(&mut self, ui: &mut egui::Ui) {
        ui.heading(
            RichText::new(trd("kfx_editor.keyboard_shortcuts", "Keyboard shortcuts")).color(GOLD),
        );
        ui.label(
            RichText::new(trd(
                "kfx_editor.click_a_shortcut_then_press_the_new_key_combinat",
                "Click a shortcut, then press the new key combination (Esc cancels, \
                Backspace removes it).",
            ))
            .small()
            .color(TEXT_DIM),
        );
        // capture
        if let Some(id) = self.shortcut_capture {
            if let Some(sc) = shortcuts::captured(&self.ctx) {
                if sc.key == egui::Key::Escape && !sc.ctrl && !sc.shift && !sc.alt {
                    self.shortcut_capture = None;
                } else if sc.key == egui::Key::Backspace && !sc.ctrl && !sc.shift && !sc.alt {
                    self.keymap.set(id, None);
                    self.shortcut_capture = None;
                } else {
                    self.keymap.set(id, Some(sc));
                    self.shortcut_capture = None;
                }
                self.settings.shortcuts = self.keymap.overrides();
            }
        }
        let conflicts = self.keymap.conflicts();
        if !conflicts.is_empty() {
            for (a, b) in &conflicts {
                let la = ACTIONS
                    .iter()
                    .find(|d| d.id == *a)
                    .map(|d| d.title())
                    .unwrap_or_else(|| a.to_string());
                let lb = ACTIONS
                    .iter()
                    .find(|d| d.id == *b)
                    .map(|d| d.title())
                    .unwrap_or_else(|| b.to_string());
                ui.colored_label(
                    Color32::from_rgb(240, 120, 90),
                    trf(
                        "kfx_editor.conflict_a_is_used_by_la_and_lb",
                        "Conflict: {key} is used by \"{la}\" and \"{lb}\"",
                        &[("key", &self.keymap.label(a)), ("la", &la), ("lb", &lb)],
                    ),
                );
            }
        }
        let mut group = "";
        egui::Grid::new("shortcuts")
            .num_columns(3)
            .striped(true)
            .show(ui, |ui| {
                for d in ACTIONS {
                    if d.group != group {
                        group = d.group;
                        ui.label(RichText::new(d.group_title()).color(GOLD_DIM).strong());
                        ui.end_row();
                    }
                    let conflict = conflicts.iter().any(|(a, b)| *a == d.id || *b == d.id);
                    ui.label(if conflict {
                        RichText::new(d.title()).color(Color32::from_rgb(240, 120, 90))
                    } else {
                        RichText::new(d.title())
                    });
                    let text = if self.shortcut_capture == Some(d.id) {
                        trd("kfx_editor.press_keys", "Press keys…").to_string()
                    } else {
                        self.keymap
                            .get(d.id)
                            .map(|s| s.text())
                            .unwrap_or_else(|| trd("kfx_editor.none", "(none)").into())
                    };
                    if ui
                        .add_enabled(
                            d.editable,
                            egui::Button::new(RichText::new(text).monospace()),
                        )
                        .on_disabled_hover_text(trd(
                            "kfx_editor.system_clipboard_keys",
                            "System clipboard keys",
                        ))
                        .clicked()
                    {
                        self.shortcut_capture = Some(d.id);
                    }
                    let default = Shortcut::parse(d.default);
                    if ui
                        .add_enabled(
                            d.editable && self.keymap.get(d.id) != default,
                            egui::Button::new(trd("kfx_editor.default", "Default")).small(),
                        )
                        .clicked()
                    {
                        self.keymap.set(d.id, default);
                        self.settings.shortcuts = self.keymap.overrides();
                    }
                    ui.end_row();
                }
            });
    }

    pub(super) fn shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let mut open = true;
        let before = self.settings.shortcuts.clone();
        egui::Window::new(trd("kfx_editor.keyboard_shortcuts", "Keyboard shortcuts"))
            .open(&mut open)
            .default_size([560.0, 600.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.shortcuts_editor(ui));
            });
        self.show_shortcuts = open;
        if before != self.settings.shortcuts {
            self.save_settings();
        }
    }

    // --------------------------------------------------------------------------- help

    pub(super) fn help_windows(&mut self, ctx: &egui::Context) {
        let keymap = self.keymap.clone();
        egui::Window::new(trd("kfx_editor.controls", "Controls"))
            .open(&mut self.show_controls)
            .default_size([600.0, 620.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(trd("kfx_editor.mouse", "Mouse"))
                                .color(GOLD)
                                .strong(),
                        );
                        egui::Grid::new("mouse-ref")
                            .num_columns(2)
                            .striped(true)
                            .show(ui, |ui| {
                                for (k, v) in shortcuts::MOUSE {
                                    ui.label(RichText::new(*k).color(GOLD));
                                    ui.label(*v);
                                    ui.end_row();
                                }
                            });
                        let mut group = "";
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(trd(
                                "kfx_editor.keyboard_configuration_keyboard_shortcuts_to_cha",
                                "Keyboard (Configuration > Keyboard shortcuts to change)",
                            ))
                            .color(GOLD)
                            .strong(),
                        );
                        egui::Grid::new("key-ref")
                            .num_columns(2)
                            .striped(true)
                            .show(ui, |ui| {
                                for d in ACTIONS {
                                    if d.group != group {
                                        group = d.group;
                                        ui.label(RichText::new(d.group_title()).color(GOLD_DIM));
                                        ui.end_row();
                                    }
                                    ui.label(
                                        RichText::new(
                                            keymap
                                                .get(d.id)
                                                .map(|s| s.label())
                                                .unwrap_or_else(|| "-".into()),
                                        )
                                        .monospace()
                                        .color(GOLD),
                                    );
                                    ui.label(d.title());
                                    ui.end_row();
                                }
                            });
                    });
            });
        egui::Window::new(trd("kfx_editor.about", "About"))
            .open(&mut self.show_about)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(trd("kfx_editor.keeperfx_map_editor", "KeeperFX Map Editor"))
                        .color(GOLD)
                        .strong()
                        .size(18.0),
                );
                ui.label(trf(
                    "kfx_editor.version_env",
                    "Version {env}",
                    &[("env", &env!("CARGO_PKG_VERSION"))],
                ));
                ui.label(trd(
                    "kfx_editor.licence_gpl_2_0_or_later",
                    "Licence: GPL-2.0-or-later",
                ));
                ui.add_space(6.0);
                ui.label(trd(
                    "kfx_editor.a_map_and_level_script_editor_for_keeperfx",
                    "A map and level script editor for KeeperFX, the open-source engine \
                    for Dungeon Keeper 1.",
                ));
                ui.label(trd(
                    "kfx_editor.formats_slab_generation_and_rules_follow_the_kee",
                    "Formats, slab generation and rules follow the KeeperFX source code. \
                    Thanks to the KeeperFX project and the authors of its wiki.",
                ));
                ui.add_space(4.0);
                for (n, u) in WIKI.iter().take(1) {
                    ui.hyperlink_to(*n, *u);
                }
                ui.hyperlink_to(
                    trd("kfx_editor.keeperfx_project", "KeeperFX project"),
                    "https://github.com/dkfans/keeperfx",
                );
                ui.label(
                    RichText::new(trd(
                        "kfx_editor.dungeon_keeper_is_a_trademark_of_electronic_arts",
                        "Dungeon Keeper is a trademark of Electronic Arts. The editor ships \
                        no game data: it reads your KeeperFX install.",
                    ))
                    .small()
                    .color(TEXT_DIM),
                );
            });
    }
}

/// Approximate output size of an export.
pub fn export_size(dims: MapDims, iso: bool, pps: u32) -> (u32, u32) {
    let s = pps as f32 / 3.0;
    let (w, h) = (dims.stl_w() as f32 * s, dims.stl_h() as f32 * s);
    if iso {
        let d = (w * w + h * h).sqrt() * 0.5f32.sqrt();
        ((d * 1.42) as u32 + 16, (d * 0.73 + 8.0 * s) as u32 + 16)
    } else {
        (w as u32 + 16, h as u32 + 16)
    }
}
