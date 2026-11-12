//! Game text in every language of the install through the UI text path (egui
//! layout with the editor's fonts, then an egui_kittest render), and the editor contexts'
//! system font fallback.
use crate::ui_tests::{gpu_available, shots_dir};
use eframe::egui;
use egui_kittest::Harness;
use kfx_formats::campaign::Campaign;
use kfx_formats::strings::{Lang, StringTable};
use kfx_formats::testutil;
use std::collections::BTreeSet;
use std::path::Path;

/// Every string table of the install: the 17 GUI tables (`fxdata/gtext_<lang>.dat`) and the
/// `[strings]` tables of every campaign / map pack `.cfg`, as (label, language, strings).
fn string_tables(install: &Path) -> Vec<(String, Lang, Vec<String>)> {
    let mut out = vec![];
    let mut seen: BTreeSet<std::path::PathBuf> = BTreeSet::new();
    let fx = testutil::resolve_ci(install, "fxdata").unwrap();
    let mut gui: Vec<_> = std::fs::read_dir(&fx)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_ascii_lowercase())
        .collect();
    gui.sort();
    for n in gui {
        let Some(code) = n
            .strip_prefix("gtext_")
            .and_then(|r| r.strip_suffix(".dat"))
        else {
            continue;
        };
        let lang = Lang::from_code(code).unwrap_or_else(|| panic!("{n}: unknown language"));
        seen.insert(fx.join(&n));
        let t = StringTable::load(&fx.join(&n), lang).unwrap();
        out.push((format!("fxdata/{n}"), lang, t.strings()));
    }
    for group in ["campgns", "levels", "multiplayer"] {
        let Some(dir) = testutil::resolve_ci(install, group) else {
            continue;
        };
        let mut cfgs: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cfg")))
            .collect();
        cfgs.sort();
        for cfg in cfgs {
            let Ok(c) = Campaign::load(&cfg) else {
                continue;
            };
            for (code, file) in c.strings() {
                let (Some(lang), Some(p)) = (
                    Lang::from_code(&code),
                    testutil::resolve_ci(install, &file.trim().replace('\\', "/")),
                ) else {
                    continue;
                };
                // many packs point at the same files (often the GUI tables)
                if !seen.insert(p.clone()) {
                    continue;
                }
                let Ok(t) = StringTable::load(&p, lang) else {
                    continue;
                };
                out.push((format!("{} [{code}]", file.trim()), lang, t.strings()));
            }
        }
    }
    out
}

/// The fonts of an editor context: egui's, the editor theme, the system fallback.
fn editor_fonts(ctx: &egui::Context) {
    crate::theme::apply(ctx);
    kfx_fonts::install(ctx);
}

/// Every string table of the install (all 17 GUI languages, including Chinese, Japanese and
/// Korean, and every campaign's strings) is laid out with the editor's fonts and rendered in an
/// egui_kittest harness without a panic. A table whose characters all have a glyph is rendered
/// with the harness's panicking missing-glyph policy, which proves it; the others (scripts this
/// machine has no font for) with the application's policy, boxes, and are reported.
#[test]
fn ui_every_string_table_renders() {
    let Some(install) = testutil::install_dir() else {
        return;
    };
    let tables = string_tables(&install);
    let gui_langs: BTreeSet<Lang> = tables
        .iter()
        .filter(|t| t.0.starts_with("fxdata/"))
        .map(|t| t.1)
        .collect();
    assert_eq!(gui_langs.len(), 17, "GUI languages: {gui_langs:?}");
    // 1. layout of every string with the editor's fonts (plain context, application policy)
    let ctx = egui::Context::default();
    editor_fonts(&ctx);
    let mut out = ctx.run_ui(Default::default(), |_| {});
    out.textures_delta.clear();
    let font = egui::FontId::proportional(14.0);
    let mut uncovered: Vec<(String, usize, usize, String)> = vec![];
    let mut total = 0usize;
    for (label, _, strings) in &tables {
        let mut out = ctx.run_ui(Default::default(), |ui| {
            for s in strings {
                let g = ui
                    .painter()
                    .layout_no_wrap(s.clone(), font.clone(), egui::Color32::WHITE);
                assert!(g.size().x.is_finite());
            }
        });
        out.textures_delta.clear();
        total += strings.len();
        let chars: BTreeSet<char> = strings
            .iter()
            .flat_map(|s| s.chars())
            .filter(|c| !c.is_control())
            .collect();
        let missing: Vec<char> = chars
            .iter()
            .copied()
            .filter(|&c| !ctx.fonts_mut(|f| f.has_glyph(&font, c)))
            .collect();
        if !missing.is_empty() {
            uncovered.push((
                label.clone(),
                missing.len(),
                chars.len(),
                missing.iter().take(12).collect(),
            ));
        }
    }
    eprintln!(
        "{} tables, {total} strings laid out; system fonts used: {:?}",
        tables.len(),
        kfx_fonts::discovered(&ctx)
    );
    for (l, m, n, ex) in &uncovered {
        eprintln!(
            "  {l}: {m} of {n} characters have no font on this system (shown as boxes), e.g. {ex}"
        );
    }
    // the scripts egui's own fonts cover never need a system font (private-use code points
    // are the game font's own symbols: no font has them, they show as boxes)
    for (label, lang, strings) in tables.iter().filter(|t| t.0.starts_with("fxdata/")) {
        if matches!(
            lang,
            Lang::English
                | Lang::French
                | Lang::German
                | Lang::Italian
                | Lang::Spanish
                | Lang::Swedish
                | Lang::Polish
                | Lang::Dutch
                | Lang::Czech
                | Lang::Russian
                | Lang::Ukrainian
                | Lang::Portuguese
                | Lang::Latin
        ) {
            let missing: BTreeSet<char> = strings
                .iter()
                .flat_map(|s| s.chars())
                .filter(|&c| {
                    !c.is_control()
                        && !('\u{e000}'..='\u{f8ff}').contains(&c)
                        && !ctx.fonts_mut(|f| f.has_glyph(&font, c))
                })
                .collect();
            assert!(missing.is_empty(), "{label}: no glyph for {missing:?}");
        }
    }
    // 2. an egui_kittest render of every table: all its distinct characters and its first strings
    if !gpu_available() {
        return;
    }
    let dir = shots_dir();
    let _ = std::fs::create_dir_all(&dir);
    for (label, lang, strings) in &tables {
        let chars: String = strings
            .iter()
            .flat_map(|s| s.chars())
            .filter(|c| !c.is_control())
            .collect::<BTreeSet<char>>()
            .into_iter()
            .collect();
        let sample: Vec<String> = strings
            .iter()
            .filter(|s| !s.trim().is_empty())
            .take(24)
            .cloned()
            .collect();
        let complete = !uncovered.iter().any(|u| &u.0 == label);
        let mut b = Harness::builder()
            .with_context_setup(editor_fonts)
            .with_size(egui::vec2(1000.0, 700.0))
            .with_pixels_per_point(1.0)
            .with_accessibility_check(false)
            .wgpu();
        if !complete {
            b = b.allow_missing_glyphs();
        }
        let title = label.clone();
        let mut h = b.build_ui(move |ui| {
            ui.heading(&title);
            ui.add(egui::Label::new(&chars).wrap());
            ui.separator();
            for s in &sample {
                ui.add(egui::Label::new(s).truncate());
            }
        });
        h.run();
        let img = h.render().unwrap_or_else(|e| panic!("{label}: {e}"));
        if matches!(
            lang,
            Lang::ChineseInt | Lang::Japanese | Lang::Korean | Lang::Russian
        ) && label.starts_with("fxdata/")
        {
            let _ = img.save(dir.join(format!("fonts_{}.png", lang.lower())));
        }
    }
}
