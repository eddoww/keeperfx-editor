//! Every snippet, with its placeholder defaults filled in, must validate without errors or
//! warnings (an unknown creature literal in Lua is only a warning).
use kfx_script::lua::{self, ApiIndex, LuaDoc};
use kfx_script::{validate, ConstantProvider, Document, InstallProvider, NullProvider, Severity};
use kfx_script_ui::snippets::{lua_validation_text, LIBRARY};

/// The KeeperFX install named by `KFX_INSTALL`; without it the snippets are only checked
/// without the install's constants.
fn install() -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from(std::env::var_os("KFX_INSTALL")?);
    p.join("levels").exists().then_some(p)
}

fn dk_errors(text: &str, host: &dyn ConstantProvider) -> Vec<String> {
    let doc = Document::parse(text.as_bytes());
    validate(&doc, host)
        .diagnostics
        .iter()
        .filter(|d| d.severity != Severity::Info)
        .map(|d| format!("line {}: {} ({})", d.line, d.message, d.code))
        .collect()
}

fn lua_errors(text: &str, host: &dyn ConstantProvider) -> Vec<String> {
    let doc = LuaDoc::parse(text);
    lua::diagnostics(&doc, &ApiIndex::stock(), host)
        .iter()
        .filter(|d| d.severity != Severity::Info)
        .map(|d| format!("line {}: {} ({})", d.line, d.message, d.code))
        .collect()
}

#[test]
fn dk_snippets_have_no_errors() {
    let inst = install();
    let provider = inst.as_ref().map(|i| InstallProvider::new(i));
    let mut checked = 0;
    for s in LIBRARY {
        let Some(plain) = s.plain("dk-script") else {
            continue;
        };
        // a fragment is validated as it would sit in a script (after LEVEL_VERSION)
        let text = format!("LEVEL_VERSION(1)\n{plain}\n");
        let errs = dk_errors(&text, &NullProvider);
        assert!(
            errs.is_empty(),
            "{} (no constants):\n{text}\n{errs:#?}",
            s.id
        );
        if let Some(p) = &provider {
            let errs = dk_errors(&text, p);
            assert!(
                errs.is_empty(),
                "{} (install constants):\n{text}\n{errs:#?}",
                s.id
            );
        }
        checked += 1;
    }
    assert!(checked >= 7, "{checked}");
}

#[test]
fn lua_snippets_have_no_errors() {
    let inst = install();
    let provider = inst.as_ref().map(|i| InstallProvider::new(i));
    let mut checked = 0;
    for s in LIBRARY {
        let Some(text) = lua_validation_text(s) else {
            continue;
        };
        let errs = lua_errors(&text, &NullProvider);
        assert!(
            errs.is_empty(),
            "{} (no constants):\n{text}\n{errs:#?}",
            s.id
        );
        if let Some(p) = &provider {
            let errs = lua_errors(&text, p);
            assert!(
                errs.is_empty(),
                "{} (install constants):\n{text}\n{errs:#?}",
                s.id
            );
        }
        checked += 1;
    }
    assert!(checked >= 7, "{checked}");
}

#[test]
fn snippets_also_work_together_in_one_file() {
    // all GameStart snippets in one OnGameStart body, all top-level ones after it
    let mut body = String::new();
    let mut top = String::new();
    for s in LIBRARY {
        if let Some((t, place)) = s.lua {
            let plain = kfx_script_ui::textops::snippet_defaults(t);
            match place {
                kfx_script_ui::LuaPlace::GameStart => {
                    body.push_str(&format!("\t-- {}\n{}\n", s.id, plain.replace('\n', "\n\t")))
                }
                kfx_script_ui::LuaPlace::Top => top.push_str(&format!("{plain}\n")),
            }
        }
    }
    let text = format!(
        "function OnGameStart()\n{body}\nend\n{}",
        top.replace("function OnGameStart()", "function OnGameStartUnused()")
    );
    let errs = lua_errors(&text, &NullProvider);
    assert!(errs.is_empty(), "{text}\n{errs:#?}");
}
