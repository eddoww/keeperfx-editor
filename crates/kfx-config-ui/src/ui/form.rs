//! The generic, schema-driven config form: section list with search, typed fields with
//! provenance badges, override / revert, docs, inline validation, raw text and diff view.
use super::common::{badge, image_box, layer_color, picker, weak};
use super::Cx;
use crate::cfgedit::LayeredCfg;
use crate::diff::{line_diff, DiffLine};
use crate::schema::{ArgSpec, KeySpec, SectionSpec};
use crate::session::OpenFile;
use crate::tomledit::path_string;
use crate::validate::validate;
use egui::{Color32, RichText, Ui};
use kfx_formats::cfg::{split_tokens, CfgDoc};
use kfx_map::gamedata::parse_number;

/// State of one form view.
#[derive(Default)]
pub struct FormState {
    pub file: String,
    pub section: Option<String>,
    pub search: String,
    pub raw: bool,
    search_cache: Option<(String, u64, String, Vec<String>)>,
    /// Text of the raw editor while the user types an unparsable TOML document.
    raw_toml_buf: Option<String>,
    raw_error: Option<String>,
}

impl FormState {
    pub fn new(file: &str) -> Self {
        FormState {
            file: file.to_string(),
            ..Default::default()
        }
    }
}

/// A field change requested by a control.
enum Act {
    Set(String),
    Revert,
    Pin,
}

/// Does a section name pass a section filter prefix list (empty = all)?
pub fn section_matches(name: &str, prefixes: &[&str]) -> bool {
    prefixes.is_empty()
        || prefixes.iter().any(|p| {
            let n = name.to_ascii_lowercase();
            n == *p
                || (n.starts_with(p)
                    && n[p.len()..].bytes().all(|b| b.is_ascii_digit())
                    && n.len() > p.len())
        })
}

/// The section list + form of a cfg/toml file. `prefixes` limits the sections (e.g.
/// `["object"]`); `preview` draws something above the form for the selected section.
pub fn file_form(
    ui: &mut Ui,
    cx: &mut Cx,
    st: &mut FormState,
    prefixes: &[&str],
    preview: Option<&mut dyn FnMut(&mut Ui, &mut Cx, &str)>,
) {
    let file = st.file.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new(&file).strong().monospace());
        let tl = cx.s.target_kind().clone();
        ui.label("Editing at");
        badge(ui, &tl.badge(), layer_color(&tl));
        if let Some(p) = cx.s.file(&file).target_path() {
            ui.label(weak(p.display().to_string()));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.toggle_value(&mut st.raw, "Raw / diff");
            if ui
                .small_button("Open file")
                .on_hover_text("Open the target file")
                .clicked()
            {
                if let Some(p) = cx.s.file(&file).target_path().cloned() {
                    cx.s.push_event(crate::session::ConfigEvent::RequestOpenFile { path: p });
                }
            }
        });
    });
    ui.separator();
    if st.raw {
        raw_view(ui, cx, st);
        return;
    }
    let is_cfg = matches!(cx.s.file(&file), OpenFile::Cfg(_));
    egui::Panel::left(("sections", file.as_str()))
        .resizable(true)
        .default_size(230.0)
        .show(ui, |ui| {
            section_list(ui, cx, st, prefixes);
        });
    egui::CentralPanel::default().show(ui, |ui| {
        let Some(sec) = st.section.clone() else {
            ui.label(weak("Pick a section"));
            return;
        };
        egui::ScrollArea::vertical()
            .id_salt(("form", &file, &sec))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if let Some(p) = preview {
                    p(ui, cx, &sec);
                }
                if is_cfg {
                    cfg_section_form(ui, cx, &file, &sec, &st.search);
                } else {
                    toml_section_form(ui, cx, &file, &sec);
                }
            });
    });
}

fn section_label(cx: &mut Cx, file: &str, sec: &str) -> String {
    let name = match cx.s.file(file) {
        OpenFile::Cfg(c) => c.at_target(sec, "Name").map(|p| p.value),
        OpenFile::Toml(t) => {
            let mut p: Vec<String> = sec.split('.').map(str::to_string).collect();
            p.push("Name".into());
            t.at_target(&p)
                .and_then(|v| v.value.as_str().map(str::to_string))
                .or_else(|| {
                    *p.last_mut().unwrap() = "name".into();
                    t.at_target(&p)
                        .and_then(|v| v.value.as_str().map(str::to_string))
                })
        }
        _ => None,
    };
    match name {
        Some(n) if !n.is_empty() => format!("{sec}  {n}"),
        _ => sec.to_string(),
    }
}

fn section_names(cx: &mut Cx, file: &str) -> Vec<String> {
    match cx.s.file(file) {
        OpenFile::Cfg(c) => c.section_names(),
        OpenFile::Toml(t) => t
            .section_paths()
            .into_iter()
            .map(|p| path_string(&p))
            .collect(),
        _ => vec![],
    }
}

fn section_overridden(cx: &mut Cx, file: &str, sec: &str) -> bool {
    match cx.s.file(file) {
        OpenFile::Cfg(c) => c.target_has_section(sec),
        OpenFile::Toml(t) => {
            let p: Vec<String> = sec.split('.').map(str::to_string).collect();
            crate::tomledit::get_path(t.target_table(), &p).is_some()
                || (p.is_empty() && !t.target_table().is_empty())
        }
        _ => false,
    }
}

fn search_hits(cx: &mut Cx, file: &str, names: &[String], q: &str) -> Vec<String> {
    let q = q.to_ascii_lowercase();
    let mut out = vec![];
    match cx.s.file(file) {
        OpenFile::Cfg(c) => {
            for s in names {
                let hit = s.to_ascii_lowercase().contains(&q)
                    || c.keys(s).iter().any(|k| {
                        k.to_ascii_lowercase().contains(&q)
                            || c.at_target(s, k)
                                .is_some_and(|p| p.value.to_ascii_lowercase().contains(&q))
                    });
                if hit {
                    out.push(s.clone());
                }
            }
        }
        OpenFile::Toml(t) => {
            for s in names {
                let p: Vec<String> = if s.is_empty() {
                    vec![]
                } else {
                    s.split('.').map(str::to_string).collect()
                };
                let hit = s.to_ascii_lowercase().contains(&q)
                    || t.keys(&p).iter().any(|k| {
                        let mut kp = p.clone();
                        kp.push(k.clone());
                        k.to_ascii_lowercase().contains(&q)
                            || t.at_target(&kp).is_some_and(|v| {
                                v.value.to_string().to_ascii_lowercase().contains(&q)
                            })
                    });
                if hit {
                    out.push(s.clone());
                }
            }
        }
        _ => {}
    }
    out
}

fn section_list(ui: &mut Ui, cx: &mut Cx, st: &mut FormState, prefixes: &[&str]) {
    let file = st.file.clone();
    ui.add(egui::TextEdit::singleline(&mut st.search).hint_text("search names, keys, values"));
    let mut names: Vec<String> = section_names(cx, &file)
        .into_iter()
        .filter(|n| section_matches(n, prefixes))
        .collect();
    if !st.search.trim().is_empty() {
        let key = (st.search.clone(), cx.s.generation, file.clone());
        let fresh = st
            .search_cache
            .as_ref()
            .is_some_and(|c| c.0 == key.0 && c.1 == key.1 && c.2 == key.2);
        if !fresh {
            let hits = search_hits(cx, &file, &names, st.search.trim());
            st.search_cache = Some((key.0, key.1, key.2, hits));
        }
        names = st.search_cache.as_ref().unwrap().3.clone();
    }
    if st.section.is_none() {
        st.section = names.first().cloned();
    }
    ui.label(weak(format!("{} sections", names.len())));
    egui::ScrollArea::vertical()
        .id_salt(("seclist", &file))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for n in &names {
                let label = section_label(cx, &file, n);
                let ov = section_overridden(cx, &file, n);
                let text = if ov {
                    RichText::new(format!("* {label}")).color(layer_color(cx.s.target_kind()))
                } else {
                    RichText::new(label)
                };
                let sel = st.section.as_deref() == Some(n.as_str());
                if ui.selectable_label(sel, text).clicked() {
                    st.section = Some(n.clone());
                }
            }
        });
}

// ------------------------------------------------------------------------------ cfg form

/// Keys to show for a section: schema keys first (all of them, set or not), then keys found in
/// the files that the schema does not know.
fn form_keys(
    c: &LayeredCfg,
    spec: Option<&SectionSpec>,
    sec: &str,
) -> Vec<(String, Option<KeySpec>)> {
    let present = c.keys(sec);
    let mut out: Vec<(String, Option<KeySpec>)> = vec![];
    if let Some(sp) = spec {
        for k in &sp.keys {
            let name = present
                .iter()
                .find(|p| p.eq_ignore_ascii_case(&k.name))
                .cloned()
                .unwrap_or_else(|| pretty_key(&k.name));
            out.push((name, Some(k.clone())));
        }
    }
    for p in present {
        if !out.iter().any(|(n, _)| n.eq_ignore_ascii_case(&p)) {
            out.push((p, None));
        }
    }
    out
}

/// `POTOFGOLDHOLDS` -> `PotOfGoldHolds` is not recoverable; keep the table spelling.
fn pretty_key(k: &str) -> String {
    k.to_string()
}

/// The form of one cfg section.
pub fn cfg_section_form(ui: &mut Ui, cx: &mut Cx, file: &str, sec: &str, search: &str) {
    let spec = cx.s.schema.file(file).and_then(|f| f.section(sec)).cloned();
    let keys = {
        let Some(c) = cx.s.cfg(file) else { return };
        form_keys(c, spec.as_ref(), sec)
    };
    ui.horizontal(|ui| {
        ui.heading(sec);
        if let Some(sp) = &spec {
            if let Some(src) = &sp.source {
                ui.label(weak(src.clone()));
            }
        }
        let has = cx.s.cfg(file).is_some_and(|c| c.target_has_section(sec));
        if has
            && ui
                .small_button("Revert section")
                .on_hover_text("Remove this section from the target layer")
                .clicked()
        {
            if cx.s.cfg(file).is_some_and(|c| c.revert_section(sec)) {
                cx.s.touched(file);
            }
        }
        if numbered(sec).is_some()
            && ui
                .small_button("Duplicate as new")
                .on_hover_text("Copy every value into the next free number at the target layer")
                .clicked()
        {
            duplicate_section(cx, file, sec);
        }
    });
    if let Some(d) = spec.as_ref().and_then(|s| s.doc.clone()) {
        ui.label(weak(d));
    }
    if spec.as_ref().is_some_and(|s| s.replaces_whole_block()) {
        ui.label(
            RichText::new(
                "A later layer's block replaces this whole list: it is written complete.",
            )
            .color(Color32::from_rgb(220, 180, 80)),
        );
    }
    let q = search.trim().to_ascii_lowercase();
    egui::Grid::new(("grid", file, sec))
        .num_columns(3)
        .min_col_width(60.0)
        .striped(true)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            for (key, ks) in keys {
                let hit = !q.is_empty() && key.to_ascii_lowercase().contains(&q);
                field_row(ui, cx, file, sec, &key, ks.as_ref(), hit);
                ui.end_row();
            }
        });
}

fn numbered(sec: &str) -> Option<(String, usize)> {
    let i = sec.find(|c: char| c.is_ascii_digit())?;
    let (b, n) = sec.split_at(i);
    if b.is_empty() || !n.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((b.to_string(), n.parse().ok()?))
}

fn duplicate_section(cx: &mut Cx, file: &str, sec: &str) {
    let Some((base, _)) = numbered(sec) else {
        return;
    };
    let Some(c) = cx.s.cfg(file) else { return };
    let used: Vec<usize> = c
        .section_names()
        .iter()
        .filter_map(|s| {
            numbered(s)
                .filter(|(b, _)| b.eq_ignore_ascii_case(&base))
                .map(|(_, n)| n)
        })
        .collect();
    let next = used.iter().max().map_or(0, |m| m + 1);
    let new = format!("{base}{next}");
    let vals: Vec<(String, String)> = c
        .keys(sec)
        .into_iter()
        .filter_map(|k| c.at_target(sec, &k).map(|p| (k, p.value)))
        .collect();
    c.target.add_section(&new);
    for (k, v) in vals {
        let v = if k.eq_ignore_ascii_case("Name") {
            format!("{}_COPY", v.split_whitespace().next().unwrap_or("NEW"))
        } else {
            v
        };
        c.target.add(&new, &k, &v);
    }
    cx.s.touched(file);
}

/// One `key = value` row: label with doc, typed controls, provenance and actions, issues.
fn field_row(
    ui: &mut Ui,
    cx: &mut Cx,
    file: &str,
    sec: &str,
    key: &str,
    ks: Option<&KeySpec>,
    hit: bool,
) {
    let (at, inherited, target_v, shadow, tlayer, comment) = {
        let c = cx.s.cfg(file).unwrap();
        (
            c.at_target(sec, key),
            c.inherited(sec, key),
            c.target_value(sec, key),
            c.shadowed_by(sec, key),
            c.target_layer,
            c.base_comment(sec, key),
        )
    };
    // label
    let mut label = RichText::new(key).monospace();
    if hit {
        label = label.background_color(Color32::from_rgb(90, 80, 20));
    }
    if at.is_none() {
        label = label.color(Color32::GRAY);
    }
    let mut tip = String::new();
    if let Some(k) = ks {
        if let Some(d) = &k.doc {
            tip.push_str(d);
            tip.push('\n');
        }
        for (i, a) in k.args.iter().enumerate() {
            tip.push_str(&format!("\narg {}: {}", i + 1, describe_arg(a)));
        }
        if let Some(s) = &k.source {
            tip.push_str(&format!(
                "\n\n{s} ({})",
                k.confidence.as_deref().unwrap_or("?")
            ));
        }
    } else {
        tip.push_str("Not in the schema: kept as written and editable as text.");
    }
    if let Some(c) = comment {
        tip.push_str(&format!("\n\nbase file comment:\n{c}"));
    }
    ui.label(label).on_hover_text(tip);

    // controls
    let mut act: Option<Act> = None;
    let value = at.as_ref().map(|p| p.value.clone());
    ui.vertical(|ui| {
        ui.set_min_width(460.0);
        ui.horizontal_wrapped(|ui| match &value {
            Some(v) => {
                let mut buf = v.clone();
                if value_controls(ui, cx, ks, &mut buf, (file, sec, key)) && buf != *v {
                    act = Some(Act::Set(buf));
                }
            }
            None => {
                let def = ks
                    .and_then(|k| k.args.first())
                    .and_then(|a| a.default.clone())
                    .map(|d| match d {
                        serde_json::Value::String(s) => s,
                        o => o.to_string(),
                    });
                ui.label(weak(match &def {
                    Some(d) => format!("not set (game default {d})"),
                    None => "not set".into(),
                }));
                if ui.small_button("Set").clicked() {
                    act = Some(Act::Set(def.unwrap_or_else(|| "0".into())));
                }
            }
        });
        if let (Some(k), Some(v)) = (ks, &value) {
            let schema = cx.s.schema;
            let s = &mut *cx.s;
            let issues = validate(k, v, schema, &mut |r| s.names(r));
            for i in issues {
                let col = if i.error {
                    Color32::from_rgb(230, 90, 80)
                } else {
                    Color32::from_rgb(220, 180, 80)
                };
                ui.label(RichText::new(i.msg).small().color(col));
            }
        }
    });

    // provenance + actions
    ui.horizontal(|ui| {
        if let Some(p) = &at {
            let kind = cx.s.layers.layers[p.layer].kind.clone();
            badge(ui, &kind.badge(), layer_color(&kind));
        }
        if target_v.is_some() {
            let inh = inherited
                .as_ref()
                .map(|p| p.value.clone())
                .unwrap_or_else(|| "(unset)".into());
            ui.label(
                RichText::new("overridden")
                    .small()
                    .color(layer_color(&cx.s.layers.layers[tlayer].kind)),
            )
            .on_hover_text(format!("Inherited: {inh}"));
            if ui
                .small_button("Revert")
                .on_hover_text(format!("Revert to inherited ({inh})"))
                .clicked()
            {
                act = Some(Act::Revert);
            }
        } else if inherited.is_some()
            && ui
                .small_button("Override here")
                .on_hover_text("Write the inherited value explicitly at the target layer")
                .clicked()
        {
            act = Some(Act::Pin);
        }
        if let Some(l) = shadow {
            let kind = cx.s.layers.layers[l].kind.clone();
            ui.label(
                RichText::new(format!("! {} overrides this", kind.badge()))
                    .small()
                    .color(Color32::from_rgb(220, 180, 80)),
            );
        }
    });

    match act {
        Some(Act::Set(v)) => {
            cx.s.set_cfg(file, sec, key, &v);
        }
        Some(Act::Revert) => {
            cx.s.revert_cfg(file, sec, key);
        }
        Some(Act::Pin) => {
            if let (Some(c), Some(p)) = (cx.s.cfg(file), inherited) {
                c.target.set(sec, key, &p.value);
                cx.s.touched(file);
            }
        }
        None => {}
    }
}

fn describe_arg(a: &ArgSpec) -> String {
    let mut s = a.ty.clone();
    if let Some(l) = &a.label {
        s = format!("{l}: {s}");
    }
    if let Some(r) = a.ref_kind() {
        s.push_str(&format!(" ({r})"));
    } else if let Some(e) = &a.enum_table {
        s.push_str(&format!(" ({e})"));
    }
    match (&a.min_symbol, &a.max_symbol, a.min, a.max) {
        (_, _, Some(mn), Some(mx)) => s.push_str(&format!(" [{mn} .. {mx}]")),
        (_, _, Some(mn), None) => s.push_str(&format!(" [>= {mn}]")),
        _ => {}
    }
    if let Some(d) = &a.default {
        s.push_str(&format!(" default {d}"));
    }
    if a.variadic {
        s.push_str(", repeats");
    }
    s
}

/// Options of an enum / ref argument.
fn arg_options(cx: &mut Cx, a: &ArgSpec, arg0: Option<&str>) -> Vec<(i64, String)> {
    if let (Some(a0), false) = (arg0, a.enum_by_arg0.is_empty()) {
        if let Some((_, t)) = a
            .enum_by_arg0
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(a0))
        {
            if let Some(r) = crate::schema::table_ref_kind(t) {
                return cx.s.names(&r);
            }
            return enum_options(cx, t);
        }
    }
    if let Some(r) = a.ref_kind() {
        let v = cx.s.names(&r);
        if !v.is_empty() {
            return v;
        }
    }
    a.enum_table
        .as_deref()
        .map(|t| enum_options(cx, t))
        .unwrap_or_default()
}

fn enum_options(cx: &Cx, t: &str) -> Vec<(i64, String)> {
    cx.s.schema
        .enum_table(t)
        .map(|v| {
            v.iter()
                .map(|e| (e.value.unwrap_or(-1), e.name.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Icon thumbnail for a reference option.
fn ref_icon(ui: &mut Ui, cx: &mut Cx, kind: &str, name: &str) -> Option<egui::TextureHandle> {
    let ctx = ui.ctx().clone();
    match kind {
        "creature" => {
            let key = format!("creatrs/{}.cfg", name.to_ascii_lowercase());
            let qs =
                cx.s.cfg(&key)
                    .and_then(|c| c.at_target("sprites", "QuerySymbol"))
                    .map(|p| p.value)?;
            let assets = &*cx.assets;
            cx.tex.get(&ctx, &format!("portrait:{qs}"), || {
                assets.as_ref()?.portrait(&qs)
            })
        }
        "object" | "trap" | "door" => {
            let (file, base) = match kind {
                "object" => ("objects.cfg", "object"),
                "trap" => ("trapdoor.cfg", "trap"),
                _ => ("trapdoor.cfg", "door"),
            };
            let id =
                cx.s.names(kind)
                    .into_iter()
                    .find(|(_, n)| n.eq_ignore_ascii_case(name))?
                    .0;
            let sec = format!("{base}{id}");
            let c = cx.s.cfg(file)?;
            if kind == "object" {
                let a = c.at_target(&sec, "AnimationID")?.value;
                cx.tex.anim(&ctx, cx.assets, &a, 0).into_iter().next()
            } else {
                let v = c.at_target(&sec, "SymbolSprites")?.value;
                let assets = &*cx.assets;
                cx.tex
                    .get(&ctx, &format!("panel:{v}"), || assets.as_ref()?.panel(&v))
            }
        }
        "room" | "power" | "spell" => {
            let (file, base) = match kind {
                "room" => ("terrain.cfg", "room"),
                "power" => ("magic.cfg", "power"),
                _ => ("magic.cfg", "spell"),
            };
            let id =
                cx.s.names(kind)
                    .into_iter()
                    .find(|(_, n)| n.eq_ignore_ascii_case(name))?
                    .0;
            let v =
                cx.s.cfg(file)?
                    .at_target(&format!("{base}{id}"), "SymbolSprites")?
                    .value;
            let assets = &*cx.assets;
            cx.tex
                .get(&ctx, &format!("panel:{v}"), || assets.as_ref()?.panel(&v))
        }
        _ => None,
    }
}

/// Typed controls for a value; returns whether `buf` changed.
pub fn value_controls(
    ui: &mut Ui,
    cx: &mut Cx,
    ks: Option<&KeySpec>,
    buf: &mut String,
    salt: (&str, &str, &str),
) -> bool {
    let Some(k) = ks.filter(|k| !k.args.is_empty()) else {
        return ui
            .add(egui::TextEdit::singleline(buf).desired_width(320.0))
            .changed();
    };
    let first = &k.args[0];
    // flags on the whole line or variadic flags: checkboxes
    if matches!(first.ty.as_str(), "flags" | "longflags")
        && (k.whole_line || first.variadic)
        && k.args.len() == 1
    {
        return flag_controls(ui, cx, first, buf);
    }
    if k.whole_line
        || (k.args.len() == 1 && first.variadic && !matches!(first.ty.as_str(), "enum" | "ref"))
    {
        return ui
            .add(egui::TextEdit::singleline(buf).desired_width(320.0))
            .changed();
    }
    let mut toks = split_tokens(buf);
    let n_args = k.args.len();
    let variadic = k.args.last().is_some_and(|a| a.variadic);
    let count = if variadic {
        toks.len().max(n_args)
    } else {
        n_args.max(toks.len())
    };
    let mut changed = false;
    let arg0 = toks.first().cloned();
    for i in 0..count {
        let a = k
            .args
            .get(i)
            .or(if variadic { k.args.last() } else { None });
        if i >= toks.len() {
            if a.is_none() || variadic && i >= n_args {
                break;
            }
            toks.push(String::new());
        }
        let t = &mut toks[i];
        let id = (salt.0, salt.1, salt.2, i);
        changed |= match a {
            Some(a) => token_control(ui, cx, a, t, id, arg0.as_deref()),
            None => ui
                .add(egui::TextEdit::singleline(t).desired_width(90.0))
                .changed(),
        };
    }
    if variadic {
        if ui.small_button("+").on_hover_text("Add a value").clicked() {
            toks.push(
                k.args
                    .last()
                    .and_then(|a| a.default.as_ref())
                    .map(|d| d.to_string().trim_matches('"').to_string())
                    .unwrap_or_else(|| "0".into()),
            );
            changed = true;
        }
        if toks.len() > n_args
            && ui
                .small_button("-")
                .on_hover_text("Remove the last value")
                .clicked()
        {
            toks.pop();
            changed = true;
        }
    }
    if changed {
        while toks.last().is_some_and(|t| t.is_empty()) {
            toks.pop();
        }
        *buf = toks.join(" ");
    }
    changed
}

fn flag_controls(ui: &mut Ui, cx: &mut Cx, a: &ArgSpec, buf: &mut String) -> bool {
    let opts = arg_options(cx, a, None);
    if opts.is_empty() {
        return ui
            .add(egui::TextEdit::singleline(buf).desired_width(320.0))
            .changed();
    }
    let toks = split_tokens(buf);
    let num = toks
        .first()
        .and_then(|t| parse_number(t))
        .filter(|_| toks.len() == 1);
    let mut set: Vec<String> = vec![];
    for (v, n) in &opts {
        let on = match num {
            Some(m) => *v > 0 && m & v == *v,
            None => toks.iter().any(|t| t.eq_ignore_ascii_case(n)),
        };
        if on {
            set.push(n.clone());
        }
    }
    let mut changed = false;
    for (_, n) in &opts {
        let mut on = set.iter().any(|s| s == n);
        if ui.checkbox(&mut on, RichText::new(n).small()).changed() {
            changed = true;
            if on {
                set.push(n.clone());
            } else {
                set.retain(|s| s != n);
            }
        }
    }
    if changed {
        // keep the table order
        let ordered: Vec<String> = opts
            .iter()
            .filter(|(_, n)| set.contains(n))
            .map(|(_, n)| n.clone())
            .collect();
        *buf = ordered.join(" ");
    }
    changed
}

fn token_control(
    ui: &mut Ui,
    cx: &mut Cx,
    a: &ArgSpec,
    t: &mut String,
    id: (&str, &str, &str, usize),
    arg0: Option<&str>,
) -> bool {
    let ctx = ui.ctx().clone();
    match a.ty.as_str() {
        "int" | "coord" | "stringid" => match parse_number(t) {
            Some(mut n) if !(a.ty == "int" && a.enum_table.is_some()) => {
                let mut d = egui::DragValue::new(&mut n).speed(1.0);
                if let (Some(mn), Some(mx)) = (a.min_i64(), a.max_i64()) {
                    if mx > mn {
                        d = d.range(mn..=mx);
                    }
                }
                let r = ui.add(d);
                if r.changed() {
                    *t = n.to_string();
                    return true;
                }
                false
            }
            _ => ui
                .add(egui::TextEdit::singleline(t).desired_width(90.0))
                .changed(),
        },
        "bool" => {
            let mut on = parse_number(t).is_some_and(|n| n != 0) || t.eq_ignore_ascii_case("true");
            if ui.checkbox(&mut on, "").changed() {
                *t = if on { "1".into() } else { "0".into() };
                return true;
            }
            false
        }
        "enum" | "name" | "ref" | "flags" | "longflags" => {
            let opts = arg_options(cx, a, arg0);
            if opts.is_empty() {
                return ui
                    .add(egui::TextEdit::singleline(t).desired_width(120.0))
                    .changed();
            }
            let kind = if a.enum_by_arg0.is_empty() {
                a.ref_kind().unwrap_or_default()
            } else {
                arg0.and_then(|a0| {
                    a.enum_by_arg0
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(a0))
                })
                .and_then(|(_, tb)| crate::schema::table_ref_kind(tb))
                .unwrap_or_default()
            };
            picker(ui, id, t, &opts, 180.0, |ui, name, _| {
                ref_icon(ui, cx, &kind, name)
            })
        }
        "anim" => {
            let r = ui
                .add(egui::TextEdit::singleline(t).desired_width(90.0))
                .changed();
            let frames = cx.tex.anim(&ctx, cx.assets, t, 0);
            if !frames.is_empty() {
                let i = (cx.time * 8.0) as usize % frames.len();
                image_box(ui, Some(&frames[i]), egui::vec2(40.0, 40.0))
                    .on_hover_text(format!("{} frames", frames.len()));
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(120));
            }
            r
        }
        "icon" => {
            let r = ui
                .add(egui::TextEdit::singleline(t).desired_width(90.0))
                .changed();
            let gui1 =
                id.2.eq_ignore_ascii_case("QuerySymbol") || id.2.eq_ignore_ascii_case("HandSymbol");
            let key = format!("icon:{}:{t}", if gui1 { 1 } else { 2 });
            let v = t.clone();
            let assets = &*cx.assets;
            let tex = cx.tex.get(&ctx, &key, || {
                let a = assets.as_ref()?;
                if gui1 {
                    a.portrait(&v)
                } else {
                    a.panel(&v)
                }
            });
            if let Some(tx) = tex {
                image_box(ui, Some(&tx), egui::vec2(40.0, 40.0));
            }
            r
        }
        _ => ui
            .add(egui::TextEdit::singleline(t).desired_width(120.0))
            .changed(),
    }
}

// ------------------------------------------------------------------------------ toml form

fn toml_section_form(ui: &mut Ui, cx: &mut Cx, file: &str, sec: &str) {
    let path: Vec<String> = if sec.is_empty() {
        vec![]
    } else {
        sec.split('.').map(str::to_string).collect()
    };
    let spec =
        cx.s.schema
            .file(file)
            .and_then(|f| f.section(path.first().map(String::as_str).unwrap_or("")))
            .cloned();
    ui.heading(if sec.is_empty() { "(top level)" } else { sec });
    let keys = cx.s.toml(file).map(|t| t.keys(&path)).unwrap_or_default();
    egui::Grid::new(("tgrid", file, sec))
        .num_columns(3)
        .striped(true)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            for k in keys {
                let mut kp = path.clone();
                kp.push(k.clone());
                let ks = spec.as_ref().and_then(|s| s.key(&k)).cloned();
                let (at, inh, tv, shadow) = {
                    let t = cx.s.toml(file).unwrap();
                    (
                        t.at_target(&kp),
                        t.inherited(&kp),
                        t.target_value(&kp).cloned(),
                        t.shadowed_by(&kp),
                    )
                };
                let tip = ks
                    .as_ref()
                    .and_then(|k| k.doc.clone())
                    .unwrap_or_else(|| "no documentation".into());
                ui.label(RichText::new(&k).monospace()).on_hover_text(tip);
                let mut newv: Option<toml::Value> = None;
                let mut revert = false;
                if let Some(p) = &at {
                    newv = toml_value_control(ui, &p.value, (file, sec, &k));
                } else {
                    ui.label(weak("not set"));
                }
                ui.horizontal(|ui| {
                    if let Some(p) = &at {
                        let kind = cx.s.layers.layers[p.layer].kind.clone();
                        badge(ui, &kind.badge(), layer_color(&kind));
                    }
                    if tv.is_some() {
                        let i = inh
                            .as_ref()
                            .map(|p| p.value.to_string())
                            .unwrap_or_else(|| "(unset)".into());
                        if ui
                            .small_button("Revert")
                            .on_hover_text(format!("Revert to inherited ({i})"))
                            .clicked()
                        {
                            revert = true;
                        }
                    }
                    if let Some(l) = shadow {
                        ui.label(
                            RichText::new(format!(
                                "! {} overrides this",
                                cx.s.layers.layers[l].kind.badge()
                            ))
                            .small(),
                        );
                    }
                });
                if let Some(v) = newv {
                    if cx.s.toml(file).is_some_and(|t| t.set(&kp, v)) {
                        cx.s.touched(file);
                    }
                }
                if revert && cx.s.toml(file).is_some_and(|t| t.revert(&kp)) {
                    cx.s.touched(file);
                }
                ui.end_row();
            }
        });
}

/// Controls for a TOML value; `Some(new)` when edited.
pub fn toml_value_control(
    ui: &mut Ui,
    v: &toml::Value,
    salt: (&str, &str, &str),
) -> Option<toml::Value> {
    match v {
        toml::Value::Integer(i) => {
            let mut n = *i;
            ui.add(egui::DragValue::new(&mut n))
                .changed()
                .then_some(toml::Value::Integer(n))
        }
        toml::Value::Boolean(b) => {
            let mut x = *b;
            ui.checkbox(&mut x, "")
                .changed()
                .then_some(toml::Value::Boolean(x))
        }
        toml::Value::String(s) => {
            let mut x = s.clone();
            ui.add(egui::TextEdit::singleline(&mut x).desired_width(240.0))
                .changed()
                .then_some(toml::Value::String(x))
        }
        toml::Value::Array(a) if a.iter().all(|x| x.is_integer()) && a.len() <= 16 => {
            let mut out = a.clone();
            let mut ch = false;
            ui.horizontal_wrapped(|ui| {
                for x in out.iter_mut() {
                    let mut n = x.as_integer().unwrap_or(0);
                    if ui.add(egui::DragValue::new(&mut n)).changed() {
                        *x = toml::Value::Integer(n);
                        ch = true;
                    }
                }
            });
            ch.then_some(toml::Value::Array(out))
        }
        other => {
            let id = ui.make_persistent_id(("tomlraw", salt.0, salt.1, salt.2));
            let mut text = ui
                .data(|d| d.get_temp::<String>(id))
                .unwrap_or_else(|| other.to_string());
            let r = ui.add(
                egui::TextEdit::multiline(&mut text)
                    .code_editor()
                    .desired_rows(1)
                    .desired_width(360.0),
            );
            let mut out = None;
            if r.changed() {
                ui.data_mut(|d| d.insert_temp(id, text.clone()));
                if let Ok(t) = format!("v = {text}").parse::<toml::Table>() {
                    out = t.get("v").cloned();
                }
            }
            if r.lost_focus() {
                ui.data_mut(|d| d.remove::<String>(id));
            }
            out
        }
    }
}

// ------------------------------------------------------------------------------ raw view

/// Raw text of the target layer's file (editable for plain cfg / toml files), the diff
/// against the saved file, and every override with its inherited value.
pub fn raw_view(ui: &mut Ui, cx: &mut Cx, st: &mut FormState) {
    let file = st.file.clone();
    let (text, orig, editable) = {
        let f = cx.s.file(&file);
        (
            f.target_text(),
            f.original_text(),
            matches!(f, OpenFile::Cfg(_) | OpenFile::Toml(_)),
        )
    };
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        ui.label(RichText::new("Target layer file (as it will be saved)").strong());
        if !editable {
            ui.label(weak(
                "Edited through the specialised editor; read only here",
            ));
        }
        egui::ScrollArea::vertical()
            .id_salt(("raw", &file))
            .max_height(ui.available_height() - 20.0)
            .show(ui, |ui| {
                let mut buf = st.raw_toml_buf.clone().unwrap_or_else(|| text.clone());
                let r = ui.add_enabled(
                    editable,
                    egui::TextEdit::multiline(&mut buf)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(24),
                );
                if r.changed() {
                    match cx.s.file(&file) {
                        OpenFile::Cfg(c) => {
                            c.target = CfgDoc::parse(&buf);
                            cx.s.touched(&file);
                        }
                        OpenFile::Toml(t) => match buf.parse::<toml_edit::DocumentMut>() {
                            Ok(d) => {
                                t.target = d;
                                t.resync();
                                st.raw_toml_buf = None;
                                st.raw_error = None;
                                cx.s.touched(&file);
                            }
                            Err(e) => {
                                st.raw_toml_buf = Some(buf.clone());
                                st.raw_error = Some(e.to_string());
                            }
                        },
                        _ => {}
                    }
                }
            });
        if let Some(e) = &st.raw_error {
            ui.label(RichText::new(e).color(Color32::from_rgb(230, 90, 80)));
        }
        let ui = &mut cols[1];
        ui.label(RichText::new("Changes since the file was opened / saved").strong());
        egui::ScrollArea::vertical()
            .id_salt(("diff", &file))
            .max_height(ui.available_height() * 0.5)
            .show(ui, |ui| {
                let d = line_diff(&orig, &text);
                if d.iter().all(|l| matches!(l, DiffLine::Same(_))) {
                    ui.label(weak("No unsaved changes"));
                }
                for l in d {
                    let (t, c) = match l {
                        DiffLine::Same(s) => (format!("  {s}"), Color32::GRAY),
                        DiffLine::Added(s) => (format!("+ {s}"), Color32::from_rgb(110, 200, 110)),
                        DiffLine::Removed(s) => {
                            (format!("- {s}"), Color32::from_rgb(230, 110, 100))
                        }
                    };
                    ui.label(RichText::new(t).monospace().color(c));
                }
            });
        ui.separator();
        ui.label(RichText::new("Values this layer overrides (inherited -> here)").strong());
        egui::ScrollArea::vertical()
            .id_salt(("ovr", &file))
            .show(ui, |ui| {
                if let OpenFile::Cfg(c) = cx.s.file(&file) {
                    for (s, k, inh, v) in c.overrides() {
                        ui.label(
                            RichText::new(format!(
                                "[{s}] {k}: {} -> {v}",
                                inh.unwrap_or_else(|| "(unset)".into())
                            ))
                            .monospace(),
                        );
                    }
                } else {
                    ui.label(weak("See the diff above"));
                }
            });
    });
}
