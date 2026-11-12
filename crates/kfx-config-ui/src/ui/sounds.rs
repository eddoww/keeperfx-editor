//! Sounds override editor (`sounds.cfg`, `src/config_sounds.c`): the `[sounds]` name -> id /
//! custom file mappings with their layer, and adding new mappings (`NAME = id|file [count]
//! [STACK=mode[:max]]`, or `id = file [count] [ALIAS]`). Playback comes from the host's
//! [`super::SoundHost`] (a browser beside the mappings; "Use" fills the value to add).
use super::common::{badge, layer_color, weak};
use super::form::cfg_section_form;
use super::Cx;
use egui::{Color32, RichText, Ui};

const FILE: &str = "sounds.cfg";

#[derive(Default)]
pub struct SoundState {
    pub filter: String,
    pub new_key: String,
    pub new_value: String,
    pub only_overrides: bool,
    pub section: String,
}

pub fn show(ui: &mut Ui, cx: &mut Cx, st: &mut SoundState) {
    if st.section.is_empty() {
        st.section = "sounds".into();
    }
    if let Some(host) = cx.sound.as_deref_mut() {
        let mut pick = None;
        egui::Panel::right("sounds-browser")
            .resizable(true)
            .default_size(560.0)
            .show(ui, |ui| {
                ui.label(RichText::new("Sound browser").strong());
                pick = host.browser(ui, super::SoundPlace::SoundsTab);
            });
        if let Some(p) = pick {
            st.new_value = p.cfg_value();
            if st.new_key.trim().is_empty() && p.id_count.is_none() {
                st.new_key = p
                    .value
                    .to_ascii_uppercase()
                    .replace(['.', '/', '\\', ' '], "_");
            }
        }
    }
    let sections: Vec<String> =
        cx.s.cfg(FILE)
            .map(|c| c.section_names())
            .unwrap_or_default();
    ui.horizontal(|ui| {
        for s in &sections {
            if ui
                .selectable_label(st.section.eq_ignore_ascii_case(s), s)
                .clicked()
            {
                st.section = s.clone();
            }
        }
    });
    ui.separator();
    if !st.section.eq_ignore_ascii_case("sounds") {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                cfg_section_form(ui, cx, FILE, &st.section.clone(), "")
            });
        return;
    }
    ui.horizontal(|ui| {
        ui.label("Add / change mapping");
        ui.add(
            egui::TextEdit::singleline(&mut st.new_key)
                .hint_text("NAME or sound ID")
                .desired_width(160.0),
        );
        ui.label("=");
        let w = (ui.available_width() - 50.0).clamp(120.0, 320.0);
        ui.add(
            egui::TextEdit::singleline(&mut st.new_value)
                .hint_text("ID or custom/file.wav [count] [ALIAS] [STACK=duck:4]")
                .desired_width(w),
        );
        let ok = !st.new_key.trim().is_empty()
            && !st.new_value.trim().is_empty()
            && !st.new_key.contains(char::is_whitespace);
        if ui.add_enabled(ok, egui::Button::new("Set")).clicked() {
            cx.s.set_cfg(FILE, "sounds", st.new_key.trim(), st.new_value.trim());
            st.new_key.clear();
            st.new_value.clear();
        }
    });
    ui.label(weak(
        "A file path (WAV, OGG, MP3, FLAC) is relative to the game \
            folder; count looks for name2, name3… variants (see the comments of \
            fxdata/sounds.cfg).",
    ));
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut st.filter)
                .hint_text("filter")
                .desired_width(160.0),
        );
        ui.checkbox(&mut st.only_overrides, "Only this layer's entries");
    });
    let rows: Vec<(String, Option<crate::cfgedit::Provenance>, bool)> = match cx.s.cfg(FILE) {
        Some(c) => c
            .keys("sounds")
            .into_iter()
            .map(|k| {
                (
                    k.clone(),
                    c.at_target("sounds", &k),
                    c.target_value("sounds", &k).is_some(),
                )
            })
            .collect(),
        None => vec![],
    };
    let f = st.filter.to_ascii_lowercase();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("sounds")
                .striped(true)
                .num_columns(4)
                .show(ui, |ui| {
                    for (k, p, here) in rows {
                        if st.only_overrides && !here {
                            continue;
                        }
                        let v = p.as_ref().map(|p| p.value.clone()).unwrap_or_default();
                        if !f.is_empty()
                            && !k.to_ascii_lowercase().contains(&f)
                            && !v.to_ascii_lowercase().contains(&f)
                        {
                            continue;
                        }
                        ui.label(RichText::new(&k).monospace());
                        let mut buf = v.clone();
                        let custom = buf
                            .split_whitespace()
                            .next()
                            .is_some_and(|t| t.contains('.') || t.contains('/'));
                        let r = ui.add(egui::TextEdit::singleline(&mut buf).desired_width(260.0));
                        if r.changed() {
                            cx.s.set_cfg(FILE, "sounds", &k, &buf);
                        }
                        ui.horizontal(|ui| {
                            if let Some(p) = &p {
                                let kind = cx.s.layers.layers[p.layer].kind.clone();
                                badge(ui, &kind.badge(), layer_color(&kind));
                            }
                            if custom {
                                ui.label(
                                    RichText::new("custom file")
                                        .small()
                                        .color(Color32::from_rgb(110, 200, 110)),
                                );
                            }
                        });
                        if here {
                            if ui
                                .small_button("Revert")
                                .on_hover_text("Revert to inherited")
                                .clicked()
                            {
                                cx.s.revert_cfg(FILE, "sounds", &k);
                            }
                        } else {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                });
        });
}
