//! Random doodads (Map > Random doodads), the door lock toggle, the script wizard menu, toasts,
//! the changelog, install checks, the optional release check and the remembered window
//! positions.
use super::*;
use kfx_prefab::i18n::{trd, trf};

/// What the editor found out about the KeeperFX folder (Preferences > Files, first run).
pub struct InstallChecks {
    /// The game's version from the first line of `keeperfx.log` (`Dungeon Keeper FX ver ...`,
    /// written by the game at start), when the game has run once.
    pub version: Option<String>,
    /// The levels folder (or the install) is writable according to its permissions (nothing
    /// is written to test it).
    pub writable: bool,
}

impl InstallChecks {
    pub fn summary(&self) -> String {
        format!(
            "{} · {}",
            self.version
                .as_ref()
                .map(|v| trf("kfx_editor.keeperfx_v", "KeeperFX {v}", &[("v", &v)]))
                .unwrap_or_else(|| trd(
                    "kfx_editor.keeperfx_version_unknown_run_the_game_once_to_wr",
                    "KeeperFX version unknown (run the game once to write keeperfx.log)"
                )
                .into()),
            if self.writable {
                trd(
                    "kfx_editor.maps_can_be_saved_in_this_folder",
                    "maps can be saved in this folder",
                )
            } else {
                trd(
                    "kfx_editor.this_folder_is_read_only_save_maps_elsewhere_fil",
                    "this folder is read-only: save maps elsewhere (File > Save as)",
                )
            }
        )
    }
}

/// Version and write checks of an install folder (a test write, the version from the game's
/// log).
pub fn install_checks(install: &Path) -> InstallChecks {
    let version = kfx_formats::testutil::resolve_ci(install, "keeperfx.log")
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| {
            let first = String::from_utf8_lossy(&b[..b.len().min(200)])
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            first
                .split(" ver ")
                .nth(1)
                .map(|v| v.split_whitespace().next().unwrap_or("").to_string())
                .filter(|v| !v.is_empty())
        });
    let dir = kfx_formats::testutil::resolve_ci(install, "levels")
        .unwrap_or_else(|| install.to_path_buf());
    let writable = std::fs::metadata(&dir)
        .map(|m| !m.permissions().readonly())
        .unwrap_or(false);
    InstallChecks { version, writable }
}

/// Whether the game finds levels saved in `folder`: inside the install's `levels`,
/// `campgns` or `multiplayer` tree. `Ok` names the tree.
pub fn playable_from(install: Option<&Path>, folder: &Path) -> Result<String, String> {
    let Some(install) = install else {
        return Err(trd(
            "kfx_editor.no_keeperfx_folder_chosen",
            "no KeeperFX folder chosen",
        ));
    };
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let (root, f) = (canon(install), canon(folder));
    let rel = f.strip_prefix(&root).map_err(|_| {
        trd(
            "kfx_editor.outside_the_keeperfx_folder_the_game_will_not_li",
            "outside the KeeperFX folder: the game will not list this level",
        )
        .to_string()
    })?;
    let first = rel
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match first.as_str() {
        "levels" | "campgns" | "multiplayer" if rel.components().count() >= 2 => Ok(trf(
            "kfx_editor.first_folder",
            "{first} folder",
            &[("first", &first)],
        )),
        "levels" | "campgns" | "multiplayer" => Err(trf(
            "kfx_editor.directly_in_first_put_the_level_in_a_map_pack_fo",
            "directly in {first}: put the level in a map pack folder below it (e.g. \
            levels/personal)",
            &[("first", &first)],
        )),
        _ => Err(trd(
            "kfx_editor.not_in_levels_campgns_or_multiplayer_the_game_wi",
            "not in levels, campgns or multiplayer: the game will not list this level",
        )),
    }
}

/// The same folder (canonical paths when they exist).
pub fn same_dir(a: &Path, b: &Path) -> bool {
    let c = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    c(a) == c(b)
}

/// Files of level `level` in `folder` packed with RNC (`RNC\x01` header, `rnc.rs`).
pub fn rnc_packed_files(folder: &Path, level: u32) -> usize {
    crate::document::level_files(folder, level)
        .iter()
        .filter(|p| {
            use std::io::Read;
            let mut h = [0u8; 4];
            std::fs::File::open(p)
                .and_then(|mut f| f.read_exact(&mut h))
                .is_ok()
                && &h == b"RNC\x01"
        })
        .count()
}

/// `folder` lies below a `multiplayer` folder.
pub fn in_multiplayer_folder(folder: &Path) -> bool {
    folder.components().any(|c| {
        c.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("multiplayer")
    })
}

/// The embedded changelog (Help > Changelog).
pub const CHANGELOG: &str = include_str!("../../../../CHANGELOG.md");

/// A fading message in the corner (the status bar keeps the last one).
pub struct Toast {
    pub text: String,
    pub at: f64,
}

/// Compare dotted versions (`0.2.10` > `0.2.9`); a leading `v` is ignored.
pub fn newer(remote: &str, local: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.trim()
            .trim_start_matches(['v', 'V'])
            .split(['.', '-', '+'])
            .map_while(|p| p.parse().ok())
            .collect()
    };
    parse(remote) > parse(local)
}

/// Ask `url` (a GitHub "latest release" API URL or anything answering JSON with `tag_name` and
/// `html_url`) for the latest release, with the system's `curl` and a short timeout. Returns
/// (tag, page) when it is newer than this build.
pub fn check_release(url: &str) -> Result<Option<(String, String)>, String> {
    let out = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "8",
            "-H",
            "Accept: application/json",
            "-A",
            "kfx-editor",
            url,
        ])
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{url}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("{url}: {e}"))?;
    let tag = v
        .get("tag_name")
        .or_else(|| v.get("name"))
        .and_then(|t| t.as_str())
        .ok_or("no tag_name in the answer")?
        .to_string();
    let page = v
        .get("html_url")
        .and_then(|t| t.as_str())
        .unwrap_or(url)
        .to_string();
    Ok(newer(&tag, env!("CARGO_PKG_VERSION")).then_some((tag, page)))
}
use crate::doodads::{self, DoodadKind, DoodadRule, Doodads};

/// The open doodad dialog: an editable copy of the map's rules.
pub struct DoodadDialog {
    pub set: Doodads,
    pub seed_text: String,
}

impl EditorApp {
    /// Open a map from a backup without restoring it: Save is refused until it is saved
    /// elsewhere with Save as.
    pub fn open_backup(&mut self, dir: &Path, level: u32) -> Result<(), String> {
        let p = level_bin::find_file(dir, level, "slb").ok_or_else(|| {
            trf(
                "kfx_editor.no_map_level_slb_in_dir",
                "no map{level}.slb in {dir}",
                &[("level", &format!("{:05}", level)), ("dir", &dir.display())],
            )
        })?;
        self.open_map(&p)?;
        self.from_backup = true;
        self.status = trf(
            "kfx_editor.opened_the_backup_in_dir_read_only_file_save_as",
            "Opened the backup in {dir} (read-only: File > Save as saves it elsewhere)",
            &[("dir", &dir.display())],
        );
        Ok(())
    }

    /// Start-up work of a persistent session: restore the window positions, the "what's new"
    /// note, the optional release check (in the background).
    pub(super) fn startup_extras(&mut self) {
        if !self.persist {
            return;
        }
        if !self.settings.ui_memory.is_empty() {
            if let Ok(m) = ron::from_str::<egui::Memory>(&self.settings.ui_memory) {
                self.ctx.memory_mut(|mem| *mem = m);
            }
        }
        let v = env!("CARGO_PKG_VERSION");
        if self.settings.last_seen_version != v {
            if !self.settings.last_seen_version.is_empty() {
                self.status = trf(
                    "kfx_editor.updated_to_kfx_editor_v_help_changelog_lists_wha",
                    "Updated to kfx-editor {v}: Help > Changelog lists what changed",
                    &[("v", &v)],
                );
            }
            self.settings.last_seen_version = v.to_string();
        }
        if self.settings.update_check && !self.settings.update_url.trim().is_empty() {
            let url = self.settings.update_url.trim().to_string();
            let slot = self.release_note.clone();
            std::thread::spawn(move || {
                if let Ok(Some(r)) = check_release(&url) {
                    if let Ok(mut s) = slot.lock() {
                        *s = Some(r);
                    }
                }
            });
        }
    }

    /// Keep the window positions and sizes (egui memory) for the next start.
    pub(super) fn remember_ui_memory(&mut self) {
        if self.persist {
            // RON: egui's maps have non-string keys, which JSON cannot hold
            self.settings.ui_memory = self.ctx.memory(|m| ron::to_string(m).unwrap_or_default());
        }
    }

    /// Toasts: a new status message also shows for a few seconds in the corner of the window
    /// (the same text within 3 s is shown once).
    pub(super) fn toasts_ui(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if let Ok(mut r) = self.release_note.lock() {
            if let Some((tag, page)) = r.take() {
                self.status = trf(
                    "kfx_editor.a_newer_kfx_editor_release_is_available_tag_page",
                    "A newer kfx-editor release is available: {tag} ({page})",
                    &[("tag", &tag), ("page", &page)],
                );
            }
        }
        if self.status != self.last_status {
            self.last_status = self.status.clone();
            if !self.status.is_empty() {
                match self.toasts.iter_mut().find(|t| {
                    self.status.starts_with(t.text.trim_end_matches('…')) && now - t.at < 3.0
                }) {
                    Some(t) => t.at = now,
                    None => {
                        // long messages stay in the status bar; the toast shows the start
                        let mut text: String = self.status.chars().take(160).collect();
                        if text.len() < self.status.len() {
                            text.push('…');
                        }
                        self.toasts.push(Toast { text, at: now });
                    }
                }
            }
        }
        const LIFE: f64 = 4.0;
        self.toasts.retain(|t| now - t.at < LIFE);
        if self.toasts.len() > 4 {
            let n = self.toasts.len() - 4;
            self.toasts.drain(..n);
        }
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::unique("toasts"))
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::RIGHT_BOTTOM, EVec2::new(-14.0, -34.0))
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                for t in &self.toasts {
                    let a = (((LIFE - (now - t.at)) / 0.6).clamp(0.0, 1.0)) as f32;
                    let warn = t.text.starts_with("Not ")
                        || t.text.contains("error")
                        || t.text.contains("failed")
                        || t.text.contains("cannot");
                    let edge = if warn {
                        Color32::from_rgb(230, 120, 80)
                    } else {
                        GOLD_DIM
                    };
                    egui::Frame::popup(ui.style())
                        .fill(Color32::from_rgba_unmultiplied(
                            28,
                            24,
                            20,
                            (235.0 * a) as u8,
                        ))
                        .stroke(egui::Stroke::new(1.0, edge.gamma_multiply(a)))
                        .show(ui, |ui| {
                            ui.label(RichText::new(&t.text).color(theme::TEXT.gamma_multiply(a)));
                        });
                }
            });
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    /// Help > Changelog.
    pub(super) fn changelog_ui(&mut self, ctx: &egui::Context) {
        if !self.show_changelog {
            return;
        }
        let mut open = true;
        egui::Window::new(trd("kfx_editor.changelog", "Changelog"))
            .open(&mut open)
            .default_size([620.0, 520.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for line in CHANGELOG.lines() {
                            if let Some(h) = line.strip_prefix("### ") {
                                ui.add_space(4.0);
                                ui.label(RichText::new(h).strong());
                            } else if let Some(h) = line.strip_prefix("## ") {
                                ui.add_space(6.0);
                                ui.label(RichText::new(h).color(GOLD).strong());
                            } else if let Some(h) = line.strip_prefix("# ") {
                                ui.heading(RichText::new(h).color(GOLD));
                            } else if let Some(b) = line.strip_prefix("- ") {
                                ui.label(format!("• {b}"));
                            } else if !line.trim().is_empty() {
                                ui.label(line);
                            }
                        }
                    });
            });
        self.show_changelog = open;
    }

    /// Open the script wizard (DK or Lua) on the level's script, showing the script panel.
    pub fn open_script_wizard(&mut self, lang: kfx_script_ui::wizard::ScriptLang) {
        if self.doc.is_none() {
            return;
        }
        if self.settings.script_dock == crate::settings::ScriptDock::Hidden {
            self.settings.script_dock = crate::settings::ScriptDock::Docked;
        }
        match self.scripts.as_mut() {
            Some(s) => s.ws.open_wizard(lang),
            None => {
                self.status = trd(
                    "kfx_editor.the_level_s_scripts_are_not_loaded",
                    "the level's scripts are not loaded",
                )
            }
        }
    }

    /// Open a file of the level in the system's program for it (script, log).
    pub fn open_level_file_externally(&mut self, what: &str) {
        let path = match what {
            "log" => self.install().map(|i| {
                kfx_formats::testutil::resolve_ci(i, "keeperfx.log")
                    .unwrap_or_else(|| i.join("keeperfx.log"))
            }),
            ext => self.doc.as_ref().map(|d| {
                level_bin::find_file(&d.doc.folder, d.doc.level, ext)
                    .unwrap_or_else(|| d.doc.folder.join(level_bin::file_name(d.doc.level, ext)))
            }),
        };
        let Some(p) = path else { return };
        if !p.exists() {
            self.status = trf(
                "kfx_editor.p_does_not_exist_yet",
                "{p} does not exist yet",
                &[("p", &p.display())],
            );
            return;
        }
        if let Err(e) = crate::files::open_path(&p) {
            self.status = e;
        }
    }

    /// Lock or unlock the door on the hovered slab (Space). Returns whether
    /// there was a door.
    pub fn toggle_door_lock(&mut self) -> bool {
        let Some((x, y)) = self
            .hover
            .as_ref()
            .map(|h| (h.slab.0 as i32, h.slab.1 as i32))
        else {
            return false;
        };
        let Some(doc) = self.doc.as_mut() else {
            return false;
        };
        let Some(i) = doc.doc.door_on_slab(x, y) else {
            self.status = trd(
                "kfx_editor.space_toggles_the_lock_of_the_door_under_the_poi",
                "Space toggles the lock of the door under the pointer: no door there",
            );
            return false;
        };
        let mut t = doc.doc.things.items[i].clone();
        let locked = t.door_locked.unwrap_or(0) > 0;
        t.door_locked = Some(!locked as i64);
        doc.end_stroke();
        let (_, rf) = doc.op(
            &if locked {
                trd("kfx_editor.unlock_door", "Unlock door")
            } else {
                trd("kfx_editor.lock_door", "Lock door")
            },
            |d, g| ((), d.update_thing(g, i, t)),
        );
        self.after_change(rf);
        self.status = trf(
            "kfx_editor.door_at_slab_x_y_locked",
            "door at slab ({x},{y}) {locked}",
            &[
                ("x", &x),
                ("y", &y),
                (
                    "locked",
                    &(if locked {
                        trd("kfx_editor.unlocked", "unlocked")
                    } else {
                        trd("kfx_editor.locked_ecb3", "locked")
                    }),
                ),
            ],
        );
        true
    }

    pub fn open_doodad_dialog(&mut self) {
        let Some(d) = &self.doc else { return };
        let set = d.doodads.clone();
        let seed_text = set.seed.map(|s| s.to_string()).unwrap_or_default();
        self.doodad_dialog = Some(DoodadDialog { set, seed_text });
    }

    /// Scatter the doodads of `set` over the whole map (one undo step); the rules become the
    /// map's and are saved with it.
    pub fn apply_doodads(&mut self, set: Doodads) -> Result<usize, String> {
        let d = self.doc.as_mut().ok_or("no map")?;
        let problems = doodads::problems(&set, &d.gd);
        let (n, rf) = d.apply_doodads(set);
        self.after_change(rf);
        self.status = if problems.is_empty() {
            trf(
                "kfx_editor.n_doodads_scattered",
                "{n} doodads scattered",
                &[("n", &n)],
            )
        } else {
            trf(
                "kfx_editor.n_doodads_scattered_skipped_problems",
                "{n} doodads scattered; skipped: {problems}",
                &[("n", &n), ("problems", &problems.join("; "))],
            )
        };
        Ok(n)
    }

    pub(super) fn doodad_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.doodad_dialog.take() else {
            return;
        };
        let Some(gd) = self.doc.as_ref().map(|d| d.gd.clone()) else {
            return;
        };
        let mut open = true;
        let mut apply = false;
        let mut close = false;
        let slabs: Vec<String> = (0..gd.slab_count())
            .filter_map(|k| gd.slab_name(k).map(str::to_string))
            .collect();
        let objects: Vec<String> = gd
            .object_names()
            .into_iter()
            .map(|(_, n)| n)
            .filter(|n| !n.is_empty())
            .collect();
        let effects: Vec<String> = gd
            .effect_generator_names()
            .into_iter()
            .map(|(_, n)| n)
            .filter(|n| !n.is_empty())
            .collect();
        egui::Window::new(trd("kfx_editor.random_doodads", "Random doodads"))
            .open(&mut open)
            .default_width(720.0)
            .max_height(ctx.content_rect().height() * 0.8)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(trd(
                        "kfx_editor.things_scattered_on_slabs_of_a_kind_each_subtile",
                        "Things scattered on slabs of a kind: each subtile of such a \
                        slab gets the thing with the chance given. They are attached to their \
                        slab, \
                        so placing or regenerating the slab scatters them again (seeded: the same \
                        slab gets the same doodads). Saved with the map.",
                    ))
                    .color(TEXT_DIM),
                );
                ui.horizontal(|ui| {
                    ui.label(trd("kfx_editor.seed", "Seed"));
                    ui.add(
                        egui::TextEdit::singleline(&mut dlg.seed_text)
                            .desired_width(140.0)
                            .hint_text(trd(
                                "kfx_editor.from_the_level_number",
                                "from the level number",
                            )),
                    )
                    .on_hover_text(trd(
                        "kfx_editor.empty_derived_from_the_level_number",
                        "Empty: derived from the level number",
                    ));
                    if ui
                        .button(trd("kfx_editor.new_seed_def3", "🎲 New seed"))
                        .on_hover_text(trd("kfx_editor.pick_a_random_seed", "Pick a random seed"))
                        .clicked()
                    {
                        let t = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_nanos() as u64)
                            .unwrap_or(1);
                        dlg.seed_text = (t % 1_000_000_007).to_string();
                    }
                });
                ui.separator();
                let mut remove = None;
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        egui::Grid::new("doodad-rules")
                            .striped(true)
                            .num_columns(7)
                            .show(ui, |ui| {
                                for h in [
                                    trd("kfx_editor.on", "On"),
                                    trd("kfx_editor.slab_kind", "Slab kind"),
                                    trd("kfx_editor.thing", "Thing"),
                                    trd("kfx_editor.name", "Name"),
                                    trd("kfx_editor.chance", "Chance %"),
                                    trd("kfx_editor.orientation", "Orientation"),
                                    String::new(),
                                ] {
                                    ui.label(RichText::new(h).color(GOLD));
                                }
                                ui.end_row();
                                for (i, r) in dlg.set.rules.iter_mut().enumerate() {
                                    ui.checkbox(&mut r.enabled, "").on_hover_text(trd(
                                        "kfx_editor.use_this_rule",
                                        "Use this rule",
                                    ));
                                    egui::ComboBox::from_id_salt(("dd-slab", i))
                                        .selected_text(r.slab.clone())
                                        .width(130.0)
                                        .show_ui(ui, |ui| {
                                            for s in &slabs {
                                                ui.selectable_value(&mut r.slab, s.clone(), s);
                                            }
                                        });
                                    egui::ComboBox::from_id_salt(("dd-kind", i))
                                        .selected_text(match r.kind {
                                            DoodadKind::Object => {
                                                trd("kfx_editor.object", "Object")
                                            }
                                            DoodadKind::Effect => trd(
                                                "kfx_editor.effect_generator",
                                                "Effect generator",
                                            ),
                                            DoodadKind::Light => trd("kfx_editor.light", "Light"),
                                        })
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut r.kind,
                                                DoodadKind::Object,
                                                trd("kfx_editor.object", "Object"),
                                            );
                                            ui.selectable_value(
                                                &mut r.kind,
                                                DoodadKind::Effect,
                                                trd(
                                                    "kfx_editor.effect_generator",
                                                    "Effect generator",
                                                ),
                                            );
                                            ui.selectable_value(
                                                &mut r.kind,
                                                DoodadKind::Light,
                                                trd("kfx_editor.light", "Light"),
                                            );
                                        });
                                    match r.kind {
                                        DoodadKind::Light => {
                                            ui.horizontal(|ui| {
                                                ui.add(
                                                    egui::DragValue::new(&mut r.light_range)
                                                        .range(0.0..=32.0)
                                                        .speed(0.1)
                                                        .prefix(trd(
                                                            "kfx_editor.range_4c16",
                                                            "range ",
                                                        )),
                                                );
                                                ui.add(
                                                    egui::DragValue::new(&mut r.light_intensity)
                                                        .range(0..=255)
                                                        .prefix(trd(
                                                            "kfx_editor.intensity_967e",
                                                            "intensity ",
                                                        )),
                                                );
                                            });
                                        }
                                        k => {
                                            let list = if k == DoodadKind::Object {
                                                &objects
                                            } else {
                                                &effects
                                            };
                                            egui::ComboBox::from_id_salt(("dd-model", i))
                                                .selected_text(if r.model.is_empty() {
                                                    trd("kfx_editor.choose", "(choose)")
                                                } else {
                                                    r.model.clone()
                                                })
                                                .width(170.0)
                                                .show_ui(ui, |ui| {
                                                    for n in list {
                                                        ui.selectable_value(
                                                            &mut r.model,
                                                            n.clone(),
                                                            n,
                                                        );
                                                    }
                                                });
                                        }
                                    }
                                    ui.add(
                                        egui::DragValue::new(&mut r.chance)
                                            .range(0.0..=100.0)
                                            .speed(0.05)
                                            .max_decimals(2)
                                            .suffix("%"),
                                    )
                                    .on_hover_text(trd(
                                        "kfx_editor.chance_per_subtile_nine_per_slab",
                                        "Chance per subtile (nine per slab)",
                                    ));
                                    let mut random = r.orientation.is_none();
                                    ui.horizontal(|ui| {
                                        if ui
                                            .checkbox(
                                                &mut random,
                                                trd("kfx_editor.random_7a82", "random"),
                                            )
                                            .changed()
                                        {
                                            r.orientation = if random { None } else { Some(0) };
                                        }
                                        if let Some(o) = r.orientation.as_mut() {
                                            ui.add(
                                                egui::DragValue::new(o).range(0..=2047).speed(8.0),
                                            );
                                        }
                                    });
                                    if ui
                                        .button("🗑")
                                        .on_hover_text(trd(
                                            "kfx_editor.remove_this_rule",
                                            "Remove this rule",
                                        ))
                                        .clicked()
                                    {
                                        remove = Some(i);
                                    }
                                    ui.end_row();
                                }
                            });
                    });
                if let Some(i) = remove {
                    dlg.set.rules.remove(i);
                }
                ui.horizontal(|ui| {
                    if ui
                        .button(trd("kfx_editor.add_rule", "➕ Add rule"))
                        .clicked()
                    {
                        dlg.set.rules.push(DoodadRule {
                            model: objects.first().cloned().unwrap_or_default(),
                            ..Default::default()
                        });
                    }
                    if ui
                        .button(trd("kfx_editor.default_effects", "Default effects"))
                        .on_hover_text(trd(
                            "kfx_editor.water_and_lava_effect_generators_at_0_25",
                            "Water and lava effect generators at 0.25%",
                        ))
                        .clicked()
                    {
                        dlg.set
                            .rules
                            .extend(Doodads::liquid_effect_defaults(&gd).rules);
                    }
                });
                dlg.set.seed = dlg.seed_text.trim().parse().ok();
                for p in doodads::problems(&dlg.set, &gd) {
                    ui.colored_label(Color32::from_rgb(240, 190, 80), format!("⚠ {p}"));
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .button(
                            RichText::new(trd(
                                "kfx_editor.apply_to_the_whole_map",
                                "Apply to the whole map",
                            ))
                            .strong(),
                        )
                        .on_hover_text(trd(
                            "kfx_editor.remove_the_doodads_of_these_rules_and_scatter_th",
                            "Remove the doodads of these rules and scatter them again (one \
                            undo step)",
                        ))
                        .clicked()
                    {
                        apply = true;
                    }
                    if ui.button(trd("kfx_editor.close", "Close")).clicked() {
                        close = true;
                    }
                });
            });
        if apply {
            if let Err(e) = self.apply_doodads(dlg.set.clone()) {
                self.status = e;
            }
        }
        if open && !close {
            self.doodad_dialog = Some(dlg);
        }
    }
}
