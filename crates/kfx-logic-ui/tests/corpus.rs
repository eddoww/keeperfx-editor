//! Round trip over every shipped DK script of the test install:
//! import -> export -> compare the command sequences (per condition context).
use kfx_formats::testutil;
use kfx_logic_ui::{compare, compare_per_context, export_bytes, import};
use std::path::{Path, PathBuf};

fn scripts(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let n = p
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase();
            if n.starts_with("map") && n.ends_with(".txt") && n.len() == "map00000.txt".len() {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn corpus_round_trip() {
    let Some(inst) = testutil::install_dir() else {
        println!("no install: skipped");
        return;
    };
    let files = scripts(&inst);
    let (mut total, mut semantic, mut per_ctx, mut bytes, mut editable) = (0, 0, 0, 0, 0);
    let mut failures = vec![];
    let (mut regenerated, mut opaque, mut opaque_files) = (0, 0, vec![]);
    for f in &files {
        let src = kfx_formats::rnc::load_file(f).unwrap();
        total += 1;
        let doc = import(&src);
        let out = export_bytes(&doc);
        let sem = compare(&src, &out);
        if sem.is_ok() {
            semantic += 1;
        }
        if compare_per_context(&src, &out).is_ok() {
            per_ctx += 1;
        }
        if out == src {
            bytes += 1;
        }
        match &doc.read_only {
            None => editable += 1,
            Some(r) => failures.push(format!(
                "{}: view-only: {r}",
                f.strip_prefix(&inst).unwrap().display()
            )),
        }
        if let Err(e) = sem {
            failures.push(format!(
                "{}: NOT semantic: {e}",
                f.strip_prefix(&inst).unwrap().display()
            ));
        }
        // regenerated from the model alone (no source text) it must still mean the same
        let mut regen = doc.clone();
        regen.forget_source();
        let rout = export_bytes(&regen);
        match compare(&src, &rout) {
            Ok(()) => regenerated += 1,
            Err(e) => failures.push(format!(
                "{}: regenerated NOT semantic: {e}",
                f.strip_prefix(&inst).unwrap().display()
            )),
        }
        opaque += doc.opaque_count();
        if doc.opaque_count() > 0 {
            opaque_files.push(format!(
                "{} ({})",
                f.strip_prefix(&inst).unwrap().display(),
                doc.opaque_count()
            ));
        }
        // second generation is stable
        let doc2 = import(&out);
        assert_eq!(
            export_bytes(&doc2),
            out,
            "{}: export not stable",
            f.display()
        );
    }
    println!(
        "scripts {total}: semantic {semantic}, per-context \
            {per_ctx}, byte-identical {bytes}, editable {editable}, regenerated-semantic \
            {regenerated}"
    );
    println!(
        "opaque (verbatim) lines: {opaque} in {} scripts: {}",
        opaque_files.len(),
        opaque_files.join(", ")
    );
    for l in &failures {
        println!("  {l}");
    }
    assert_eq!(semantic, total, "every script must round-trip semantically");
    assert_eq!(
        regenerated, editable,
        "every editable script must regenerate semantically"
    );
}

#[test]
#[ignore]
fn list_opaque() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    for f in scripts(&inst) {
        let src = kfx_formats::rnc::load_file(&f).unwrap();
        let doc = import(&src);
        fn walk(nodes: &[kfx_logic_ui::Node]) {
            for n in nodes {
                match n {
                    kfx_logic_ui::Node::Action(a) if a.opaque => {
                        println!("  line {}: {:?}", a.line, a.raw)
                    }
                    kfx_logic_ui::Node::Event(e) => walk(&e.body),
                    _ => {}
                }
            }
        }
        if doc.opaque_count() > 0 {
            println!("{}", f.display());
            walk(&doc.nodes);
        }
    }
}

#[test]
fn corpus_lua_export() {
    use kfx_logic_ui::lua::{export_lua, LuaFlagKind};
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let idx = kfx_script::lua::ApiIndex::stock();
    let prov = kfx_script::InstallProvider::new(&inst);
    let (mut files, mut parse_ok, mut noeq, mut approx, mut events) = (0, 0, 0, 0, 0);
    let mut reasons: std::collections::BTreeMap<String, usize> = Default::default();
    for f in scripts(&inst) {
        let src = kfx_formats::rnc::load_file(&f).unwrap();
        let doc = import(&src);
        files += 1;
        events += doc.events().len();
        let x = export_lua(&doc, &idx, &prov);
        let d = kfx_script::lua::LuaDoc::parse(&x.lua);
        if d.syntax_errors.is_empty() && d.lex_errors.is_empty() {
            parse_ok += 1;
        } else {
            println!(
                "{}: Lua does not parse: {:?}",
                f.display(),
                d.syntax_errors.first()
            );
        }
        for fl in &x.flags {
            match fl.kind {
                LuaFlagKind::NoEquivalent => {
                    noeq += 1;
                    let key = format!(
                        "{}: {}",
                        fl.what,
                        fl.reason.split(['\'', '(']).next().unwrap_or("")
                    );
                    *reasons.entry(key).or_default() += 1;
                }
                LuaFlagKind::Approximate => {
                    approx += 1;
                    let key = if fl.reason.starts_with("through RunDKScriptCommand") {
                        "(approx) RunDKScriptCommand fallback".to_string()
                    } else {
                        format!("(approx) {}: {}", fl.what, fl.reason)
                    };
                    *reasons.entry(key).or_default() += 1;
                }
            }
        }
    }
    println!(
        "Lua export: {files} scripts, {parse_ok} parse, {events} \
            events, {noeq} no-equivalent flags, {approx} approximate flags"
    );
    let mut r: Vec<_> = reasons.into_iter().collect();
    r.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, n) in r.iter().take(20) {
        println!("  {n:5}  {k}");
    }
    assert_eq!(parse_ok, files);
}
