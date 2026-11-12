//! The editor's language (Preferences > Interface > Language, `ui.language` in the settings).
//!
//! The texts of the editor are `kfx_prefab::i18n` calls with the English text in the code
//! (`trd("kfx_editor.save", "Save")`, see that module). English is compiled in; other languages
//! are catalogues `<code>.toml` read from, in this order (a later folder replaces a language of an
//! earlier one):
//!
//! 1. `data/i18n/` next to the executable (the layout of a release archive),
//! 2. `../share/kfx-editor/i18n/` beside the executable's folder (an install under a prefix, e.g.
//!    `/usr/bin/kfx-editor` with `/usr/share/kfx-editor/i18n/`),
//! 3. the first `data/i18n/` in the folders above the executable's, up to four levels (a
//!    development build run from the source tree: `target/debug/kfx-editor`),
//! 4. `i18n/` in the editor's configuration folder (the user's own or newer catalogues).
//!
//! All of them are found relative to the running executable; nothing depends on where the
//! program was built.
//!
//! A key missing from the chosen catalogue shows its English text.
use std::path::PathBuf;

/// The folders catalogues are read from, in loading order (existing ones only).
pub fn catalog_dirs() -> Vec<PathBuf> {
    let mut out = vec![];
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from))
    {
        out.push(exe_dir.join("data").join("i18n"));
        if let Some(prefix) = exe_dir.parent() {
            out.push(prefix.join("share").join("kfx-editor").join("i18n"));
        }
        if let Some(d) = exe_dir
            .ancestors()
            .skip(1)
            .take(4)
            .map(|a| a.join("data").join("i18n"))
            .find(|d| d.is_dir())
        {
            out.push(d);
        }
    }
    if let Some(d) = crate::settings::config_dir() {
        out.push(d.join("i18n"));
    }
    let mut seen: Vec<PathBuf> = vec![];
    out.retain(|d| {
        let c = d.canonicalize().unwrap_or_else(|_| d.clone());
        let keep = d.is_dir() && !seen.contains(&c);
        seen.push(c);
        keep
    });
    out
}

/// Read the catalogues of [`catalog_dirs`] (once per process); returns the language codes.
pub fn load_catalogs() -> Vec<String> {
    static LOADED: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    LOADED
        .get_or_init(|| {
            let mut codes = vec![];
            for d in catalog_dirs() {
                for c in kfx_prefab::i18n::load_dir(&d) {
                    if !codes.contains(&c) {
                        codes.push(c);
                    }
                }
            }
            codes
        })
        .clone()
}

/// `(code, name)` of the languages to choose from, English first.
pub fn languages() -> Vec<(String, String)> {
    load_catalogs();
    let mut v = kfx_prefab::i18n::available_languages();
    v.sort_by_key(|(c, n)| (c != "en", n.to_lowercase()));
    v
}

/// Switch the editor to `code` (empty: English); false when there is no such catalogue (the
/// editor stays in English). Tests switch only their own thread: they run in parallel.
pub fn apply(code: &str) -> bool {
    load_catalogs();
    let code = if code.is_empty() { "en" } else { code };
    #[cfg(not(test))]
    let ok = kfx_prefab::i18n::set_language(code);
    #[cfg(test)]
    let ok = kfx_prefab::i18n::set_thread_language(Some(code));
    if !ok {
        apply("en");
    }
    ok
}

/// Headless action `{"language": "fr"}` (see docs/manual/cli.md).
pub fn run_actions(
    app: &mut crate::app::EditorApp,
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    if let Some(v) = obj.get("language").and_then(|v| v.as_str()) {
        app.settings.ui.language = v.to_string();
        if !apply(v) {
            return Err(format!(
                "no catalogue for language {v:?} (have: {})",
                languages()
                    .into_iter()
                    .map(|l| l.0)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    Ok(())
}
