//! Runtime localisation of the editor's own texts.
//!
//! **The English text lives in the code**, next to a stable key:
//!
//! ```ignore
//! ui.button(trd("kfx_editor.save", "Save"));
//! self.status = trf("kfx_editor.saved_path", "Saved {path}", &[("path", &p.display())]);
//! let s = trn("kfx_editor.n_files", n, "{n} file", "{n} files", &[]);
//! const TABS: [(Tab, Tx); 2] = [(Tab::A, tx("kfx_editor.slabs", "Slabs")), (Tab::B, tx("kfx_editor.things", "Things"))];
//! ```
//!
//! [`trd`] / [`trf`] / [`trn`] / [`Tx`] return the active language's text for the key, else the
//! English text written in the call. So the code reads like plain English, English output is
//! exactly the literal (whatever the catalogues say), and a missing translation never shows a
//! key. [`tx`] is the `const` form for tables, translated when shown. [`tr`] (key only) looks
//! the English up in the compiled-in catalogue; the editor does not use it.
//!
//! `data/i18n/en.toml` is **generated** from the code by
//! `cargo run -p kfx-prefab --example extract_strings`: every key with its English text and
//! where it is used, plus `draft = true` entries for literals still passed to egui directly.
//! It is the template for translators; a test checks that it matches the code (keys, texts,
//! placeholders). After changing an English text in the code, run the extractor; when the
//! meaning changes, also change the key so old translations are not used for the new text.
//!
//! Catalogues are TOML files `<lang>.toml`:
//!
//! ```toml
//! [meta]
//! language = "fr"            # code of the catalogue
//! name = "Francais"          # shown in language pickers
//!
//! [strings]
//! "editor.open" = "Ouvrir..."                         # plain text
//! "editor.undo_n" = { one = "Annuler {n} action", other = "Annuler {n} actions" }  # plural
//! [strings."editor.save"]                               # the form the extractor writes:
//! text = "Enregistrer"
//! src = ["crates/kfx-editor/src/app.rs:100"]          # where the English text is used
//! ```
//!
//! Arguments are `{name}` placeholders ([`trf`]); plurals have `one` (n == 1) and `other`
//! ([`trn`]). That is all: no genders, no further plural categories. The English catalogue is
//! compiled in; others are loaded with [`register_catalog`] / [`load_dir`]. [`scan`] is the
//! source scanner of the extractor and of the key tests.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

pub mod scan;

/// The English catalogue shipped in the repository.
pub const ENGLISH: &str = include_str!("../../../data/i18n/en.toml");

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub text: String,
    pub one: Option<String>,
    pub other: Option<String>,
    pub src: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    pub code: String,
    pub name: String,
    pub entries: BTreeMap<String, Entry>,
}

impl Catalog {
    pub fn parse(text: &str) -> Result<Catalog, String> {
        let t: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
        let meta = t.get("meta").and_then(|v| v.as_table());
        let code = meta
            .and_then(|m| m.get("language"))
            .and_then(|v| v.as_str())
            .ok_or("[meta] language is missing")?
            .to_string();
        let name = meta
            .and_then(|m| m.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or(&code)
            .to_string();
        let mut entries = BTreeMap::new();
        if let Some(s) = t.get("strings").and_then(|v| v.as_table()) {
            for (k, v) in s {
                let mut e = Entry::default();
                match v {
                    toml::Value::String(s) => e.text = s.clone(),
                    toml::Value::Table(t) => {
                        e.text = t
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        e.one = t.get("one").and_then(|v| v.as_str()).map(str::to_string);
                        e.other = t.get("other").and_then(|v| v.as_str()).map(str::to_string);
                        if let Some(toml::Value::Array(a)) = t.get("src") {
                            e.src = a
                                .iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect();
                        }
                    }
                    _ => return Err(format!("strings.{k}: expected a string or a table")),
                }
                entries.insert(k.clone(), e);
            }
        }
        Ok(Catalog {
            code,
            name,
            entries,
        })
    }
}

struct State {
    catalogs: BTreeMap<String, Catalog>,
    active: String,
}

fn state() -> &'static RwLock<State> {
    static S: OnceLock<RwLock<State>> = OnceLock::new();
    S.get_or_init(|| {
        let mut catalogs = BTreeMap::new();
        if let Ok(c) = Catalog::parse(ENGLISH) {
            catalogs.insert(c.code.clone(), c);
        }
        RwLock::new(State {
            catalogs,
            active: "en".into(),
        })
    })
}

/// Add (or replace) a catalogue; returns its language code.
pub fn register_catalog(text: &str) -> Result<String, String> {
    let c = Catalog::parse(text)?;
    let code = c.code.clone();
    if let Ok(mut s) = state().write() {
        s.catalogs.insert(code.clone(), c);
    }
    Ok(code)
}

/// Register every `*.toml` of a folder; returns the codes loaded (files that do not parse are
/// skipped).
pub fn load_dir(dir: &std::path::Path) -> Vec<String> {
    let mut out = vec![];
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut files: Vec<_> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    for f in files {
        if let Ok(text) = std::fs::read_to_string(&f) {
            if let Ok(code) = register_catalog(&text) {
                out.push(code);
            }
        }
    }
    out
}

thread_local! {
    static THREAD_LANGUAGE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Select the language of the calling thread only, over the process-wide one (`None` returns to
/// it); false when no catalogue has that code. For tests, which run in parallel threads of one
/// process and must not change each other's texts.
pub fn set_thread_language(code: Option<&str>) -> bool {
    if let Some(c) = code {
        if !state().read().is_ok_and(|s| s.catalogs.contains_key(c)) {
            return false;
        }
    }
    THREAD_LANGUAGE.with(|t| *t.borrow_mut() = code.map(str::to_string));
    true
}

fn thread_language() -> Option<String> {
    THREAD_LANGUAGE.with(|t| t.borrow().clone())
}

/// Select the active language; false when no catalogue has that code.
pub fn set_language(code: &str) -> bool {
    let Ok(mut s) = state().write() else {
        return false;
    };
    if s.catalogs.contains_key(code) {
        s.active = code.to_string();
        true
    } else {
        false
    }
}

/// The active language of this thread.
pub fn language() -> String {
    thread_language().unwrap_or_else(|| {
        state()
            .read()
            .map(|s| s.active.clone())
            .unwrap_or_else(|_| "en".into())
    })
}

/// `(code, name)` of the registered catalogues.
pub fn available_languages() -> Vec<(String, String)> {
    state()
        .read()
        .map(|s| {
            s.catalogs
                .values()
                .map(|c| (c.code.clone(), c.name.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// The entry of `key` in the active language, then (with `english`) in the English catalogue.
fn lookup(s: &State, key: &str, english: bool) -> Option<Entry> {
    let active = thread_language().unwrap_or_else(|| s.active.clone());
    let codes: &[&str] = if english {
        &[active.as_str(), "en"]
    } else if active == "en" {
        &[]
    } else {
        &[active.as_str()]
    };
    for &code in codes {
        if let Some(e) = s.catalogs.get(code).and_then(|c| c.entries.get(key)) {
            if !e.text.is_empty() || e.one.is_some() || e.other.is_some() {
                return Some(e.clone());
            }
        }
    }
    None
}

/// The text of `key`: active language, else the English catalogue, else `key` itself.
pub fn tr(key: &str) -> String {
    match state().read().ok().and_then(|s| lookup(&s, key, true)) {
        Some(e) if !e.text.is_empty() => e.text,
        Some(e) => e.other.or(e.one).unwrap_or_else(|| key.to_string()),
        None => key.to_string(),
    }
}

/// The text of `key` in the active language, else `english` (the English text written in the
/// code, which is what English shows).
pub fn trd(key: &str, english: &str) -> String {
    match state().read().ok().and_then(|s| lookup(&s, key, false)) {
        Some(e) if !e.text.is_empty() => e.text,
        Some(e) => e.other.or(e.one).unwrap_or_else(|| english.to_string()),
        None => english.to_string(),
    }
}

/// Replace `{name}` placeholders.
pub fn fill(text: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = text.to_string();
    for (k, v) in args {
        out = out.replace(&format!("{{{k}}}"), &v.to_string());
    }
    out
}

/// [`trd`] with `{name}` arguments.
pub fn trf(key: &str, english: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    fill(&trd(key, english), args)
}

/// Plural: `one` for n == 1, else `other`; `{n}` is filled in automatically.
pub fn trn(
    key: &str,
    n: i64,
    one_default: &str,
    other_default: &str,
    args: &[(&str, &dyn std::fmt::Display)],
) -> String {
    let e = state().read().ok().and_then(|s| lookup(&s, key, false));
    let text = match (&e, n == 1) {
        (Some(e), true) => e
            .one
            .clone()
            .or_else(|| (!e.text.is_empty()).then(|| e.text.clone())),
        (Some(e), false) => e
            .other
            .clone()
            .or_else(|| (!e.text.is_empty()).then(|| e.text.clone())),
        (None, _) => None,
    }
    .unwrap_or_else(|| {
        if n == 1 {
            one_default.to_string()
        } else {
            other_default.to_string()
        }
    });
    let mut all: Vec<(&str, &dyn std::fmt::Display)> = vec![("n", &n)];
    all.extend_from_slice(args);
    fill(&text, &all)
}

/// A text of a `const` table: key and English, translated when shown ([`Tx::text`], `Display`,
/// or directly as an egui text).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Tx {
    pub key: &'static str,
    pub en: &'static str,
}

/// `const` form of [`trd`] for tables.
pub const fn tx(key: &'static str, en: &'static str) -> Tx {
    Tx { key, en }
}

impl Tx {
    /// The text in the active language.
    pub fn text(&self) -> String {
        trd(self.key, self.en)
    }
}

impl std::fmt::Display for Tx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text())
    }
}

impl From<Tx> for String {
    fn from(t: Tx) -> String {
        t.text()
    }
}

impl From<Tx> for egui::WidgetText {
    fn from(t: Tx) -> egui::WidgetText {
        t.text().into()
    }
}

impl From<Tx> for egui::RichText {
    fn from(t: Tx) -> egui::RichText {
        egui::RichText::new(t.text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_args_and_plurals() {
        register_catalog(
            "[meta]\nlanguage = \"xx\"\nname = \"Test\"\n[strings]\n\
                \"t.hello\" = \"Salut {who}\"\n\"t.items\" = { one = \"{n} chose\", other \
                = \"{n} choses\" }\n[strings.\"t.draft\"]\ntext = \"Brouillon\"\nsrc = \
                [\"a.rs:1\"]\n",
        )
        .unwrap();
        register_catalog(
            "[meta]\nlanguage = \"en\"\nname = \"English\"\n[strings]\n\
                \"t.hello\" = \"Hello {who}\"\n\"t.only_en\" = \"Only English\"\n",
        )
        .unwrap();
        assert!(set_language("xx"));
        assert_eq!(trf("t.hello", "Hello", &[("who", &"toi")]), "Salut toi");
        assert_eq!(tr("t.draft"), "Brouillon");
        assert_eq!(tr("t.only_en"), "Only English", "falls back to English");
        assert_eq!(tr("t.nowhere"), "t.nowhere", "then to the key");
        assert_eq!(trd("t.nowhere", "Default text"), "Default text");
        assert_eq!(trn("t.items", 1, "{n} thing", "{n} things", &[]), "1 chose");
        assert_eq!(
            trn("t.items", 5, "{n} thing", "{n} things", &[]),
            "5 choses"
        );
        assert_eq!(
            trn("t.missing", 2, "{n} thing", "{n} things", &[]),
            "2 things"
        );
        assert_eq!(
            trd("t.only_en", "Code text"),
            "Code text",
            "the code's English wins over the English catalogue"
        );
        assert_eq!(tx("t.hello", "Hi {who}").text(), "Salut {who}");
        assert!(!set_language("zz"));
        // a thread's own language
        assert!(set_thread_language(Some("en")));
        assert_eq!(trd("t.hello", "Hi"), "Hi");
        assert!(
            std::thread::spawn(|| trd("t.hello", "Hi") == "Salut {who}")
                .join()
                .unwrap(),
            "other threads keep the process language"
        );
        assert!(set_thread_language(None));
        assert!(!set_thread_language(Some("zz")));
        set_language("en");
        assert_eq!(tr("t.hello"), "Hello {who}");
        assert_eq!(trd("t.hello", "Hi {who}"), "Hi {who}");
    }
}
