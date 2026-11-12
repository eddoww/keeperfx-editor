//! The toolbar, the players panel, the History panel and the status bar.
use super::*;

impl EditorApp {
    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui) {
        // a labelled group; it goes to the next row when its width (from the last frame) does
        // not fit, so nothing is cut off at the window's edge
        let mut widths = std::mem::take(&mut self.toolbar_widths);
        // narrow windows (or a large UI scale): icons only, no group titles, the options in menus
        let compact = ui.available_width() < 1150.0;
        let t = |s: String| -> String {
            if compact && s.chars().next().is_some_and(|c| !c.is_ascii()) {
                s.split(' ').next().unwrap_or(&s).to_string()
            } else {
                s
            }
        };
        let mut group = |ui: &mut egui::Ui,
                         title: &'static str,
                         label: String,
                         add: &mut dyn FnMut(&mut egui::Ui)| {
            if let Some(w) = widths.get(title) {
                let left = ui.max_rect().max.x - ui.cursor().min.x;
                if *w > left && ui.cursor().min.x > ui.max_rect().min.x + 1.0 {
                    ui.end_row();
                }
            }
            let r = ui.vertical(|ui| {
                if !compact {
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);
                        ui.label(RichText::new(label).small().color(GOLD));
                    });
                }
                ui.horizontal(|ui| add(ui));
            });
            widths.insert(title, r.response.rect.width() + 12.0);
            ui.separator();
        };
        let has_data = self.assets.is_some();
        let has_doc = self.doc.is_some();
        let (can_undo, can_redo) = self
            .doc
            .as_ref()
            .map(|d| (d.undo_name().is_some(), d.redo_name().is_some()))
            .unwrap_or((false, false));
        let sel = self.selection.is_some();
        let clip = self.clipboard.is_some();
        let mut act: Vec<&'static str> = vec![];
        let mut tool = self.tool;
        let k = self.keymap.clone();
        let sc = |id: &str| k.label(id);
        let tip = |what: &str, id: &str| {
            let key = k.label(id);
            if key.is_empty() {
                what.to_string()
            } else {
                format!("{what} ({key})")
            }
        };
        let view_mode = self.settings.layout.view_mode;
        let fly = self.camera.fly;
        let names = panels::texture_pack_names();
        // one row of labelled groups that wraps on narrow windows (DK2 editor style)
        ui.horizontal_wrapped(|ui| {
            group(ui, "File", trd("kfx_editor.file", "File"), &mut |ui| {
                if ui
                    .add_enabled(
                        has_data,
                        egui::Button::new(t(trd("kfx_editor.new", "🗋 New"))),
                    )
                    .on_hover_text(tip(&trd("kfx_editor.new_map", "New map"), "file.new"))
                    .on_disabled_hover_text(trd(
                        "kfx_editor.choose_the_keeperfx_folder_first",
                        "Choose the KeeperFX folder first",
                    ))
                    .clicked()
                {
                    act.push("new");
                }
                if ui
                    .add_enabled(
                        has_data,
                        egui::Button::new(t(trd("kfx_editor.open_5644", "🗁 Open"))),
                    )
                    .on_hover_text(tip(
                        &trd("kfx_editor.open_a_map", "Open a map"),
                        "file.open",
                    ))
                    .on_disabled_hover_text(trd(
                        "kfx_editor.choose_the_keeperfx_folder_first",
                        "Choose the KeeperFX folder first",
                    ))
                    .clicked()
                {
                    act.push("open");
                }
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new(t(trd("kfx_editor.save", "💾 Save"))),
                    )
                    .on_hover_text(tip(
                        &trd(
                            "kfx_editor.save_the_map_and_its_scripts",
                            "Save the map and its scripts",
                        ),
                        "file.save",
                    ))
                    .clicked()
                {
                    act.push("save");
                }
            });
            group(ui, "Edit", trd("kfx_editor.edit", "Edit"), &mut |ui| {
                if ui
                    .add_enabled(
                        can_undo,
                        egui::Button::new(t(trd("kfx_editor.undo", "⟲ Undo"))),
                    )
                    .on_hover_text(tip(&trd("kfx_editor.undo_530f", "Undo"), "edit.undo"))
                    .clicked()
                {
                    act.push("undo");
                }
                if ui
                    .add_enabled(
                        can_redo,
                        egui::Button::new(t(trd("kfx_editor.redo", "⟳ Redo"))),
                    )
                    .on_hover_text(tip(&trd("kfx_editor.redo_0149", "Redo"), "edit.redo"))
                    .clicked()
                {
                    act.push("redo");
                }
                if ui
                    .add_enabled(
                        sel,
                        egui::Button::new(t(trd("kfx_editor.cut_0d07", "✂ Cut"))),
                    )
                    .on_hover_text(trd(
                        "kfx_editor.cut_the_selected_slabs_ctrl_x",
                        "Cut the selected slabs (Ctrl+X)",
                    ))
                    .clicked()
                {
                    act.push("cut");
                }
                if ui
                    .add_enabled(
                        sel,
                        egui::Button::new(t(trd("kfx_editor.copy_7ffc", "🗐 Copy"))),
                    )
                    .on_hover_text(trd(
                        "kfx_editor.copy_the_selected_slabs_and_their_things_ctrl_c",
                        "Copy the selected slabs and their things (Ctrl+C)",
                    ))
                    .clicked()
                {
                    act.push("copy");
                }
                if ui
                    .add_enabled(
                        clip && has_doc,
                        egui::Button::new(t(trd("kfx_editor.paste_4d95", "📋 Paste"))),
                    )
                    .on_hover_text(trd(
                        "kfx_editor.paste_click_on_the_map_to_stamp_ctrl_v",
                        "Paste: click on the map to stamp (Ctrl+V)",
                    ))
                    .clicked()
                {
                    act.push("paste");
                }
            });
            // the zoom of the 2D camera in the 2D view
            let cam = if view_mode == crate::settings::ViewMode::TwoD {
                &mut self.camera2
            } else {
                &mut self.camera
            };
            group(ui, "Zoom", trd("kfx_editor.zoom", "Zoom"), &mut |ui| {
                let mut pct = cam.zoom * 100.0;
                let r = ui
                    .add_enabled(
                        !fly && has_doc,
                        egui::DragValue::new(&mut pct)
                            .range(MIN_ZOOM * 100.0..=MAX_ZOOM * 100.0)
                            .suffix("%")
                            .speed(1.0)
                            .max_decimals(0),
                    )
                    .on_hover_text(trd(
                        "kfx_editor.zoom_mouse_wheel_over_the_map",
                        "Zoom (mouse wheel over the map)",
                    ));
                if r.changed() {
                    cam.zoom = (pct / 100.0).clamp(MIN_ZOOM, MAX_ZOOM);
                }
                ui.menu_button(trd("kfx_editor.presets", "Presets"), |ui| {
                    for p in [25.0, 50.0, 100.0, 200.0, 400.0] {
                        if ui.button(format!("{p}%")).clicked() {
                            cam.zoom = p / 100.0;
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text(trd("kfx_editor.zoom_presets", "Zoom presets"));
                if ui
                    .button(trd("kfx_editor.fit", "Fit"))
                    .on_hover_text(trf(
                        "kfx_editor.fit_the_whole_map_sc",
                        "Fit the whole map ({key})",
                        &[("key", &sc("cam.home"))],
                    ))
                    .clicked()
                {
                    act.push("fit");
                }
            });
            group(ui, "Draw", trd("kfx_editor.draw", "Draw"), &mut |ui| {
                for (tl, label, what) in [
                    (
                        Tool::Paint,
                        trd("kfx_editor.paint", "✏ Paint"),
                        trd(
                            "kfx_editor.paint_slabs_with_the_brush",
                            "Paint slabs with the brush",
                        ),
                    ),
                    (
                        Tool::Line,
                        trd("kfx_editor.line", "📏 Line"),
                        trd("kfx_editor.draw_a_line_of_slabs", "Draw a line of slabs"),
                    ),
                    (
                        Tool::Box,
                        trd("kfx_editor.box", "☐ Box"),
                        trd(
                            "kfx_editor.draw_a_rectangle_filled_or_not",
                            "Draw a rectangle (filled or not)",
                        ),
                    ),
                    (
                        Tool::Circle,
                        trd("kfx_editor.circle", "○ Circle"),
                        trd(
                            "kfx_editor.draw_a_circle_filled_or_not",
                            "Draw a circle (filled or not)",
                        ),
                    ),
                    (
                        Tool::Fill,
                        trd("kfx_editor.fill_a33f", "🎨 Fill"),
                        trd(
                            "kfx_editor.flood_fill_an_area_of_the_same_slabs",
                            "Flood fill an area of the same slabs",
                        ),
                    ),
                    (
                        Tool::Eyedropper,
                        trd("kfx_editor.pick", "💧 Pick"),
                        trd(
                            "kfx_editor.pick_the_slab_kind_and_owner_under_the_pointer",
                            "Pick the slab kind and owner under the pointer",
                        ),
                    ),
                    (
                        Tool::Select,
                        trd("kfx_editor.select", "🔲 Select"),
                        trd(
                            "kfx_editor.select_a_rectangle_of_slabs_copy_cut_fill_lock",
                            "Select a rectangle of slabs (copy, cut, fill, lock)",
                        ),
                    ),
                ] {
                    if ui
                        .add_enabled(has_doc, egui::Button::selectable(tool == tl, t(label)))
                        .on_hover_text(what)
                        .clicked()
                    {
                        tool = tl;
                    }
                }
            });
            group(
                ui,
                "Place",
                trd("kfx_editor.place_2d1c", "Place"),
                &mut |ui| {
                    if ui
                        .add_enabled(
                            has_doc,
                            egui::Button::selectable(
                                tool == Tool::Place,
                                t(trd("kfx_editor.place", "⊞ Place")),
                            ),
                        )
                        .on_hover_text(trd(
                            "kfx_editor.place_the_thing_chosen_on_the_palette_creatures",
                            "Place the thing chosen on the palette (creatures, objects, \
                            traps, doors, effects, lights, action points)",
                        ))
                        .clicked()
                    {
                        tool = Tool::Place;
                    }
                    if ui
                        .add_enabled(
                            has_doc,
                            egui::Button::selectable(
                                tool == Tool::Things,
                                t(trd("kfx_editor.things_4b5c", "👆 Things")),
                            ),
                        )
                        .on_hover_text(trd(
                            "kfx_editor.select_move_turn_and_delete_things",
                            "Select, move, turn and delete things",
                        ))
                        .clicked()
                    {
                        tool = Tool::Things;
                    }
                },
            );
            group(ui, "Brush", trd("kfx_editor.brush", "Brush"), &mut |ui| {
                ui.add(
                    egui::DragValue::new(&mut self.brush_size)
                        .range(1..=tools::MAX_BRUSH)
                        .suffix(trd("kfx_editor.slabs", " slabs"))
                        .speed(0.1),
                )
                .on_hover_text(trd("kfx_editor.brush_size_and", "Brush size ([ and ])"));
                ui.selectable_value(&mut self.brush_round, false, "■")
                    .on_hover_text(trd("kfx_editor.square_brush", "Square brush"));
                ui.selectable_value(&mut self.brush_round, true, "⚫")
                    .on_hover_text(trd("kfx_editor.round_brush", "Round brush"));
            });
            let sym_on = self.symmetry.is_some();
            group(ui, "Map", trd("kfx_editor.map", "Map"), &mut |ui| {
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::selectable(
                            sym_on,
                            t(trd("kfx_editor.symmetry_fbf1", "⚖ Symmetry")),
                        ),
                    )
                    .on_hover_text(trd(
                        "kfx_editor.symmetric_painting_and_placement",
                        "Symmetric painting and placement",
                    ))
                    .clicked()
                {
                    act.push("symmetry");
                }
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new(t(trd("kfx_editor.validate_f389", "✔ Validate"))),
                    )
                    .on_hover_text(trd(
                        "kfx_editor.check_the_map_and_its_scripts",
                        "Check the map and its scripts",
                    ))
                    .clicked()
                {
                    act.push("validate");
                }
            });
            group(ui, "View", trd("kfx_editor.view", "View"), &mut |ui| {
                use crate::settings::ViewMode as V;
                for (m, label, what, id) in [
                    (
                        V::ThreeD,
                        trd("kfx_editor.3d", "3D"),
                        trd("kfx_editor.the_game_s_view", "The game's view"),
                        "",
                    ),
                    (
                        V::TwoD,
                        trd("kfx_editor.2d", "2D"),
                        trd("kfx_editor.top_down_view", "Top-down view"),
                        "view.2d",
                    ),
                    (
                        V::Split,
                        trd("kfx_editor.split", "Split"),
                        trd(
                            "kfx_editor.2d_and_3d_side_by_side",
                            "2D and 3D side by side",
                        ),
                        "view.split",
                    ),
                ] {
                    if ui
                        .add_enabled(
                            has_doc,
                            egui::Button::selectable(
                                view_mode == m && !(fly && m == V::ThreeD),
                                label,
                            ),
                        )
                        .on_hover_text(tip(&what, id))
                        .clicked()
                    {
                        act.push(match m {
                            V::ThreeD => "view3d",
                            V::TwoD => "view2d",
                            V::Split => "viewsplit",
                        });
                    }
                }
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::selectable(fly, t(trd("kfx_editor.fly", "✈ Fly"))),
                    )
                    .on_hover_text(tip(
                        &trd(
                            "kfx_editor.first_person_fly_view_wasd_q_e_down_up_shift_fas",
                            "First-person fly view: WASD, Q/E down/up, Shift faster, M \
                            mouse-look",
                        ),
                        "view.fly",
                    ))
                    .clicked()
                {
                    act.push("fly");
                }
            });
            group(
                ui,
                "Options",
                trd("kfx_editor.options", "Options"),
                &mut |ui| {
                    if compact {
                        ui.menu_button("⚙", |ui| {
                            ui.checkbox(
                                &mut self.shape_filled,
                                trd("kfx_editor.filled_shapes", "Filled shapes"),
                            );
                            ui.checkbox(
                                &mut self.owner_only,
                                trd("kfx_editor.ownership_only", "Ownership only"),
                            );
                            ui.checkbox(&mut self.fortify, trd("kfx_editor.fortify", "Fortify"));
                            ui.checkbox(
                                &mut self.editable_border,
                                trd("kfx_editor.editable_border", "Editable border"),
                            );
                            let mut style_on = self.style_brush.is_some();
                            if ui
                                .checkbox(
                                    &mut style_on,
                                    trd("kfx_editor.style_slx_brush", "Style (.slx) brush"),
                                )
                                .changed()
                            {
                                self.style_brush = style_on.then_some(1);
                            }
                        })
                        .response
                        .on_hover_text(trd(
                            "kfx_editor.tool_options_filled_shapes_ownership_only_fortif",
                            "Tool options: filled shapes, ownership only, fortify, \
                            editable border, style brush",
                        ));
                        if self.style_brush.is_none() {
                            return;
                        }
                    }
                    if !compact {
                        ui.checkbox(&mut self.shape_filled, trd("kfx_editor.filled", "Filled"))
                            .on_hover_text(trd(
                                "kfx_editor.box_and_circle_draw_filled_shapes",
                                "Box and circle draw filled shapes",
                            ));
                        ui.checkbox(
                            &mut self.owner_only,
                            trd("kfx_editor.ownership_only", "Ownership only"),
                        )
                        .on_hover_text(trd(
                            "kfx_editor.paint_the_active_owner_without_changing_slab_kin",
                            "Paint the active owner without changing slab kinds",
                        ));
                        ui.checkbox(&mut self.fortify, trd("kfx_editor.fortify", "Fortify"))
                            .on_hover_text(trd(
                                "kfx_editor.placing_a_keeper_s_ground_claimed_rooms_turns_th",
                                "Placing a keeper's ground (claimed, rooms) turns the earth \
                                around it into that keeper's walls (art: Preferences > Placement)",
                            ));
                        ui.checkbox(
                            &mut self.editable_border,
                            trd("kfx_editor.border", "Border"),
                        )
                        .on_hover_text(trd(
                            "kfx_editor.allow_editing_the_outermost_ring_of_slabs",
                            "Allow editing the outermost ring of slabs",
                        ));
                        let mut style_on = self.style_brush.is_some();
                        if ui
                            .checkbox(&mut style_on, trd("kfx_editor.style", "Style"))
                            .on_hover_text(trd(
                                "kfx_editor.paint_the_slab_texture_style_slx_instead_of_slab",
                                "Paint the slab texture style (.slx) instead of slabs",
                            ))
                            .changed()
                        {
                            self.style_brush = style_on.then_some(1);
                        }
                    }
                    if let Some(v) = self.style_brush.as_mut() {
                        egui::ComboBox::from_id_salt("style")
                            .width(120.0)
                            .selected_text(if *v == 0 {
                                trd("kfx_editor.level_default", "level default")
                            } else {
                                panels::texture_set_name(&names, *v as u32 - 1)
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    v,
                                    0,
                                    trd("kfx_editor.0_level_default", "0: level default"),
                                );
                                for s in 1..=names.iter().map(|x| x.0).max().unwrap_or(14) as u8 {
                                    ui.selectable_value(
                                        v,
                                        s,
                                        format!(
                                            "{s}: {}",
                                            panels::texture_set_name(&names, s as u32 - 1)
                                        ),
                                    );
                                }
                            });
                    }
                },
            );
            let l = &mut self.settings.layout;
            group(ui, "Show", trd("kfx_editor.show", "Show"), &mut |ui| {
                if compact {
                    ui.menu_button("👁", |ui| {
                        ui.checkbox(
                            &mut l.show_ownership,
                            trd("kfx_editor.ownership_tint", "Ownership tint"),
                        );
                        ui.checkbox(&mut l.show_grid, trd("kfx_editor.slab_grid", "Slab grid"));
                        ui.checkbox(&mut l.lighting, trd("kfx_editor.lighting", "Lighting"));
                    })
                    .response
                    .on_hover_text(trd(
                        "kfx_editor.ownership_tint_grid_lighting",
                        "Ownership tint, grid, lighting",
                    ));
                    return;
                }
                ui.checkbox(&mut l.show_ownership, trd("kfx_editor.owners", "Owners"))
                    .on_hover_text(trd(
                        "kfx_editor.tint_slabs_with_their_owner_s_colour",
                        "Tint slabs with their owner's colour",
                    ));
                ui.checkbox(&mut l.show_grid, trd("kfx_editor.grid", "Grid"))
                    .on_hover_text(trd("kfx_editor.slab_grid", "Slab grid"));
                ui.checkbox(&mut l.lighting, trd("kfx_editor.light", "Light"))
                    .on_hover_text(trd("kfx_editor.the_map_s_lighting", "The map's lighting"));
                ui.add_enabled(
                    has_doc,
                    egui::Checkbox::new(
                        &mut l.analysis.open,
                        trd("kfx_editor.analysis", "Analysis"),
                    ),
                )
                .on_hover_text(tip(
                    &trd(
                        "kfx_editor.analysis_panel_layers_routes_fairness",
                        "Analysis panel: layers, routes, fairness",
                    ),
                    "view.analysis",
                ));
            });
            group(ui, "Level", trd("kfx_editor.level", "Level"), &mut |ui| {
                if ui
                    .add_enabled(
                        has_data,
                        egui::Button::new(t(trd("kfx_editor.maps", "🗺 Maps"))),
                    )
                    .on_hover_text(tip(
                        &trd("kfx_editor.map_browser", "Map browser"),
                        "file.browser",
                    ))
                    .clicked()
                {
                    act.push("browse");
                }
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new(t(trd("kfx_editor.scripts_5763", "📜 Scripts"))),
                    )
                    .on_hover_text(trf(
                        "kfx_editor.script_panel_sc_sc2_maximises_it",
                        "Script panel ({key}); {key2} maximises it",
                        &[
                            ("key", &sc("view.script")),
                            ("key2", &sc("view.script_max")),
                        ],
                    ))
                    .clicked()
                {
                    act.push("scripts");
                }
                let r = ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new(t(trd("kfx_editor.play", "▶ Play"))),
                    )
                    .on_hover_text(trf(
                        "kfx_editor.play_the_level_in_keeperfx_sc_sc2_saves_first_ri",
                        "Play the level in KeeperFX ({key}; {key2} saves first; \
                        right-click: record / replay)",
                        &[("key", &sc("play.play")), ("key2", &sc("play.save_play"))],
                    ));
                if r.clicked() {
                    act.push("play");
                }
                r.context_menu(|ui| {
                    if ui
                        .button(trd(
                            "kfx_editor.play_and_record_packet_save",
                            "Play and record (packet save)",
                        ))
                        .clicked()
                    {
                        act.push("play_record");
                        ui.close();
                    }
                    if ui
                        .button(trd(
                            "kfx_editor.replay_the_last_recording_packet_load",
                            "Replay the last recording (packet load)",
                        ))
                        .clicked()
                    {
                        act.push("play_replay");
                        ui.close();
                    }
                });
            });
        });
        self.toolbar_widths = widths;
        if tool != self.tool {
            self.set_tool(tool);
        }
        for a in act {
            match a {
                "new" => self.request(Pending::New),
                "open" => self.open_dialog(),
                "save" => self.do_save(),
                "undo" => self.undo(),
                "redo" => self.redo(),
                "cut" => self.cut(),
                "copy" => {
                    self.copy();
                }
                "paste" => self.paste(),
                "fit" => {
                    if self.settings.layout.view_mode == crate::settings::ViewMode::TwoD {
                        self.camera2_home = true;
                    } else {
                        self.camera_pending_home = true;
                    }
                }
                "symmetry" => self.show_symmetry = true,
                "validate" => {
                    self.revalidate();
                    self.settings.layout.show_history = true;
                    self.settings.layout.right_tab = "validate".into();
                }
                "browse" => self.browser.open = true,
                "scripts" => self.toggle_script_panel(),
                "play" => self.play(false),
                "play_record" => self.play_packet(false),
                "view3d" => {
                    if self.camera.fly {
                        self.toggle_fly();
                    }
                    self.set_view_mode(crate::settings::ViewMode::ThreeD);
                }
                "view2d" => self.set_view_mode(crate::settings::ViewMode::TwoD),
                "viewsplit" => self.set_view_mode(crate::settings::ViewMode::Split),
                "fly" => self.toggle_fly(),
                "play_replay" => self.play_packet(true),
                _ => {}
            }
        }
    }

    pub(super) fn players_panel(&mut self, ui: &mut egui::Ui) {
        if !self.settings.layout.tabs_left {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(trd("kfx_editor.tree", "Tree"))
                        .color(GOLD)
                        .strong(),
                );
            });
            ui.separator();
        }
        let Some(doc) = &self.doc else {
            ui.label(RichText::new(trd("kfx_editor.no_map_open", "No map open")).color(TEXT_DIM));
            return;
        };
        let counts = doc.owner_counts();
        let d = doc.dims();
        let title = format!(
            "map{:05} ({}x{}){}",
            doc.doc.level,
            d.w,
            d.h,
            if doc.is_dirty() { " *" } else { "" }
        );
        let mut chosen = None;
        egui::CollapsingHeader::new(RichText::new(title).strong())
            .default_open(true)
            .show(ui, |ui| {
                for &p in &PLAYER_TREE_ORDER {
                    let (name, rgb) = PLAYERS[p as usize];
                    let selected = self.active_owner == p;
                    let (rect, resp) = ui.allocate_exact_size(
                        EVec2::new(ui.available_width(), 24.0),
                        egui::Sense::click(),
                    );
                    let painter = ui.painter_at(rect);
                    if selected {
                        painter.rect(
                            rect,
                            3.0,
                            Color32::from_rgb(70, 30, 26),
                            egui::Stroke::new(1.0, Color32::from_rgb(170, 50, 50)),
                            egui::StrokeKind::Inside,
                        );
                    } else if resp.hovered() {
                        painter.rect_filled(rect, 3.0, Color32::from_rgb(40, 35, 30));
                    }
                    let c = rect.left_center() + EVec2::new(16.0, 0.0);
                    painter.circle(
                        c,
                        8.0,
                        Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
                        egui::Stroke::new(1.0, Color32::from_gray(200)),
                    );
                    if p == 5 {
                        painter.line_segment(
                            [c + EVec2::new(-5.0, 5.0), c + EVec2::new(5.0, -5.0)],
                            egui::Stroke::new(1.5, Color32::from_gray(220)),
                        );
                    }
                    painter.text(
                        rect.left_center() + EVec2::new(32.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        name,
                        egui::FontId::proportional(13.0),
                        theme::TEXT,
                    );
                    painter.text(
                        rect.right_center() - EVec2::new(6.0, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        counts[p as usize].to_string(),
                        egui::FontId::proportional(11.0),
                        TEXT_DIM,
                    );
                    resp.widget_info(|| {
                        egui::WidgetInfo::selected(egui::Role::Button, true, selected, name)
                    });
                    let resp = resp.on_hover_text(trf(
                        "kfx_editor.player_p_p2_slabs_owned_click_to_make_it_the_act",
                        "{p} in level scripts. {p2} slabs owned. Click to make it the active owner.",
                        &[
                            ("p", &kfx_map::validate::owner_script_name(p)),
                            ("p2", &counts[p as usize]),
                        ],
                    ));
                    if resp.clicked() {
                        chosen = Some(p);
                    }
                }
            });
        if let Some(p) = chosen {
            self.active_owner = p;
        }
        ui.label(
            RichText::new(trf(
                "kfx_editor.active_owner_active_owner",
                "Active owner: {active_owner}",
                &[("active_owner", &PLAYERS[self.active_owner as usize].0)],
            ))
            .small()
            .color(TEXT_DIM),
        );
        if self.settings.layout.show_minimap {
            ui.add_space(6.0);
            ui.label(
                RichText::new(trd("kfx_editor.minimap", "Minimap"))
                    .color(GOLD)
                    .strong(),
            );
            if self.minimap_dirty || self.minimap_tex.is_none() {
                if let Some(doc) = &self.doc {
                    let img = minimap::image(&doc.doc, &doc.gd);
                    match &mut self.minimap_tex {
                        Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                        None => {
                            self.minimap_tex = Some(self.ctx.load_texture(
                                "minimap",
                                img,
                                egui::TextureOptions::NEAREST,
                            ))
                        }
                    }
                }
                self.minimap_dirty = false;
            }
            if let Some(tex) = &self.minimap_tex {
                // the outline of each view on screen: the 3D camera, the 2D one, or both
                use crate::settings::ViewMode;
                let mode = self.settings.layout.view_mode;
                let mut views = vec![];
                if mode != ViewMode::TwoD {
                    views.push((&self.camera, self.last_vp, minimap::COLOR_3D));
                }
                if mode != ViewMode::ThreeD {
                    views.push((&self.camera2, self.last_vp2, minimap::COLOR_2D));
                }
                if let Some(p) = minimap::show(ui, tex, d.w as usize, d.h as usize, &views) {
                    if mode != ViewMode::TwoD {
                        self.camera.target = Vec3::new(p.x, self.camera.target.y, p.y);
                    }
                    if mode != ViewMode::ThreeD {
                        self.camera2.target = Vec3::new(p.x, self.camera2.target.y, p.y);
                    }
                }
            }
        }
    }

    pub(super) fn history_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new(trd("kfx_editor.history", "History"))
                .color(GOLD)
                .strong(),
        );
        ui.separator();
        let Some(doc) = &self.doc else {
            ui.label(RichText::new(trd("kfx_editor.no_map_open", "No map open")).color(TEXT_DIM));
            return;
        };
        let cur = doc.cursor();
        let mut jump = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if ui
                    .add(egui::Button::selectable(
                        cur == 0,
                        trd("kfx_editor.opened", "(opened)"),
                    ))
                    .clicked()
                {
                    jump = Some(0);
                }
                for (i, c) in doc.history().iter().enumerate() {
                    let text = format!("{}. {}", i + 1, c.name);
                    let rt = if i < cur {
                        RichText::new(text)
                    } else {
                        RichText::new(text).color(TEXT_DIM).italics()
                    };
                    let r = ui
                        .add(egui::Button::selectable(cur == i + 1, rt))
                        .on_hover_text(trf(
                            "kfx_editor.c_changes_c2_kib",
                            "{c} changes, ~{c2} KiB",
                            &[("c", &c.change_count()), ("c2", &(c.size() / 1024))],
                        ));
                    if r.clicked() {
                        jump = Some(i + 1);
                    }
                }
            });
        if let Some(p) = jump {
            self.jump_history(p);
        }
    }

    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // the facts on the right first; the message gets what is left (truncated, so a long
            // message never runs under the facts)
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.doc.is_some() {
                    let (e, w, _) = self.issue_counts();
                    let badge = if e > 0 {
                        RichText::new(trf(
                            "kfx_editor.e_errors_w_warnings",
                            "✖ {e} errors, {w} warnings",
                            &[("e", &e), ("w", &w)],
                        ))
                        .color(Color32::from_rgb(240, 90, 80))
                    } else if w > 0 {
                        RichText::new(trf(
                            "kfx_editor.w_warnings_6e7f",
                            "⚠ {w} warnings",
                            &[("w", &w)],
                        ))
                        .color(Color32::from_rgb(240, 190, 80))
                    } else {
                        RichText::new(trd("kfx_editor.valid", "✔ valid"))
                            .color(Color32::from_rgb(120, 200, 120))
                    };
                    if ui
                        .add(egui::Button::new(badge.small()).frame(false))
                        .on_hover_text(trd(
                            "kfx_editor.validation_click_for_the_list",
                            "Validation (click for the list)",
                        ))
                        .clicked()
                    {
                        self.settings.layout.show_history = true;
                        self.settings.layout.right_tab = "validate".into();
                    }
                    if let Some(l) = self.layers_summary() {
                        if ui
                            .add(
                                egui::Button::new(RichText::new(l).small().color(GOLD_DIM))
                                    .frame(false),
                            )
                            .on_hover_text(trd(
                                "kfx_editor.configuration_layers_and_mods_click_for_the_deta",
                                "Configuration layers and mods (click for the details)",
                            ))
                            .clicked()
                        {
                            self.show_layers = true;
                        }
                    }
                    ui.label(
                        RichText::new(trf(
                            "kfx_editor.billboards_sprites_things_ms_ms_sprite_zips_spri",
                            "{billboards} sprites ({things_ms} ms) · {sprite_zips} \
                            sprite zips",
                            &[
                                ("billboards", &self.perf.billboards),
                                ("things_ms", &format!("{:.1}", self.perf.things_ms)),
                                ("sprite_zips", &self.sprite_zips),
                            ],
                        ))
                        .small()
                        .color(TEXT_DIM),
                    );
                    ui.label(
                        RichText::new(trf(
                            "kfx_editor.triangles_tris_chunks_chunks_last_mesh_mesh_ms_m",
                            "{triangles} triangles · {chunks} chunks · last mesh \
                            {mesh_ms} ms · zoom {zoom}% · {_}",
                            &[
                                ("triangles", &self.view.triangles),
                                ("chunks", &self.perf.chunks),
                                ("mesh_ms", &format!("{:.1}", self.perf.mesh_ms)),
                                (
                                    "zoom",
                                    &format!(
                                        "{:.0}",
                                        if self.settings.layout.view_mode
                                            == crate::settings::ViewMode::TwoD
                                        {
                                            self.camera2.zoom
                                        } else {
                                            self.camera.zoom
                                        } * 100.0
                                    ),
                                ),
                                (
                                    "_",
                                    &(match self.settings.layout.view_mode {
                                        crate::settings::ViewMode::TwoD => {
                                            trd("kfx_editor.2d", "2D")
                                        }
                                        crate::settings::ViewMode::Split => {
                                            trd("kfx_editor.2d_3d", "2D + 3D")
                                        }
                                        _ if self.camera.perspective => {
                                            trd("kfx_editor.perspective_2cb5", "perspective")
                                        }
                                        _ => trd("kfx_editor.isometric", "isometric"),
                                    }),
                                ),
                            ],
                        ))
                        .small()
                        .color(TEXT_DIM),
                    );
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&self.status).small().color(TEXT_DIM))
                            .truncate(),
                    )
                    .on_hover_text(&self.status);
                });
            });
        });
    }
}
