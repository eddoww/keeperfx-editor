//! The menu bar.
use super::*;

impl EditorApp {
    pub(super) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let has_data = self.assets.is_some();
        let has_doc = self.doc.is_some();
        let k = self.keymap.clone();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(trd("kfx_editor.file", "File"), |ui| {
                self.file_menu(ui, has_data, has_doc, &k);
            });
            ui.menu_button(trd("kfx_editor.edit", "Edit"), |ui| {
                self.edit_menu(ui, has_doc, &k);
            });
            ui.menu_button(trd("kfx_editor.view", "View"), |ui| {
                self.view_menu(ui, has_doc, &k);
            });
            ui.menu_button(trd("kfx_editor.level", "Level"), |ui| {
                self.level_menu(ui, has_data, has_doc, &k);
            });
            ui.menu_button(trd("kfx_editor.campaign", "Campaign"), |ui| {
                self.campaign_menu(ui);
            });
            ui.menu_button(trd("kfx_editor.map", "Map"), |ui| {
                self.map_menu(ui, has_doc, &k);
            });
            ui.menu_button(trd("kfx_editor.tools", "Tools"), |ui| {
                self.tools_menu(ui, has_doc);
            });
            ui.menu_button(trd("kfx_editor.configuration", "Configuration"), |ui| {
                self.configuration_menu(ui, has_doc);
            });
            ui.menu_button(trd("kfx_editor.window", "Window"), |ui| {
                self.window_menu(ui, has_doc, &k);
            });
            ui.menu_button(trd("kfx_editor.help", "Help"), |ui| {
                self.help_menu(ui, &k);
            });
        });
    }

    /// The File menu.
    fn file_menu(
        &mut self,
        ui: &mut egui::Ui,
        has_data: bool,
        has_doc: bool,
        k: &crate::shortcuts::Keymap,
    ) {
        if ui
            .add_enabled(
                has_data,
                egui::Button::new(trd("kfx_editor.new_map_4b6d", "New map…"))
                    .shortcut_text(k.label("file.new")),
            )
            .clicked()
        {
            self.request(Pending::New);
        }
        if ui
            .add_enabled(
                has_data,
                egui::Button::new(trd("kfx_editor.open_59bf", "Open…"))
                    .shortcut_text(k.label("file.open")),
            )
            .clicked()
        {
            self.open_dialog();
        }
        if ui
            .add_enabled(
                has_data,
                egui::Button::new(trd("kfx_editor.map_browser_d3bf", "Map browser…"))
                    .shortcut_text(k.label("file.browser")),
            )
            .clicked()
        {
            self.browser.open = true;
        }
        ui.menu_button(trd("kfx_editor.recent_maps", "Recent maps"), |ui| {
            if self.settings.recent.is_empty() {
                ui.add_enabled(false, egui::Button::new(trd("kfx_editor.none", "(none)")));
            }
            for p in self.settings.recent.clone() {
                let label = match self.settings.recent_name(&p) {
                    Some(n) => format!("{n}  —  {}", p.display()),
                    None => p.display().to_string(),
                };
                if ui
                    .add_enabled(p.exists(), egui::Button::new(label))
                    .on_disabled_hover_text(trd(
                        "kfx_editor.the_file_no_longer_exists",
                        "the file no longer exists",
                    ))
                    .clicked()
                {
                    self.request(Pending::Open(p));
                }
            }
        });
        ui.separator();
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd(
                    "kfx_editor.save_map_and_scripts",
                    "Save (map and scripts)",
                ))
                .shortcut_text(k.label("file.save")),
            )
            .clicked()
        {
            self.do_save();
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.save_as", "Save as…"))
                    .shortcut_text(k.label("file.save_as")),
            )
            .clicked()
        {
            self.open_save_as();
        }
        if ui
            .add_enabled(
                has_doc && self.doc_on_disk,
                egui::Button::new(trd("kfx_editor.reload", "Reload")),
            )
            .clicked()
        {
            self.request(Pending::Reload);
        }
        if ui
            .add_enabled(
                has_doc && self.doc_on_disk,
                egui::Button::new(trd(
                    "kfx_editor.open_containing_folder",
                    "Open containing folder",
                )),
            )
            .clicked()
        {
            self.open_containing_folder();
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.level_files", "Level files…")),
            )
            .on_hover_text(trd(
                "kfx_editor.every_file_of_the_level_with_its_role",
                "Every file of the level with its role",
            ))
            .clicked()
        {
            self.settings.layout.show_history = true;
            self.settings.layout.right_tab = "files".into();
        }
        ui.add_enabled_ui(has_doc, |ui| {
            ui.menu_button(trd("kfx_editor.save_format", "Save format"), |ui| {
                let legacy = self.save_format_legacy();
                if ui
                    .radio(
                        !legacy,
                        trd(
                            "kfx_editor.keeperfx_tngfx_aptfx_lgtfx",
                            "KeeperFX (.tngfx / .aptfx / .lgtfx)",
                        ),
                    )
                    .clicked()
                {
                    self.set_save_format(false);
                    ui.close();
                }
                if ui
                    .radio(
                        legacy,
                        trd(
                            "kfx_editor.original_dk_tng_apt_lgt",
                            "Original DK (.tng / .apt / .lgt)",
                        ),
                    )
                    .on_hover_text(trd(
                        "kfx_editor.checked_before_every_save_what_cannot_be_written",
                        "Checked before every save: what cannot be written is \
                        listed, errors block the save",
                    ))
                    .clicked()
                {
                    self.set_save_format(true);
                    ui.close();
                }
            });
        });
        if ui
            .button(trd("kfx_editor.backups_4650", "Backups…"))
            .clicked()
        {
            self.open_backups();
        }
        ui.separator();
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.image_to_map_8f3b", "Image to map…")),
            )
            .clicked()
        {
            self.open_image_dialog(None);
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.export_image_de3e", "Export image…")),
            )
            .clicked()
        {
            self.open_export_dialog();
        }
        ui.separator();
        if ui.button(trd("kfx_editor.exit", "Exit")).clicked() {
            self.request(Pending::Exit);
        }
    }

    /// The Edit menu.
    fn edit_menu(&mut self, ui: &mut egui::Ui, has_doc: bool, k: &crate::shortcuts::Keymap) {
        let (u, r) = match &self.doc {
            Some(d) => (
                d.undo_name().map(str::to_string),
                d.redo_name().map(str::to_string),
            ),
            None => (None, None),
        };
        let ul = u
            .as_ref()
            .map(|n| trf("kfx_editor.undo_n_1d8b", "Undo {n}", &[("n", &n)]))
            .unwrap_or_else(|| trd("kfx_editor.undo_530f", "Undo").into());
        let rl = r
            .as_ref()
            .map(|n| trf("kfx_editor.redo_n_4925", "Redo {n}", &[("n", &n)]))
            .unwrap_or_else(|| trd("kfx_editor.redo_0149", "Redo").into());
        if ui
            .add_enabled(
                u.is_some(),
                egui::Button::new(ul).shortcut_text(k.label("edit.undo")),
            )
            .clicked()
        {
            self.undo();
        }
        if ui
            .add_enabled(
                r.is_some(),
                egui::Button::new(rl).shortcut_text(k.label("edit.redo")),
            )
            .clicked()
        {
            self.redo();
        }
        ui.separator();
        let sel = self.selection.is_some();
        if ui
            .add_enabled(
                sel,
                egui::Button::new(trd("kfx_editor.cut", "Cut")).shortcut_text("Ctrl+X"),
            )
            .clicked()
        {
            self.cut();
        }
        if ui
            .add_enabled(
                sel,
                egui::Button::new(trd("kfx_editor.copy", "Copy")).shortcut_text("Ctrl+C"),
            )
            .clicked()
        {
            self.copy();
        }
        if ui
            .add_enabled(
                self.clipboard.is_some() && has_doc,
                egui::Button::new(trd("kfx_editor.paste", "Paste")).shortcut_text("Ctrl+V"),
            )
            .clicked()
        {
            self.paste();
        }
        if ui
            .add_enabled(
                sel,
                egui::Button::new(trd(
                    "kfx_editor.fill_selection_with_palette_slab",
                    "Fill selection with palette slab",
                ))
                .shortcut_text(k.label("edit.delete")),
            )
            .clicked()
        {
            self.delete_selection();
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.select_all", "Select all"))
                    .shortcut_text(k.label("edit.select_all")),
            )
            .clicked()
        {
            self.select_all();
        }
        ui.separator();
        if ui
            .add_enabled(
                has_doc && sel,
                egui::Button::new(trd(
                    "kfx_editor.save_selection_as_prefab_cd24",
                    "Save selection as prefab…",
                )),
            )
            .on_hover_text(trd(
                "kfx_editor.keep_the_selected_slabs_things_and_action_points",
                "Keep the selected slabs, things and action points as a prefab \
                to place again",
            ))
            .clicked()
        {
            self.open_prefab_save_dialog();
        }
    }

    /// The View menu.
    fn view_menu(&mut self, ui: &mut egui::Ui, has_doc: bool, k: &crate::shortcuts::Keymap) {
        use crate::settings::ViewMode;
        let vm = self.settings.layout.view_mode;
        if ui
            .add(egui::Button::selectable(
                vm == ViewMode::ThreeD,
                trd("kfx_editor.3d_view", "3D view"),
            ))
            .clicked()
        {
            self.set_view_mode(ViewMode::ThreeD);
        }
        if ui
            .add(
                egui::Button::selectable(
                    vm == ViewMode::TwoD,
                    trd("kfx_editor.2d_view_top_down", "2D view (top-down)"),
                )
                .shortcut_text(k.label("view.2d")),
            )
            .clicked()
        {
            self.set_view_mode(ViewMode::TwoD);
        }
        if ui
            .add(
                egui::Button::selectable(
                    vm == ViewMode::Split,
                    trd("kfx_editor.split_view_2d_3d", "Split view (2D + 3D)"),
                )
                .shortcut_text(k.label("view.split")),
            )
            .clicked()
        {
            self.set_view_mode(ViewMode::Split);
        }
        ui.checkbox(
            &mut self.settings.layout.flat_colors,
            trd(
                "kfx_editor.2d_flat_slab_owner_colours",
                "2D: flat slab / owner colours",
            ),
        );
        if ui
            .add_enabled(
                has_doc,
                egui::Button::selectable(
                    self.camera.fly,
                    trd("kfx_editor.first_person_fly_view", "First-person fly view"),
                )
                .shortcut_text(k.label("view.fly")),
            )
            .on_hover_text(trd(
                "kfx_editor.wasd_move_q_e_down_up_shift_faster_m_captures_th",
                "WASD move, Q/E down/up, Shift faster, M captures the mouse for \
                mouse-look",
            ))
            .clicked()
        {
            self.toggle_fly();
        }
        ui.checkbox(
            &mut self.settings.camera.fly_collision,
            trd("kfx_editor.fly_view_collision", "Fly view collision"),
        )
        .on_hover_text(trd(
            "kfx_editor.the_eye_stops_at_walls_and_stays_above_the_floor",
            "The eye stops at walls and stays above the floor",
        ));
        ui.menu_button(trd("kfx_editor.grid_data", "Grid data"), |ui| {
            for g in crate::griddata::GridData::ALL {
                if ui
                    .radio(self.settings.layout.grid_data == g, g.label())
                    .clicked()
                {
                    self.settings.layout.grid_data = g;
                    ui.close();
                }
            }
        });
        ui.separator();
        let mut p = self.camera.perspective;
        if ui
            .checkbox(&mut p, trd("kfx_editor.perspective", "Perspective"))
            .changed()
        {
            self.set_perspective(p);
        }
        ui.checkbox(
            &mut self.settings.graphics.overview_boost,
            trd(
                "kfx_editor.brighter_when_zoomed_out",
                "Brighter when zoomed out",
            ),
        )
        .on_hover_text(trd(
            "kfx_editor.a_tone_curve_that_lifts_the_dark_textures_in_who",
            "A tone curve that lifts the dark textures in whole-map overviews",
        ));
        ui.checkbox(
            &mut self.settings.graphics.local_lights,
            trd(
                "kfx_editor.map_lights_when_zoomed_in",
                "Map lights when zoomed in",
            ),
        );
        ui.checkbox(
            &mut self.settings.graphics.declutter,
            trd(
                "kfx_editor.cluster_markers_when_zoomed_out",
                "Cluster markers when zoomed out",
            ),
        );
        let l = &mut self.settings.layout;
        ui.checkbox(
            &mut l.show_ownership,
            trd("kfx_editor.ownership_tint", "Ownership tint"),
        );
        ui.checkbox(&mut l.show_grid, trd("kfx_editor.slab_grid", "Slab grid"));
        ui.checkbox(&mut l.lighting, trd("kfx_editor.lighting", "Lighting"));
        ui.checkbox(&mut l.show_minimap, trd("kfx_editor.minimap", "Minimap"));
        ui.checkbox(
            &mut l.tabs_left,
            trd(
                "kfx_editor.panels_as_tabs_beside_the_tree",
                "Panels as tabs beside the tree",
            ),
        )
        .on_hover_text(trd(
            "kfx_editor.history_inspector_things_layers_validate_notes_a",
            "History, Inspector, Things, Layers, Validate, Notes and Files \
            as tabs of the left panel (else in a right panel)",
        ));
        ui.add_enabled(
            !l.tabs_left,
            egui::Checkbox::new(
                &mut l.show_history,
                trd(
                    "kfx_editor.right_panel_history_inspector_things_layers_vali",
                    "Right panel (History, Inspector, Things, Layers, Validate)",
                ),
            ),
        );
        ui.menu_button(trd("kfx_editor.layers", "Layers"), |ui| {
            for n in crate::settings::LAYER_NAMES {
                if let Some(v) = l.layers.get_mut(n) {
                    ui.checkbox(v, n.replace('_', " "));
                }
            }
        });
        if ui
            .add_enabled(
                has_doc,
                egui::Button::selectable(
                    self.settings.layout.analysis.open,
                    trd("kfx_editor.analysis", "Analysis"),
                )
                .shortcut_text(k.label("view.analysis")),
            )
            .on_hover_text(trd(
                "kfx_editor.reachability_distances_routes_economy_fairness_s",
                "Reachability, distances, routes, economy, fairness, symmetry \
                and hero pressure as layers over the map",
            ))
            .clicked()
        {
            let o = !self.settings.layout.analysis.open;
            self.set_analysis_open(o);
        }
        ui.separator();
        if ui
            .add(
                egui::Button::new(trd("kfx_editor.script_panel", "Script panel"))
                    .shortcut_text(k.label("view.script")),
            )
            .clicked()
        {
            self.toggle_script_panel();
        }
        if ui
            .add_enabled(
                self.doc.is_some(),
                egui::Button::new(trd("kfx_editor.events_editor", "Events editor"))
                    .shortcut_text(k.label("view.events")),
            )
            .on_hover_text(trd(
                "kfx_editor.the_level_s_events_parties_and_waves_as_cards_a",
                "The level's events, parties and waves as cards, a graph and a \
                timeline: a tab of the script panel",
            ))
            .clicked()
        {
            self.toggle_events();
        }
        if ui
            .add(
                egui::Button::new(trd(
                    "kfx_editor.maximise_scripts_over_the_map",
                    "Maximise scripts over the map",
                ))
                .shortcut_text(k.label("view.script_max")),
            )
            .clicked()
        {
            self.script_max = !self.script_max;
        }
        ui.separator();
        if ui
            .add(
                egui::Button::new(trd(
                    "kfx_editor.reset_camera_game_view",
                    "Reset camera (game view)",
                ))
                .shortcut_text(k.label("cam.home")),
            )
            .clicked()
        {
            self.camera_pending_home = true;
        }
        if ui
            .add(
                egui::Button::new(trd("kfx_editor.rotate_left", "Rotate left"))
                    .shortcut_text(k.label("cam.rot_left")),
            )
            .clicked()
        {
            self.camera.rotate_step(-1);
        }
        if ui
            .add(
                egui::Button::new(trd("kfx_editor.rotate_right", "Rotate right"))
                    .shortcut_text(k.label("cam.rot_right")),
            )
            .clicked()
        {
            self.camera.rotate_step(1);
        }
    }

    /// The Level menu.
    fn level_menu(
        &mut self,
        ui: &mut egui::Ui,
        has_data: bool,
        has_doc: bool,
        k: &crate::shortcuts::Keymap,
    ) {
        if ui
            .add_enabled(
                has_data,
                egui::Button::new(trd("kfx_editor.map_browser_d3bf", "Map browser…")),
            )
            .clicked()
        {
            self.browser.open = true;
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.level_properties_124e", "Level properties…")),
            )
            .clicked()
        {
            self.open_properties();
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.scripts", "Scripts"))
                    .shortcut_text(k.label("view.script")),
            )
            .clicked()
        {
            self.toggle_script_panel();
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.events", "Events…"))
                    .shortcut_text(k.label("view.events")),
            )
            .on_hover_text(trd(
                "kfx_editor.waves_triggers_and_parties_of_the_dk_script_edit",
                "Waves, triggers and parties of the DK script, edited visually",
            ))
            .clicked()
        {
            self.open_events();
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.script_wizard", "Script wizard…")),
            )
            .on_hover_text(trd(
                "kfx_editor.start_gold_creatures_rooms_powers_traps_doors_co",
                "Start gold, creatures, rooms, powers, traps, doors, computer \
                players, alliances and win / lose conditions, written as one marked block \
                of the DK script",
            ))
            .clicked()
        {
            self.open_script_wizard(kfx_script_ui::wizard::ScriptLang::Dk);
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.lua_script_wizard", "Lua script wizard…")),
            )
            .on_hover_text(trd(
                "kfx_editor.the_same_settings_as_a_marked_block_of_the_level",
                "The same settings as a marked block of the level's Lua script",
            ))
            .clicked()
        {
            self.open_script_wizard(kfx_script_ui::wizard::ScriptLang::Lua);
        }
        ui.menu_button(trd("kfx_editor.open_externally", "Open externally"), |ui| {
            for (label, what) in [
                (trd("kfx_editor.dk_script_txt", "DK script (.txt)"), "txt"),
                (trd("kfx_editor.lua_script_lua", "Lua script (.lua)"), "lua"),
                (
                    trd(
                        "kfx_editor.game_log_keeperfx_log",
                        "Game log (keeperfx.log)",
                    ),
                    "log",
                ),
            ] {
                if ui
                    .add_enabled(has_doc || what == "log", egui::Button::new(label))
                    .clicked()
                {
                    self.open_level_file_externally(what);
                    ui.close();
                }
            }
        });
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.validate", "Validate")),
            )
            .clicked()
        {
            self.revalidate();
            self.settings.layout.show_history = true;
            self.settings.layout.right_tab = "validate".into();
        }
        ui.separator();
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.play", "▶ Play"))
                    .shortcut_text(k.label("play.play")),
            )
            .clicked()
        {
            self.play(false);
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.save_and_play", "Save and play"))
                    .shortcut_text(k.label("play.save_play")),
            )
            .clicked()
        {
            self.play(true);
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd(
                    "kfx_editor.play_and_record_packet_save",
                    "Play and record (packet save)",
                )),
            )
            .clicked()
        {
            self.play_packet(false);
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd(
                    "kfx_editor.replay_the_last_recording_packet_load",
                    "Replay the last recording (packet load)",
                )),
            )
            .clicked()
        {
            self.play_packet(true);
        }
        if ui
            .add(
                egui::Button::new(trd("kfx_editor.game_log", "Game log"))
                    .shortcut_text(k.label("play.log")),
            )
            .clicked()
        {
            self.refresh_game_log();
            self.game_log.open = true;
        }
        if ui
            .add(
                egui::Button::new(trd("kfx_editor.live_link", "Live link"))
                    .shortcut_text(k.label("play.live")),
            )
            .clicked()
        {
            self.settings.audiolive.live_open = true;
        }
    }

    /// The Map menu.
    fn map_menu(&mut self, ui: &mut egui::Ui, has_doc: bool, k: &crate::shortcuts::Keymap) {
        ui.checkbox(
            &mut self.editable_border,
            trd("kfx_editor.editable_border", "Editable border"),
        )
        .on_hover_text(trd(
            "kfx_editor.allow_editing_the_outermost_ring_of_slabs",
            "Allow editing the outermost ring of slabs",
        ));
        ui.checkbox(
            &mut self.settings.ownable_natural,
            trd(
                "kfx_editor.ownable_natural_terrain",
                "Ownable natural terrain",
            ),
        )
        .on_hover_text(trd(
            "kfx_editor.earth_rock_gold_and_liquids_take_the_painted_own",
            "Earth, rock, gold and liquids take the painted owner (else they \
            stay neutral)",
        ));
        ui.checkbox(
            &mut self.settings.place_anywhere,
            trd("kfx_editor.place_things_anywhere", "Place things anywhere"),
        )
        .on_hover_text(trd(
            "kfx_editor.allow_things_inside_walls_and_rock_columns_of_5",
            "Allow things inside walls and rock (columns of 5 cubes or more)",
        ));
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.resize_4b27", "Resize…")),
            )
            .clicked()
        {
            self.resize_dialog = Some(panels::ResizeDialog {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
                fill: "EARTH".into(),
            });
        }
        let sel = self.selection.is_some();
        ui.menu_button(trd("kfx_editor.flip_rotate", "Flip / rotate"), |ui| {
            use kfx_map::transform::Transform as T;
            for (t, l) in [
                (T::FlipX, trd("kfx_editor.flip_east_west", "Flip east-west")),
                (
                    T::FlipY,
                    trd("kfx_editor.flip_north_south", "Flip north-south"),
                ),
                (
                    T::Rot90,
                    trd("kfx_editor.rotate_90_clockwise", "Rotate 90° clockwise"),
                ),
                (T::Rot180, trd("kfx_editor.rotate_180", "Rotate 180°")),
                (
                    T::Rot270,
                    trd(
                        "kfx_editor.rotate_90_anticlockwise",
                        "Rotate 90° anticlockwise",
                    ),
                ),
            ] {
                ui.horizontal(|ui| {
                    ui.label(l);
                    if ui
                        .add_enabled(
                            has_doc,
                            egui::Button::new(trd("kfx_editor.map_2d91", "map")),
                        )
                        .clicked()
                    {
                        if let Err(e) = self.transform(t, false) {
                            self.status = e;
                        }
                    }
                    if ui
                        .add_enabled(
                            has_doc && sel,
                            egui::Button::new(trd("kfx_editor.selection", "selection")),
                        )
                        .clicked()
                    {
                        if let Err(e) = self.transform(t, true) {
                            self.status = e;
                        }
                    }
                });
            }
        });
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.symmetry_87ed", "Symmetry…")),
            )
            .clicked()
        {
            self.show_symmetry = true;
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd(
                    "kfx_editor.decoration_of_this_map_9e56",
                    "Decoration of this map…",
                )),
            )
            .clicked()
        {
            self.open_decor_dialog();
        }
        if ui
            .add_enabled(
                has_doc && sel,
                egui::Button::new(trd(
                    "kfx_editor.generate_into_selection_c8f6",
                    "Generate into selection…",
                ))
                .shortcut_text(k.label("map.generate_selection")),
            )
            .on_hover_text(trd(
                "kfx_editor.fill_the_selected_rectangle_with_generated_terra",
                "Fill the selected rectangle with generated terrain (one undo \
                step)",
            ))
            .on_disabled_hover_text(trd(
                "kfx_editor.select_a_rectangle_of_slabs_first",
                "Select a rectangle of slabs first",
            ))
            .clicked()
        {
            self.open_gen_patch();
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.random_doodads_85ba", "Random doodads…")),
            )
            .on_hover_text(trd(
                "kfx_editor.things_scattered_on_chosen_slab_kinds_seeded_sav",
                "Things scattered on chosen slab kinds, seeded, saved with the \
                map",
            ))
            .clicked()
        {
            self.open_doodad_dialog();
        }
        ui.separator();
        if ui
            .add_enabled(
                has_doc && sel,
                egui::Button::new(trd("kfx_editor.lock_selection", "🔒 Lock selection")),
            )
            .on_hover_text(trd(
                "kfx_editor.tools_refuse_to_change_these_slabs_stored_with_t",
                "Tools refuse to change these slabs (stored with the map)",
            ))
            .clicked()
        {
            if let Err(e) = self.lock_selection("") {
                self.status = e;
            }
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.unlock_selection", "🔓 Unlock selection")),
            )
            .clicked()
        {
            if let Err(e) = self.unlock_selection() {
                self.status = e;
            }
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.add_note", "📝 Add note…")),
            )
            .on_hover_text(trd(
                "kfx_editor.pin_a_note_to_the_selected_or_hovered_slab",
                "Pin a note to the selected (or hovered) slab",
            ))
            .clicked()
        {
            self.open_note_dialog();
        }
        let stl = self
            .hover
            .as_ref()
            .map(|h| (h.hit.stl_x, h.hit.stl_y))
            .or(self
                .selection
                .map(|(a, _)| (a.0 as usize * 3 + 1, a.1 as usize * 3 + 1)));
        if ui
            .add_enabled(
                has_doc && stl.is_some(),
                egui::Button::new(trd("kfx_editor.edit_column", "Edit column…")),
            )
            .on_hover_text(trd(
                "kfx_editor.cubes_and_floor_of_the_selected_slab_s_centre_su",
                "Cubes and floor of the selected slab's centre subtile (stored \
                as a custom slab)",
            ))
            .clicked()
        {
            if let Some(p) = stl {
                self.open_column_dialog(p);
            }
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd(
                    "kfx_editor.columns_of_this_map_fd4f",
                    "Columns of this map…",
                )),
            )
            .on_hover_text(trd(
                "kfx_editor.the_distinct_columns_the_save_writes_to_the_clm",
                "The distinct columns the save writes to the .clm: use counts, \
                edit one everywhere it is used",
            ))
            .clicked()
        {
            self.open_columns_window();
        }
        if ui
            .add_enabled(
                has_doc && sel,
                egui::Button::new(trd(
                    "kfx_editor.new_custom_slab_from_selection",
                    "New custom slab from selection",
                )),
            )
            .clicked()
        {
            if let Some(((x, y), _)) = self.selection {
                if let Err(e) = self.create_custom_from_slab(x, y, None) {
                    self.status = e;
                }
            }
        }
    }

    /// The Tools menu.
    fn tools_menu(&mut self, ui: &mut egui::Ui, has_doc: bool) {
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd(
                    "kfx_editor.regenerate_all_slabs_128b",
                    "Regenerate all slabs…",
                )),
            )
            .clicked()
        {
            self.regen_dialog = Some(panels::RegenDialog {
                what: document::Regen::default(),
                selection: false,
                confirm: false,
            });
        }
        if ui
            .add_enabled(
                has_doc && self.selection.is_some(),
                egui::Button::new(trd(
                    "kfx_editor.regenerate_selection_b8a3",
                    "Regenerate selection…",
                )),
            )
            .clicked()
        {
            self.regen_dialog = Some(panels::RegenDialog {
                what: document::Regen::default(),
                selection: true,
                confirm: false,
            });
        }
        ui.separator();
        for t in [
            Tool::Paint,
            Tool::Line,
            Tool::Box,
            Tool::Circle,
            Tool::Fill,
            Tool::Eyedropper,
            Tool::Select,
            Tool::Place,
            Tool::Things,
        ] {
            if ui.radio(self.tool == t, t.label()).clicked() {
                self.set_tool(t);
            }
        }
        ui.separator();
        ui.checkbox(
            &mut self.owner_only,
            trd("kfx_editor.ownership_only", "Ownership only"),
        );
        ui.checkbox(
            &mut self.shape_filled,
            trd("kfx_editor.filled_shapes", "Filled shapes"),
        );
    }

    /// The Configuration menu.
    fn configuration_menu(&mut self, ui: &mut egui::Ui, has_doc: bool) {
        use kfx_config_ui::ui::Tab as T;
        for (label, tab) in [
            (
                trd(
                    "kfx_editor.configuration_editor_1d4e",
                    "Configuration editor…",
                ),
                None,
            ),
            (trd("kfx_editor.rules", "Rules…"), Some(T::Rules)),
            (
                trd("kfx_editor.creatures_2e37", "Creatures…"),
                Some(T::Creatures),
            ),
            (trd("kfx_editor.objects_1b15", "Objects…"), Some(T::Objects)),
            (
                trd("kfx_editor.traps_doors", "Traps & doors…"),
                Some(T::TrapsDoors),
            ),
            (trd("kfx_editor.rooms_d2b5", "Rooms…"), Some(T::Rooms)),
            (trd("kfx_editor.terrain_309e", "Terrain…"), Some(T::Terrain)),
            (trd("kfx_editor.slab_sets", "Slab sets…"), Some(T::Slabset)),
            (trd("kfx_editor.columns", "Columns…"), Some(T::Columns)),
            (trd("kfx_editor.cubes", "Cubes…"), Some(T::Cubes)),
        ] {
            if ui
                .add_enabled(has_doc, egui::Button::new(label))
                .on_hover_text(trd(
                    "kfx_editor.edit_the_configuration_of_this_map_or_its_campai",
                    "Edit the configuration of this map (or its campaign / a mod)",
                ))
                .clicked()
            {
                self.open_config_editor(tab);
            }
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.sounds", "Sounds…")),
            )
            .on_hover_text(trd(
                "kfx_editor.the_sounds_cfg_mappings_with_a_sound_browser_lis",
                "The sounds.cfg mappings with a sound browser: listen to every \
                sound of this level",
            ))
            .clicked()
        {
            self.open_config_editor(Some(T::Sounds));
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.creature_stats", "Creature stats…")),
            )
            .on_hover_text(trd(
                "kfx_editor.all_creatures_side_by_side_at_a_level_with_the_o",
                "All creatures side by side at a level, with the overrides of \
                this map, its campaign and mods",
            ))
            .clicked()
        {
            self.open_creature_stats();
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.layers_mods_a396", "Layers & mods…")),
            )
            .clicked()
        {
            self.show_layers = true;
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd("kfx_editor.strings_11f7", "Strings…")),
            )
            .clicked()
        {
            self.show_strings = true;
        }
        let mut ign = self.settings.ignore_mods;
        if ui
            .checkbox(&mut ign, trd("kfx_editor.ignore_mods", "Ignore mods"))
            .on_hover_text(trd(
                "kfx_editor.the_game_ignores_mods_in_multiplayer",
                "The game ignores mods in multiplayer",
            ))
            .changed()
        {
            self.settings.ignore_mods = ign;
            self.save_settings();
            self.reload_mods();
        }
        ui.separator();
        if ui
            .button(trd("kfx_editor.keeperfx_folder_8559", "KeeperFX folder…"))
            .clicked()
        {
            self.first_run.open = true;
            self.first_run.text = self
                .install()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
        }
        if ui
            .button(trd("kfx_editor.preferences_c063", "Preferences…"))
            .clicked()
        {
            self.show_prefs = true;
        }
        if ui
            .button(trd(
                "kfx_editor.keyboard_shortcuts_baa1",
                "Keyboard shortcuts…",
            ))
            .clicked()
        {
            self.show_shortcuts = true;
        }
    }

    /// The Window menu.
    fn window_menu(&mut self, ui: &mut egui::Ui, has_doc: bool, k: &crate::shortcuts::Keymap) {
        ui.checkbox(
            &mut self.browser.open,
            trd("kfx_editor.map_browser", "Map browser"),
        );
        ui.checkbox(
            &mut self.settings.layout.tabs_left,
            trd(
                "kfx_editor.panels_as_tabs_beside_the_tree",
                "Panels as tabs beside the tree",
            ),
        );
        ui.add_enabled(
            !self.settings.layout.tabs_left,
            egui::Checkbox::new(
                &mut self.settings.layout.show_history,
                trd("kfx_editor.right_panel", "Right panel"),
            ),
        );
        ui.checkbox(
            &mut self.settings.layout.show_minimap,
            trd("kfx_editor.minimap", "Minimap"),
        );
        ui.checkbox(
            &mut self.game_log.open,
            trd("kfx_editor.game_log", "Game log"),
        );
        ui.checkbox(
            &mut self.settings.audiolive.live_open,
            trd("kfx_editor.live_link", "Live link"),
        );
        if ui.button(trd("kfx_editor.scripts", "Scripts")).clicked() {
            self.toggle_script_panel();
        }
        if ui
            .add_enabled(
                has_doc,
                egui::Button::new(trd(
                    "kfx_editor.configuration_editor",
                    "Configuration editor",
                )),
            )
            .clicked()
        {
            self.open_config_editor(None);
        }
        ui.checkbox(
            &mut self.show_layers,
            trd("kfx_editor.layers_mods", "Layers & mods"),
        );
        ui.checkbox(&mut self.show_strings, trd("kfx_editor.strings", "Strings"));
        if ui
            .add(
                egui::Button::selectable(
                    self.settings.prefabs.open,
                    trd("kfx_editor.prefabs", "Prefabs"),
                )
                .shortcut_text(k.label("view.prefabs")),
            )
            .clicked()
        {
            self.toggle_prefabs();
        }
        for (k, l) in [
            ("files", trd("kfx_editor.level_files_3bbe", "Level files")),
            ("notes", trd("kfx_editor.notes", "Notes")),
        ] {
            if ui.button(l).clicked() {
                self.settings.layout.show_history = true;
                self.settings.layout.right_tab = k.into();
            }
        }
    }

    /// The Help menu.
    fn help_menu(&mut self, ui: &mut egui::Ui, k: &crate::shortcuts::Keymap) {
        if ui
            .add(
                egui::Button::new(trd("kfx_editor.controls", "Controls"))
                    .shortcut_text(k.label("help.controls")),
            )
            .clicked()
        {
            self.show_controls = true;
        }
        ui.menu_button(trd("kfx_editor.keeperfx_wiki", "KeeperFX wiki"), |ui| {
            for (n, u) in dialogs::WIKI {
                if ui.button(*n).on_hover_text(*u).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(*u));
                }
            }
        });
        if ui
            .button(trd("kfx_editor.changelog", "Changelog"))
            .clicked()
        {
            self.show_changelog = true;
        }
        if ui.button(trd("kfx_editor.about", "About")).clicked() {
            self.show_about = true;
        }
    }
}
