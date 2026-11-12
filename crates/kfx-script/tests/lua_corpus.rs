//! Lexes, parses and lints every shipped Lua file: the install named by `KFX_INSTALL`
//! (`fxdata/lua`, campaign `lua/` dirs, `map*.lua` in campgns/levels/multiplayer) and the `config/`
//! tree of the KeeperFX source checkout named by `KFX_SOURCE`. Each part is skipped when its
//! variable is unset.
use kfx_formats::testutil;
use kfx_script::lua::diagnostics::diagnostics;
use kfx_script::lua::doc::LuaDoc;
use kfx_script::lua::index::ApiIndex;
use kfx_script::lua::lexer::{lex, reassemble};
use kfx_script::{NullProvider, Severity};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The KeeperFX source checkout (`KFX_SOURCE`), `None` when unset.
fn source_dir() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("KFX_SOURCE")?);
    if p.is_dir() {
        Some(p)
    } else {
        None
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, pred: &dyn Fn(&Path) -> bool) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out, pred);
        } else if pred(&p) {
            out.push(p);
        }
    }
}

fn is_lua(p: &Path) -> bool {
    p.extension()
        .map_or(false, |x| x.eq_ignore_ascii_case("lua"))
}

/// Every Lua file of the install (fxdata/lua, campaign lua dirs, level scripts) and of the
/// source checkout's `config/` tree, whichever of the two is available.
fn corpus() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = testutil::install_dir() {
        walk(&d, &mut v, &is_lua);
    } else {
        eprintln!("KFX_INSTALL is not set: install Lua files skipped");
    }
    if let Some(src) = source_dir() {
        walk(&src.join("config"), &mut v, &is_lua);
    } else {
        eprintln!("KFX_SOURCE is not set: source tree Lua files skipped");
    }
    v.sort();
    v
}

#[test]
fn lua_corpus_lex_parse_lint() {
    let files = corpus();
    if files.is_empty() {
        eprintln!("skip: no Lua files found");
        return;
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(d) = testutil::install_dir() {
        let r = d.join("fxdata/lua");
        if r.is_dir() {
            roots.push(r);
        }
    }
    let rr: Vec<&Path> = roots.iter().map(|p| p.as_path()).collect();
    let ix = if rr.is_empty() {
        ApiIndex::stock()
    } else {
        ApiIndex::build(&rr)
    };

    let mut by_code: BTreeMap<String, usize> = BTreeMap::new();
    let mut sev: BTreeMap<String, usize> = BTreeMap::new();
    let (mut bytes, mut non_utf8, mut errors_total) = (0usize, 0usize, 0usize);
    let mut lines_out: Vec<String> = Vec::new();
    let mut largest = (0usize, PathBuf::new());
    for f in &files {
        let raw = std::fs::read(f).unwrap();
        bytes += raw.len();
        if std::str::from_utf8(&raw).is_err() {
            non_utf8 += 1;
        }
        let src = String::from_utf8_lossy(&raw).into_owned();
        let lexed = lex(&src);
        assert_eq!(
            reassemble(&src, &lexed.tokens),
            src,
            "lossless lexing of {}",
            f.display()
        );
        assert!(
            lexed.errors.is_empty(),
            "lex errors in {}: {:?}",
            f.display(),
            lexed.errors
        );
        let doc = LuaDoc::parse_named(&f.to_string_lossy(), &src);
        assert!(
            doc.syntax_errors.is_empty(),
            "syntax errors in {}: {:?}",
            f.display(),
            doc.syntax_errors
        );
        let diags = diagnostics(&doc, &ix, &NullProvider);
        for d in &diags {
            *by_code.entry(d.code.to_string()).or_default() += 1;
            *sev.entry(format!("{:?}", d.severity)).or_default() += 1;
            if d.severity == Severity::Error {
                errors_total += 1;
            }
            lines_out.push(format!(
                "{}:{}: [{:?} {}] {}",
                f.display(),
                d.line,
                d.severity,
                d.code,
                d.message
            ));
        }
        if raw.len() > largest.0 {
            largest = (raw.len(), f.clone());
        }
    }
    println!(
        "files: {}  bytes: {}  non-utf8: {}",
        files.len(),
        bytes,
        non_utf8
    );
    println!("diagnostics by severity: {sev:?}");
    println!("diagnostics by code: {by_code:?}");
    for l in &lines_out {
        println!("{l}");
    }
    assert_eq!(
        errors_total, 0,
        "Error-severity diagnostics on shipped code"
    );
}

#[test]
fn lua_timing_largest_file() {
    let files = corpus();
    let Some(big) = files
        .iter()
        .max_by_key(|f| std::fs::metadata(f).map(|m| m.len()).unwrap_or(0))
    else {
        eprintln!("skip: no Lua files found");
        return;
    };
    let src = String::from_utf8_lossy(&std::fs::read(big).unwrap()).into_owned();
    let ix = ApiIndex::stock();
    let t = Instant::now();
    let lexed = lex(&src);
    let t_lex = t.elapsed();
    let doc = LuaDoc::parse_named("big.lua", &src);
    let t_parse = t.elapsed();
    let d = diagnostics(&doc, &ix, &NullProvider);
    let t_all = t.elapsed();
    println!(
        "largest: {} ({} bytes, {} lines): lex {:?}, +parse/analyse {:?}, +diagnostics {:?}, \
        {} tokens, {} diagnostics",
        big.display(),
        src.len(),
        src.lines().count(),
        t_lex,
        t_parse,
        t_all,
        lexed.tokens.len(),
        d.len()
    );
    let t = Instant::now();
    let rr: Vec<PathBuf> = testutil::install_dir()
        .map(|d| d.join("fxdata/lua"))
        .into_iter()
        .filter(|p| p.is_dir())
        .collect();
    let rr2: Vec<&Path> = rr.iter().map(|p| p.as_path()).collect();
    let _ = ApiIndex::build(&rr2);
    println!("stock index build: {:?}", t.elapsed());
}

/// Extra real-world Lua (not KeeperFX): lexing must be lossless and nothing may panic; the
/// share of files with syntax errors is printed (some use Lua 5.3/5.4 syntax that LuaJIT rejects).
/// Directory: env `KFX_LUA_EXTRA_DIR` (for example the `meta` stubs of lua-language-server);
/// skipped when unset.
#[test]
fn lua_extra_corpus_robustness() {
    let Ok(dir) = std::env::var("KFX_LUA_EXTRA_DIR") else {
        eprintln!("skip: KFX_LUA_EXTRA_DIR is not set");
        return;
    };
    let mut files = Vec::new();
    walk(Path::new(&dir), &mut files, &is_lua);
    if files.is_empty() {
        eprintln!("skip: no extra Lua corpus at {dir}");
        return;
    }
    files.sort();
    let ix = ApiIndex::stock();
    let (mut with_err, mut bytes, mut worst_ms) = (0usize, 0usize, 0u128);
    let mut samples: Vec<String> = Vec::new();
    for f in &files {
        let src = String::from_utf8_lossy(&std::fs::read(f).unwrap()).into_owned();
        bytes += src.len();
        let lexed = lex(&src);
        assert_eq!(reassemble(&src, &lexed.tokens), src);
        let t = Instant::now();
        let doc = LuaDoc::parse_named(&f.to_string_lossy(), &src);
        let _ = diagnostics(&doc, &ix, &NullProvider);
        worst_ms = worst_ms.max(t.elapsed().as_millis());
        if !doc.syntax_errors.is_empty() || !doc.lex_errors.is_empty() {
            with_err += 1;
            if samples.len() < 8 {
                let e = doc
                    .syntax_errors
                    .first()
                    .map(|e| (e.span.start, e.message.clone()))
                    .or_else(|| {
                        doc.lex_errors
                            .first()
                            .map(|e| (e.span.start, e.message.clone()))
                    })
                    .unwrap();
                samples.push(format!(
                    "{}:{}: {}",
                    f.strip_prefix(&dir).unwrap_or(f).display(),
                    doc.line_of(e.0),
                    e.1
                ));
            }
        }
    }
    println!(
        "extra corpus {dir}: {} files, {bytes} bytes, {with_err} with syntax errors, worst \
        parse+lint {worst_ms} ms",
        files.len()
    );
    for s in samples {
        println!("  {s}");
    }
}
