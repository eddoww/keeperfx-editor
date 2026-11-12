//! Writes the English catalogue `data/i18n/en.toml` from the code of the workspace's crates.
//!
//! Usage: `extract_strings [WORKSPACE_ROOT] [OUT_EN_TOML]` (defaults: the workspace of this
//! crate, `data/i18n/en.toml`). A summary per crate is printed.
//!
//! Two kinds of entries:
//!
//! * Live keys: every `trd("key", "English")`, `trf("key", "English", &[..])`,
//!   `trn("key", n, "one", "other", &[..])` and `tx("key", "English")` call, plus the rows of the
//!   tables listed in `kfx_prefab::i18n::scan::TABLES` (the shortcut actions). The text is the
//!   English written in the code; `src` lists where it is used. `tr("key")` calls (no English
//!   in the code) keep the text the previous `en.toml` had.
//! * Drafts (`draft = true`): string literals still passed to egui calls directly
//!   (`ui.label("..")`, `ui.button("..")`, `.on_hover_text("..")`, window titles, menu items,
//!   ...). Keys are `<crate>.<slug of the text>` (up to 48 characters); two different texts with
//!   the same slug get a `_xxxx` suffix (hash of the text). They show what is left to convert;
//!   converting a literal keeps its draft key.
//!
//! One key with two different texts in the code is an error (nothing is written).
//!
//! What it cannot see: texts built or chosen elsewhere (variables, `match` arms and `const`
//! tables of plain `&str`, helper functions that take a `&str` label, struct fields shown
//! later), `format!(..)` texts (counted as "formatted"), `concat!`, strings produced by other
//! macros, error texts shown by other means, and test code (files with `test` in their name and
//! `#[cfg(test)]` modules are skipped). egui calls whose text is a variable are counted as
//! "dynamic": most are data (names, paths), some are texts made elsewhere.
use kfx_prefab::i18n::scan::{self, slug};
use kfx_prefab::i18n::Catalog;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn hash4(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{:04x}", h & 0xffff)
}

#[derive(Default)]
struct Out {
    text: String,
    one: Option<String>,
    other: Option<String>,
    src: Vec<String>,
    draft: bool,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let root = root.canonicalize().unwrap_or(root);
    let out_path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("data/i18n/en.toml"));
    let previous = std::fs::read_to_string(&out_path)
        .ok()
        .and_then(|t| Catalog::parse(&t).ok())
        .unwrap_or_default();
    let mut crates: Vec<_> = std::fs::read_dir(root.join("crates"))
        .expect("crates folder")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("src").is_dir())
        .collect();
    crates.sort();
    let mut entries: BTreeMap<String, Out> = BTreeMap::new();
    let mut errors = vec![];
    let mut drafts: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    println!(
        "{:<18} {:>6} {:>6} {:>7} {:>7} {:>9} {:>8}",
        "crate", "calls", "keys", "drafts", "unique", "formatted", "dynamic"
    );
    let (mut tc, mut td, mut tf, mut ty) = (0, 0, 0, 0);
    for c in crates {
        let name = c.file_name().unwrap().to_string_lossy().to_string();
        let short = name.replace('-', "_");
        let s = scan::scan_dir(&c.join("src"), &root);
        let mut keys = BTreeSet::new();
        for u in &s.uses {
            if name == "kfx-prefab" && u.file.ends_with("i18n.rs") {
                continue; // the doc examples of the runtime itself
            }
            keys.insert(u.key.clone());
            let loc = format!("{}:{}", u.file, u.line);
            let e = entries.entry(u.key.clone()).or_default();
            let (text, one, other) = match u.form {
                "tr" => match previous.entries.get(&u.key) {
                    Some(p) => (p.text.clone(), p.one.clone(), p.other.clone()),
                    None => {
                        errors.push(format!(
                            "{loc}: tr({:?}) has no English text (add it to en.toml by \
                                hand or use trd)",
                            u.key
                        ));
                        continue;
                    }
                },
                _ => (
                    u.text.clone().unwrap_or_default(),
                    u.one.clone(),
                    u.other.clone(),
                ),
            };
            if e.src.is_empty() {
                (e.text, e.one, e.other) = (text, one, other);
            } else if (&e.text, &e.one, &e.other) != (&text, &one, &other) {
                errors.push(format!(
                    "{loc}: key {} has another text at {}",
                    u.key, e.src[0]
                ));
            }
            if !e.src.contains(&loc) {
                e.src.push(loc);
            }
        }
        for u in s
            .unkeyed
            .iter()
            .filter(|u| !u.contains("kfx-prefab/src/i18n"))
        {
            println!("note: {u}: translated call whose key is not a literal");
        }
        let mut unique = BTreeSet::new();
        for d in &s.drafts {
            unique.insert(d.text.clone());
            let loc = format!("{}:{}", d.file, d.line);
            let v = drafts.entry((short.clone(), d.text.clone())).or_default();
            if !v.contains(&loc) {
                v.push(loc);
            }
        }
        println!(
            "{:<18} {:>6} {:>6} {:>7} {:>7} {:>9} {:>8}",
            name,
            s.uses.len(),
            keys.len(),
            s.drafts.len(),
            unique.len(),
            s.formatted,
            s.dynamic
        );
        tc += s.uses.len();
        td += s.drafts.len();
        tf += s.formatted;
        ty += s.dynamic;
    }
    let live = entries.len();
    // drafts: keys that do not depend on the order the files were read in
    let mut suffixed = 0;
    for ((short, text), srcs) in drafts {
        let base = format!("{short}.{}", slug(&text));
        let key = match entries.get(&base) {
            Some(e) if e.text == text => continue, // converted elsewhere under the same key
            Some(_) => {
                suffixed += 1;
                format!("{base}_{}", hash4(&text))
            }
            None => base,
        };
        let e = entries.entry(key).or_default();
        if e.src.is_empty() {
            e.text = text;
            e.draft = true;
        }
        e.src.extend(srcs);
    }
    println!(
        "{:<18} {:>6} {:>6} {:>7} {:>7} {:>9} {:>8}",
        "total",
        tc,
        live,
        td,
        entries.len() - live,
        tf,
        ty
    );
    println!(
        "{suffixed} draft keys needed a hash suffix because their slug was taken by another text"
    );
    if !errors.is_empty() {
        for e in &errors {
            eprintln!("error: {e}");
        }
        eprintln!(
            "{} errors: {} not written",
            errors.len(),
            out_path.display()
        );
        std::process::exit(1);
    }

    let s = kfx_formats::toml_emit::string;
    let mut text = String::new();
    text.push_str(
        "# KeeperFX Editor UI strings, English: the template for translations.\n\
            # GENERATED by `cargo run -p kfx-prefab --example extract_strings` from the code, which holds\n\
            # the English texts (`trd(\"key\", \"English\")`); do not edit by hand. `src` lists where a text is\n\
            # used. `draft = true`: a literal the code does not translate yet (its key may still change).\n\
            [meta]\nlanguage = \"en\"\nname = \"English\"\n\n",
    );
    for (key, e) in &entries {
        text.push_str(&format!("[strings.{}]\n", s(key)));
        if !e.text.is_empty() || e.one.is_none() {
            text.push_str(&format!("text = {}\n", s(&e.text)));
        }
        if let Some(o) = &e.one {
            text.push_str(&format!("one = {}\n", s(o)));
        }
        if let Some(o) = &e.other {
            text.push_str(&format!("other = {}\n", s(o)));
        }
        if e.draft {
            text.push_str("draft = true\n");
        }
        text.push_str(&format!(
            "src = [{}]\n\n",
            e.src.iter().map(|x| s(x)).collect::<Vec<_>>().join(", ")
        ));
    }
    std::fs::write(&out_path, text).expect("write en.toml");
    println!(
        "wrote {} ({} keys: {} used by the code, {} drafts)",
        out_path.display(),
        entries.len(),
        live,
        entries.len() - live
    );
}
