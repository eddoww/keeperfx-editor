//! The slab palette: its tiles built from the atlas, and the palette panel (terrain, rooms,
//! custom slabs, filters).
use super::*;

impl EditorApp {
    pub(super) fn build_palette(&mut self, gd: &GameData, atlas: &AtlasData) {
        let Some(assets) = &self.assets else { return };
        let tile = |id: u16| -> Option<Vec<u8>> {
            let phys = *atlas.lookup.get(id as usize)?;
            let (layer, local) = ((phys / LAYER_TILES) as usize, phys % LAYER_TILES);
            let px = atlas.layers.get(layer)?.first()?;
            let (tx, ty) = ((local % TILES_PER_ROW) * 32, (local / TILES_PER_ROW) * 32);
            let mut out = vec![0u8; 32 * 32 * 4];
            for y in 0..32 {
                let o = (((ty + y) * LAYER_W + tx) * 4) as usize;
                out[(y * 32 * 4) as usize..((y + 1) * 32 * 4) as usize]
                    .copy_from_slice(&px[o..o + 32 * 4]);
            }
            Some(out)
        };
        // 3x3 top textures of the slab's ALL style
        let composite = |kind: usize| -> Option<kfx_gfx::Rgba> {
            let ids = assets::slab_preview_textures(gd, &self.cubes, kind);
            if ids.iter().all(Option::is_none) {
                return None;
            }
            let mut img = kfx_gfx::Rgba::new(96, 96);
            for (i, id) in ids.iter().enumerate() {
                let Some(px) = id.and_then(tile) else {
                    continue;
                };
                let (ox, oy) = ((i % 3) * 32, (i / 3) * 32);
                for y in 0..32 {
                    let o = ((oy + y) * 96 + ox) * 4;
                    img.data[o..o + 32 * 4].copy_from_slice(&px[y * 32 * 4..(y + 1) * 32 * 4]);
                }
            }
            Some(img)
        };
        let mut tiles = vec![];
        for k in 0..gd.slab_count() {
            if assets::is_door(gd, k) {
                continue;
            }
            let s = gd.slab(k);
            let room = (1..gd.rooms.len()).find(|&r| gd.rooms[r].assigned_slab as usize == k);
            let group = if room.is_some() {
                Group::Rooms
            } else if assets::is_room_wall(gd, k) {
                Group::RoomWalls
            } else if s.category == category::FORTIFIED_WALL {
                Group::Walls
            } else if assets::is_solid(gd, k) {
                Group::Natural
            } else if s.category == category::OBSTACLE
                || s.name.to_ascii_uppercase().starts_with("SLAB")
            {
                Group::Other
            } else {
                Group::Floors
            };
            let mut img = None;
            let (label, tooltip) = match room {
                Some(r) => {
                    let rc = &gd.rooms[r];
                    let sprite = rc
                        .raw
                        .get("SymbolSprites")
                        .and_then(|v| v.split_whitespace().next()?.parse::<u16>().ok())
                        .unwrap_or(0);
                    if sprite > 0 {
                        img = assets.panel_icon(sprite);
                    }
                    (
                        rc.name.clone(),
                        trf(
                            "kfx_editor.name_room_r_slab_k_name2",
                            "{name} (room {r}, slab {k} {name2})",
                            &[("name", &rc.name), ("r", &r), ("k", &k), ("name2", &s.name)],
                        ),
                    )
                }
                None => (
                    s.name.clone(),
                    trf(
                        "kfx_editor.name_slab_k",
                        "{name} (slab {k})",
                        &[("name", &s.name), ("k", &k)],
                    ),
                ),
            };
            let img = img.or_else(|| composite(k));
            tiles.push((k as u16, group, label, tooltip, img));
        }
        self.palette = tiles
            .into_iter()
            .map(|(kind, group, label, tooltip, img)| PaletteTile {
                kind,
                group,
                label,
                tooltip,
                tex: img.map(|i| {
                    self.ctx.load_texture(
                        "palette",
                        img_to_color(&i),
                        egui::TextureOptions::NEAREST,
                    )
                }),
            })
            .collect();
    }

    pub(super) fn palette_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(300.0);
                egui::Grid::new("categories")
                    .num_columns(2)
                    .spacing([4.0, 4.0])
                    .show(ui, |ui| {
                        for (i, (cat, label)) in CATEGORIES.iter().enumerate() {
                            let b = egui::Button::selectable(self.category == *cat, *label)
                                .min_size(EVec2::new(144.0, 26.0));
                            let r = ui.add(b);
                            if r.clicked() {
                                self.category = *cat;
                                if !matches!(
                                    cat,
                                    Category::Terrain | Category::Rooms | Category::Custom
                                ) && self.tool != Tool::Place
                                    && self.place.is_some()
                                {
                                    self.set_tool(Tool::Place);
                                }
                            }
                            if i % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("🔍");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.palette_search)
                            .hint_text(trd("kfx_editor.search_e1e9", "search"))
                            .desired_width(240.0),
                    );
                });
            });
            ui.separator();
            let groups: Vec<(Group, Tx)> = match self.category {
                Category::Terrain => TERRAIN_GROUPS.to_vec(),
                Category::Rooms => vec![(Group::Rooms, tx("kfx_editor.rooms_108b", "Rooms"))],
                Category::Custom => {
                    self.custom_palette_ui(ui);
                    return;
                }
                _ => {
                    self.thing_palette_ui(ui);
                    return;
                }
            };
            if self.palette.is_empty() {
                ui.label(
                    RichText::new(trd(
                        "kfx_editor.open_a_map_to_see_its_palette",
                        "Open a map to see its palette",
                    ))
                    .color(TEXT_DIM),
                );
                return;
            }
            let q = self.palette_search.trim().to_ascii_lowercase();
            let mut chosen = None;
            let l = &mut self.settings.layout;
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(trd("kfx_editor.tiles", "Tiles"))
                            .small()
                            .color(GOLD_DIM),
                    );
                    ui.add(egui::Slider::new(&mut l.palette_tile, 32.0..=96.0).show_value(false))
                        .on_hover_text(trd("kfx_editor.tile_size", "Tile size"));
                    ui.checkbox(&mut l.palette_ids, trd("kfx_editor.ids", "IDs"))
                        .on_hover_text(trd(
                            "kfx_editor.show_the_slab_kind_id_on_each_tile",
                            "Show the slab kind ID on each tile",
                        ));
                    ui.checkbox(&mut l.palette_by_id, trd("kfx_editor.by_id", "By ID"))
                        .on_hover_text(trd(
                            "kfx_editor.one_list_in_slab_id_order_instead_of_the_groups",
                            "One list in slab ID order instead of the groups",
                        ));
                });
                let (tile, show_ids, by_id) = (
                    l.palette_tile.clamp(32.0, 96.0),
                    l.palette_ids,
                    l.palette_by_id,
                );
                let groups: Vec<(Option<Group>, Tx)> = if by_id {
                    vec![(None, tx("kfx_editor.all_by_slab_id", "All, by slab ID"))]
                } else {
                    groups.into_iter().map(|(g, t)| (Some(g), t)).collect()
                };
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            for (g, title) in groups {
                                let mut tiles: Vec<&PaletteTile> = self
                                    .palette
                                    .iter()
                                    .filter(|t| {
                                        g.is_none_or(|g| t.group == g)
                                            && (q.is_empty()
                                                || t.label.to_ascii_lowercase().contains(&q)
                                                || t.tooltip.to_ascii_lowercase().contains(&q))
                                    })
                                    .collect();
                                if by_id {
                                    tiles.sort_by_key(|t| t.kind);
                                }
                                if tiles.is_empty() {
                                    continue;
                                }
                                ui.label(RichText::new(title).small().color(GOLD_DIM));
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = EVec2::new(4.0, 4.0);
                                    for t in tiles {
                                        let sel = self.brush == Some(t.kind);
                                        let (rect, resp) = ui.allocate_exact_size(
                                            EVec2::splat(tile),
                                            egui::Sense::click(),
                                        );
                                        let painter = ui.painter_at(rect.expand(2.0));
                                        painter.rect_filled(
                                            rect,
                                            2.0,
                                            Color32::from_rgb(12, 11, 10),
                                        );
                                        match &t.tex {
                                            Some(tex) => {
                                                let inner = rect.shrink(3.0);
                                                let ts = tex.size_vec2();
                                                let scale = (inner.width() / ts.x)
                                                    .min(inner.height() / ts.y);
                                                let r = egui::Rect::from_center_size(
                                                    inner.center(),
                                                    ts * scale,
                                                );
                                                painter.image(
                                                    tex.id(),
                                                    r,
                                                    egui::Rect::from_min_max(
                                                        egui::pos2(0.0, 0.0),
                                                        egui::pos2(1.0, 1.0),
                                                    ),
                                                    Color32::WHITE,
                                                );
                                            }
                                            None => {
                                                painter.text(
                                                    rect.center(),
                                                    egui::Align2::CENTER_CENTER,
                                                    &t.label,
                                                    egui::FontId::proportional(9.0),
                                                    theme::TEXT,
                                                );
                                            }
                                        }
                                        let stroke = if sel {
                                            egui::Stroke::new(2.0, Color32::from_rgb(200, 50, 60))
                                        } else if resp.hovered() {
                                            egui::Stroke::new(1.0, GOLD)
                                        } else {
                                            egui::Stroke::new(1.0, Color32::from_rgb(60, 52, 44))
                                        };
                                        painter.rect_stroke(
                                            rect,
                                            2.0,
                                            stroke,
                                            egui::StrokeKind::Outside,
                                        );
                                        if show_ids {
                                            let at = rect.left_top() + EVec2::new(2.0, 1.0);
                                            let g = painter.layout_no_wrap(
                                                t.kind.to_string(),
                                                egui::FontId::monospace(10.0),
                                                Color32::WHITE,
                                            );
                                            painter.rect_filled(
                                                egui::Rect::from_min_size(
                                                    at,
                                                    g.size() + EVec2::new(3.0, 0.0),
                                                ),
                                                2.0,
                                                Color32::from_black_alpha(170),
                                            );
                                            painter.galley(
                                                at + EVec2::new(1.5, 0.0),
                                                g,
                                                Color32::WHITE,
                                            );
                                        }
                                        resp.widget_info(|| {
                                            egui::WidgetInfo::selected(
                                                egui::Role::Button,
                                                true,
                                                sel,
                                                &t.label,
                                            )
                                        });
                                        if resp
                                            .on_hover_text(trf(
                                                "kfx_editor.tooltip_id_kind",
                                                "{tooltip} (ID {kind})",
                                                &[("tooltip", &t.tooltip), ("kind", &t.kind)],
                                            ))
                                            .clicked()
                                        {
                                            chosen = Some(t.kind);
                                        }
                                    }
                                });
                            }
                        });
                    });
            });
            if let Some(k) = chosen {
                self.brush = Some(k);
                self.custom_brush = None;
                self.style_brush = None;
                if matches!(
                    self.tool,
                    Tool::Eyedropper | Tool::Select | Tool::Place | Tool::Things
                ) {
                    self.set_tool(Tool::Paint);
                }
            }
        });
    }
}
