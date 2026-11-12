//! Level properties > "Pick on the land view…": the campaign's land view pictures, shown as the
//! game draws them, and the ensign position / zoom point set by clicking on the picture.
//!
//! Files (src/front_landview.c:931-1013 `load_map_and_window`): `LAND_VIEW = <picture> <window>`
//! names `<picture>.raw` (8-bit pixels, `LANDVIEW_MAP_WIDTH` x `LANDVIEW_MAP_HEIGHT` = 1280 x 960,
//! src/config.h:39, possibly RNC-packed), `<picture>.pal` (768-byte VGA palette) and `<window>.dat`
//! (the frame sprite), all in the campaign's `LAND_LOCATION` folder (`FGrp_LandView`,
//! src/config.c:1381, relative to the install). `ENSIGN_POS` is the picture pixel where the bottom
//! centre of the ensign stands (x 160..1120, y 120..840 per the campaign cfg comments);
//! `ENSIGN_ZOOM` is the point zoomed into when the level is chosen (src/front_landview.c:678).
use super::*;
use kfx_formats::levelinfo::LevelInfo;
use kfx_prefab::i18n::{trd, trf};

pub const LAND_W: u32 = 1280;
pub const LAND_H: u32 = 960;
/// Where an ensign may stand (campaign cfg comment: "X is ranged 160..1120, Y is 120..840").
pub const ENSIGN_X: (i32, i32) = (160, 1120);
pub const ENSIGN_Y: (i32, i32) = (120, 840);

/// A campaign with a land view folder.
#[derive(Clone, Debug)]
pub struct LandCampaign {
    pub name: String,
    pub cfg: PathBuf,
    pub dir: PathBuf,
}

pub struct LandPicker {
    pub campaigns: Vec<LandCampaign>,
    pub campaign: usize,
    /// The level belongs to `campaigns[0]`.
    pub own: bool,
    /// Pictures (`.raw` with a `.pal`) and window frames (`.dat`) of the chosen campaign.
    pub views: Vec<String>,
    pub windows: Vec<String>,
    pub view: String,
    pub window: String,
    pub pos: (i32, i32),
    pub zoom: (i32, i32),
    /// The zoom point follows the position (one click sets both).
    pub zoom_follows: bool,
    /// Decoded picture (name, texture or why not).
    image: Option<(String, Result<egui::TextureHandle, String>)>,
}

/// Campaigns of the install that have a `LAND_LOCATION` folder; `own` (the level's campaign
/// cfg) first when it has one.
pub fn land_campaigns(install: &Path, own: Option<&Path>) -> Vec<LandCampaign> {
    let mut cfgs: Vec<PathBuf> = kfx_gfx::install::resolve(install, "campgns")
        .and_then(|d| std::fs::read_dir(d).ok())
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cfg")))
                .collect()
        })
        .unwrap_or_default();
    cfgs.sort();
    if let Some(o) = own {
        cfgs.retain(|c| c != o);
        cfgs.insert(0, o.to_path_buf());
    }
    cfgs.into_iter()
        .filter_map(|cfg| {
            let c = kfx_formats::campaign::Campaign::load(&cfg).ok()?.common();
            let loc = c.land_location.as_deref()?.replace('\\', "/");
            let dir = kfx_gfx::install::resolve(install, loc.trim()).filter(|d| d.is_dir())?;
            let stem = cfg
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let name = c
                .name
                .filter(|n| !n.trim().is_empty())
                .map(|n| format!("{} ({stem})", n.trim()))
                .unwrap_or(stem);
            Some(LandCampaign { name, cfg, dir })
        })
        .collect()
}

/// Land view pictures (`.raw` with a matching `.pal`) and window frames (`.dat` other than the
/// `maphand` pointer sprite) of a land folder, sorted, lower-case without extension.
pub fn land_files(dir: &Path) -> (Vec<String>, Vec<String>) {
    let names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let stem = |n: &str, ext: &str| n.strip_suffix(ext).map(str::to_string);
    let mut views: Vec<String> = names
        .iter()
        .filter_map(|n| stem(n, ".raw"))
        .filter(|s| names.contains(&format!("{s}.pal")))
        .collect();
    let mut windows: Vec<String> = names
        .iter()
        .filter_map(|n| stem(n, ".dat"))
        .filter(|s| !s.starts_with("maphand"))
        .collect();
    views.sort();
    windows.sort();
    (views, windows)
}

/// Decode land view picture `name` of folder `dir` (`<name>.raw` + `<name>.pal`) to RGBA. A
/// short picture is padded black, like the game's buffer.
pub fn decode_land_view(dir: &Path, name: &str) -> Result<kfx_gfx::Rgba, String> {
    let find = |ext: &str| {
        kfx_map::gamedata::find_ci(dir, &format!("{name}.{ext}")).ok_or_else(|| {
            trf(
                "kfx_editor.name_ext_not_found_in_dir",
                "{name}.{ext} not found in {dir}",
                &[("name", &name), ("ext", &ext), ("dir", &dir.display())],
            )
        })
    };
    let raw = kfx_formats::rnc::load_file(&find("raw")?).map_err(|e| format!("{name}.raw: {e}"))?;
    // load_map_and_window refuses files under 1024 bytes and over the buffer
    // (src/front_landview.c:966-976)
    if raw.len() < 1024 {
        return Err(trf(
            "kfx_editor.name_raw_raw_bytes_too_small_for_a_land_view",
            "{name}.raw: {raw} bytes, too small for a land view",
            &[("name", &name), ("raw", &raw.len())],
        ));
    }
    let pal_bytes =
        kfx_formats::rnc::load_file(&find("pal")?).map_err(|e| format!("{name}.pal: {e}"))?;
    let pal = kfx_gfx::Palette::from_bytes(&pal_bytes).map_err(|e| format!("{name}.pal: {e}"))?;
    let table = pal.table();
    let mut img = kfx_gfx::Rgba::new(LAND_W, LAND_H);
    for (i, px) in img.data.chunks_exact_mut(4).enumerate() {
        let c = raw.get(i).map_or([0, 0, 0, 255], |&v| table[v as usize]);
        px.copy_from_slice(&c);
    }
    Ok(img)
}

impl EditorApp {
    /// Open (when `open`) and run the land view picker of the properties dialog.
    pub(super) fn land_picker_ui(&mut self, ctx: &egui::Context, open: bool) {
        if open {
            self.open_land_picker();
        }
        let Some(d) = self.props_dialog.as_mut() else {
            return;
        };
        if let Some(l) = d.land.as_mut() {
            if l.ui(ctx, &mut d.info) {
                d.land = None;
            }
        }
    }

    /// Open the picker over the properties dialog (for the level's own campaign when it has one).
    pub fn open_land_picker(&mut self) {
        let Some(install) = self.install().map(Path::to_path_buf) else {
            return;
        };
        let own = self
            .doc
            .as_ref()
            .and_then(|d| d.pack_cfg.as_ref())
            .filter(|(_, k)| *k == kfx_formats::campaign::PackKind::Campaign)
            .map(|(p, _)| p.clone())
            .or_else(|| self.props_dialog.as_ref().and_then(|d| d.target.clone()));
        if let Some(d) = self.props_dialog.as_mut() {
            d.land = Some(LandPicker::new(&install, own.as_deref(), &d.info));
        }
    }
}

#[cfg(test)]
impl EditorApp {
    pub fn props_info(&self) -> Option<&LevelInfo> {
        self.props_dialog.as_ref().map(|d| &d.info)
    }
    pub fn land_picker(&self) -> Option<&LandPicker> {
        self.props_dialog.as_ref().and_then(|d| d.land.as_ref())
    }
}

impl LandPicker {
    pub fn new(install: &Path, own_cfg: Option<&Path>, info: &LevelInfo) -> LandPicker {
        let campaigns = land_campaigns(install, own_cfg);
        let own = own_cfg.is_some_and(|o| campaigns.first().is_some_and(|c| c.cfg == o));
        // keeporig first among the others when the level has no campaign of its own
        let campaign = if own {
            0
        } else {
            campaigns
                .iter()
                .position(|c| {
                    c.cfg
                        .file_stem()
                        .is_some_and(|s| s.eq_ignore_ascii_case("keeporig"))
                })
                .unwrap_or(0)
        };
        let (view, window) = info.land_view.clone().unwrap_or_default();
        let mut p = LandPicker {
            campaigns,
            campaign,
            own,
            views: vec![],
            windows: vec![],
            view: view.to_ascii_lowercase(),
            window: window.to_ascii_lowercase(),
            pos: info
                .ensign_pos
                .unwrap_or(((ENSIGN_X.0 + ENSIGN_X.1) / 2, (ENSIGN_Y.0 + ENSIGN_Y.1) / 2)),
            zoom: info
                .ensign_zoom
                .or(info.ensign_pos)
                .unwrap_or(((ENSIGN_X.0 + ENSIGN_X.1) / 2, (ENSIGN_Y.0 + ENSIGN_Y.1) / 2)),
            zoom_follows: info.ensign_zoom.is_none() || info.ensign_zoom == info.ensign_pos,
            image: None,
        };
        p.load_campaign();
        p
    }

    fn load_campaign(&mut self) {
        let (v, w) = self
            .campaigns
            .get(self.campaign)
            .map(|c| land_files(&c.dir))
            .unwrap_or_default();
        self.views = v;
        self.windows = w;
        if !self.views.contains(&self.view) {
            self.view = self.views.first().cloned().unwrap_or_default();
        }
        if !self.windows.contains(&self.window) {
            self.window = self
                .matching_window()
                .unwrap_or_else(|| self.windows.first().cloned().unwrap_or_default());
        }
        self.image = None;
    }

    /// `viframeNN` for picture `rgmapNN` when it exists.
    fn matching_window(&self) -> Option<String> {
        let digits: String = self
            .view
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if digits.is_empty() {
            return None;
        }
        self.windows
            .iter()
            .find(|w| w.ends_with(&digits) && w.starts_with("viframe"))
            .cloned()
    }

    fn texture(&mut self, ctx: &egui::Context) -> Result<egui::TextureHandle, String> {
        if self.image.as_ref().is_none_or(|(n, _)| *n != self.view) {
            let r = match self.campaigns.get(self.campaign) {
                Some(c) => decode_land_view(&c.dir, &self.view).map(|img| {
                    ctx.load_texture(
                        format!("landview-{}", self.view),
                        img_to_color(&img),
                        egui::TextureOptions::LINEAR,
                    )
                }),
                None => Err(trd(
                    "kfx_editor.no_campaign_with_a_land_view_folder_in_this_inst",
                    "no campaign with a land view folder in this install",
                )),
            };
            self.image = Some((self.view.clone(), r));
        }
        self.image
            .as_ref()
            .map(|(_, r)| r.clone())
            .unwrap_or_else(|| Err(trd("kfx_editor.no_picture", "no picture")))
    }

    /// Set the ensign at picture pixel `p` (clamped to the allowed area); the zoom point follows
    /// when `zoom_follows`.
    pub fn set_pos(&mut self, p: (i32, i32)) {
        self.pos = (
            p.0.clamp(ENSIGN_X.0, ENSIGN_X.1),
            p.1.clamp(ENSIGN_Y.0, ENSIGN_Y.1),
        );
        if self.zoom_follows {
            self.zoom = self.pos;
        }
    }

    /// Write the choice into the level information.
    pub fn store(&self, info: &mut LevelInfo) {
        if !self.view.is_empty() {
            info.land_view = Some((self.view.clone(), self.window.clone()));
        }
        info.ensign_pos = Some(self.pos);
        info.ensign_zoom = Some(self.zoom);
    }

    /// The picker as a modal over the properties dialog. Returns true when it is done (OK or
    /// Cancel); OK writes into `info`.
    pub fn ui(&mut self, ctx: &egui::Context, info: &mut LevelInfo) -> bool {
        let mut done = false;
        egui::Modal::new(egui::Id::unique("land-view-picker")).show(ctx, |ui| {
            ui.set_width(860.0);
            ui.heading(
                RichText::new(trd(
                    "kfx_editor.land_view_and_ensign",
                    "Land view and ensign",
                ))
                .color(GOLD),
            );
            if self.campaigns.is_empty() {
                ui.colored_label(
                    Color32::LIGHT_RED,
                    trd(
                        "kfx_editor.no_campaign_of_this_install_has_a_land_location",
                        "No campaign of this install has a LAND_LOCATION folder.",
                    ),
                );
            } else {
                ui.horizontal(|ui| {
                    ui.label(trd("kfx_editor.pictures_of", "Pictures of"));
                    let mut c = self.campaign;
                    egui::ComboBox::from_id_salt("land-campaign")
                        .width(320.0)
                        .selected_text(self.campaigns[c].name.clone())
                        .show_ui(ui, |ui| {
                            for (i, k) in self.campaigns.iter().enumerate() {
                                ui.selectable_value(&mut c, i, &k.name);
                            }
                        });
                    if c != self.campaign {
                        self.campaign = c;
                        self.load_campaign();
                    }
                });
                if !self.own || self.campaign != 0 {
                    ui.label(
                        RichText::new(trd(
                            "kfx_editor.the_game_looks_for_the_land_view_in_the_land_loc",
                            "The game looks for the land view in the LAND_LOCATION of \
                            the level's own campaign: these files must be there too.",
                        ))
                        .small()
                        .color(Color32::from_rgb(230, 180, 90)),
                    );
                }
            }
            ui.label(
                RichText::new(trd(
                    "kfx_editor.click_the_picture_to_place_the_ensign_its_bottom",
                    "Click the picture to place the ensign (its bottom centre); the zoom \
                    point follows unless unticked, right-click sets the zoom point alone.",
                ))
                .small()
                .color(TEXT_DIM),
            );
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(150.0);
                    egui::ScrollArea::vertical()
                        .id_salt("land-views")
                        .max_height(480.0)
                        .show(ui, |ui| {
                            let mut pick = None;
                            for v in &self.views {
                                if ui.selectable_label(*v == self.view, v).clicked() {
                                    pick = Some(v.clone());
                                }
                            }
                            if let Some(v) = pick {
                                self.view = v;
                                if let Some(w) = self.matching_window() {
                                    self.window = w;
                                }
                            }
                        });
                });
                let (rect, resp) = ui.allocate_exact_size(
                    EVec2::new(LAND_W as f32 / 2.0, LAND_H as f32 / 2.0),
                    egui::Sense::click(),
                );
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::Image,
                        true,
                        trd("kfx_editor.land_view_picture", "land view picture"),
                    )
                });
                let p = ui.painter_at(rect);
                p.rect_filled(rect, 0.0, Color32::from_rgb(12, 11, 10));
                match self.texture(ui.ctx()) {
                    Ok(t) => {
                        p.image(
                            t.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    Err(e) => {
                        p.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            e,
                            egui::FontId::proportional(13.0),
                            Color32::LIGHT_RED,
                        );
                    }
                }
                let s = rect.width() / LAND_W as f32;
                let to_screen =
                    |q: (i32, i32)| rect.min + EVec2::new(q.0 as f32 * s, q.1 as f32 * s);
                let area = egui::Rect::from_min_max(
                    to_screen((ENSIGN_X.0, ENSIGN_Y.0)),
                    to_screen((ENSIGN_X.1, ENSIGN_Y.1)),
                );
                p.rect_stroke(
                    area,
                    0.0,
                    egui::Stroke::new(1.0, Color32::from_white_alpha(60)),
                    egui::StrokeKind::Inside,
                );
                // zoom point: a ring; ensign: a flag standing on its point
                let z = to_screen(self.zoom);
                p.circle_stroke(
                    z,
                    7.0,
                    egui::Stroke::new(1.5, Color32::from_rgb(90, 200, 255)),
                );
                let e = to_screen(self.pos);
                p.line_segment(
                    [e, e - EVec2::new(0.0, 26.0)],
                    egui::Stroke::new(2.0, Color32::from_rgb(240, 220, 160)),
                );
                p.add(egui::Shape::convex_polygon(
                    vec![
                        e - EVec2::new(0.0, 26.0),
                        e - EVec2::new(-16.0, 20.0),
                        e - EVec2::new(0.0, 14.0),
                    ],
                    Color32::from_rgb(210, 40, 40),
                    egui::Stroke::new(1.0, Color32::BLACK),
                ));
                p.circle_filled(e, 3.0, Color32::from_rgb(240, 220, 160));
                let to_pic = |q: egui::Pos2| {
                    (
                        ((q.x - rect.min.x) / s).round() as i32,
                        ((q.y - rect.min.y) / s).round() as i32,
                    )
                };
                if resp.clicked() {
                    if let Some(q) = resp.interact_pointer_pos() {
                        self.set_pos(to_pic(q));
                    }
                }
                if resp.secondary_clicked() {
                    if let Some(q) = resp.interact_pointer_pos() {
                        let (x, y) = to_pic(q);
                        self.zoom = (x.clamp(0, LAND_W as i32 - 1), y.clamp(0, LAND_H as i32 - 1));
                        self.zoom_follows = false;
                    }
                }
                if let Some(h) = resp.hover_pos() {
                    let (x, y) = to_pic(h);
                    resp.on_hover_text_at_pointer(format!("{x}, {y}"));
                }
            });
            egui::Grid::new("land-fields")
                .num_columns(2)
                .spacing([10.0, 4.0])
                .show(ui, |ui| {
                    ui.label(trd("kfx_editor.window_frame", "Window frame"));
                    egui::ComboBox::from_id_salt("land-window")
                        .width(160.0)
                        .selected_text(self.window.clone())
                        .show_ui(ui, |ui| {
                            for w in &self.windows {
                                ui.selectable_value(&mut self.window, w.clone(), w);
                            }
                        });
                    ui.end_row();
                    ui.label(trd("kfx_editor.ensign_position", "Ensign position"));
                    ui.horizontal(|ui| {
                        let (mut x, mut y) = self.pos;
                        let c = ui
                            .add(egui::DragValue::new(&mut x).range(ENSIGN_X.0..=ENSIGN_X.1))
                            .changed()
                            | ui.add(egui::DragValue::new(&mut y).range(ENSIGN_Y.0..=ENSIGN_Y.1))
                                .changed();
                        if c {
                            self.set_pos((x, y));
                        }
                    });
                    ui.end_row();
                    ui.label(trd("kfx_editor.ensign_zoom", "Ensign zoom"));
                    ui.horizontal(|ui| {
                        ui.add_enabled(
                            !self.zoom_follows,
                            egui::DragValue::new(&mut self.zoom.0).range(0..=LAND_W as i32 - 1),
                        );
                        ui.add_enabled(
                            !self.zoom_follows,
                            egui::DragValue::new(&mut self.zoom.1).range(0..=LAND_H as i32 - 1),
                        );
                        if ui
                            .checkbox(
                                &mut self.zoom_follows,
                                trd("kfx_editor.same_as_the_position", "same as the position"),
                            )
                            .changed()
                            && self.zoom_follows
                        {
                            self.zoom = self.pos;
                        }
                    });
                    ui.end_row();
                });
            ui.horizontal(|ui| {
                if ui
                    .button(trd("kfx_editor.use_these", "Use these"))
                    .clicked()
                {
                    self.store(info);
                    done = true;
                }
                if ui.button(trd("kfx_editor.cancel", "Cancel")).clicked() {
                    done = true;
                }
            });
        });
        done
    }
}
